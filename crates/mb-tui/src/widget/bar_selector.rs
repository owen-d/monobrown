use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::{Constraints, LayoutRenderable, Size};
use crate::render::{OverflowBehavior, display_width, ellipsize_text, summarize_text};
use crate::theme;

const DECAY_TIME_CONSTANTS: f64 = 5.0; // 5t reaches ~99.3% of target
const DETAILED_HEIGHT: u16 = 4;
const MIN_SLOT_WIDTH_FOR_BARS: u16 = 4;
const SNAP_EPSILON: f64 = 0.001;

/// A slot in the bar selector.
#[derive(Clone)]
struct Slot {
    label: &'static str,
    value: f64,  // current bar height 0.0-1.0
    target: f64, // where value is heading
}

/// Animated tri-slot selector with vertical bars.
///
/// Each slot has a vertical bar above it. The selected slot's bar is at 100%,
/// others at 0%. Transitions animate via exponential decay -- stateless,
/// handles mid-transition interrupts gracefully.
#[derive(Clone)]
pub struct BarSelector {
    slots: Vec<Slot>,
    selected: usize,
    transition_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorLayout {
    Detailed { slot_width: u16 },
    Summary,
}

impl BarSelector {
    pub fn new(labels: &[&'static str]) -> Self {
        assert!(!labels.is_empty());
        let mut slots: Vec<Slot> = labels
            .iter()
            .map(|&label| Slot {
                label,
                value: 0.0,
                target: 0.0,
            })
            .collect();
        slots[0].value = 1.0;
        slots[0].target = 1.0;
        Self {
            slots,
            selected: 0,
            transition_ms: 300.0,
        }
    }

    pub fn select(&mut self, index: usize) {
        if index >= self.slots.len() {
            return;
        }
        self.selected = index;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            slot.target = if i == index { 1.0 } else { 0.0 };
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select_next(&mut self) {
        let next = (self.selected + 1) % self.slots.len();
        self.select(next);
    }

    pub fn select_prev(&mut self) {
        let prev = (self.selected + self.slots.len() - 1) % self.slots.len();
        self.select(prev);
    }

    pub fn tick(&mut self, dt: Duration) {
        let dt_secs = dt.as_secs_f64();
        let k = DECAY_TIME_CONSTANTS / (self.transition_ms / 1000.0);
        let factor = 1.0 - (-k * dt_secs).exp();

        for slot in &mut self.slots {
            slot.value += (slot.target - slot.value) * factor;
            // Snap to target when close enough.
            if (slot.value - slot.target).abs() < SNAP_EPSILON {
                slot.value = slot.target;
            }
        }
    }

    /// Get the current value for a slot (for testing).
    pub fn value(&self, index: usize) -> f64 {
        self.slots[index].value
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether there are no slots. (Required by clippy alongside `len`.)
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn layout(&self, width: u16) -> SelectorLayout {
        let slot_count = self.slots.len() as u16;
        if slot_count == 0 {
            return SelectorLayout::Summary;
        }

        let slot_width = width / slot_count;
        if slot_width >= MIN_SLOT_WIDTH_FOR_BARS {
            SelectorLayout::Detailed { slot_width }
        } else {
            SelectorLayout::Summary
        }
    }
}

impl LayoutRenderable for BarSelector {
    fn measure(&self, constraints: Constraints) -> Size {
        if constraints.max_height == Some(0) || self.slots.is_empty() {
            return Size::ZERO;
        }

        let height_limited = constraints.max_height.is_some_and(|height| height < 2);
        let preferred_width = if height_limited {
            summary_width(self)
        } else {
            detailed_width(self)
        };
        let width = constraints.constrain(Size::new(preferred_width, 0)).width;
        if width == 0 {
            return Size::ZERO;
        }

        let desired_height = match selector_overflow(self, width, constraints.max_height) {
            OverflowBehavior::Summary => 1,
            OverflowBehavior::Clip | OverflowBehavior::Ellipsis => DETAILED_HEIGHT,
        };

        constraints.constrain(Size::new(preferred_width, desired_height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.slots.is_empty() {
            return;
        }

        match (
            selector_overflow(self, area.width, Some(area.height)),
            self.layout(area.width),
        ) {
            (OverflowBehavior::Summary, _) => render_summary(self, area, buf),
            (_, SelectorLayout::Detailed { slot_width }) if area.height >= 2 => {
                render_detailed(self, area, buf, slot_width);
            }
            _ => render_summary(self, area, buf),
        }
    }
}

fn selector_overflow(state: &BarSelector, width: u16, max_height: Option<u16>) -> OverflowBehavior {
    if max_height.is_some_and(|height| height < 2)
        || matches!(state.layout(width), SelectorLayout::Summary)
    {
        OverflowBehavior::Summary
    } else {
        OverflowBehavior::Ellipsis
    }
}

fn render_detailed(state: &BarSelector, area: Rect, buf: &mut Buffer, slot_width: u16) {
    let slot_count = state.slots.len() as u16;
    let bar_height = area.height - 1; // reserve bottom row for labels
    let label_y = area.y + area.height - 1;

    for (i, slot) in state.slots.iter().enumerate() {
        let slot_x = area.x + (i as u16) * slot_width;
        let w = if i as u16 == slot_count - 1 {
            area.width - (i as u16) * slot_width // last slot gets remaining width
        } else {
            slot_width
        };

        // Draw bar.
        let filled_rows = (slot.value * bar_height as f64).round() as u16;
        let bar_start_y = area.y + bar_height - filled_rows;

        let bar_style = if slot.value > 0.5 {
            Style::default().fg(theme::focus())
        } else if slot.value > 0.01 {
            Style::default().fg(theme::dim())
        } else {
            Style::default()
        };

        // Bar occupies the center portion of the slot width, leaving 1-cell padding.
        let bar_inner_x = slot_x + 1;
        let bar_inner_w = w.saturating_sub(2).max(1);

        for y in bar_start_y..area.y + bar_height {
            for x in bar_inner_x..bar_inner_x + bar_inner_w {
                if x < area.x + area.width {
                    buf[(x, y)].set_symbol("\u{2588}").set_style(bar_style);
                }
            }
        }

        // Draw a slot-local label that truncates before it spills into neighbors.
        let label = ellipsize_text(slot.label, w as usize);
        let label_width = display_width(&label) as u16;
        let label_x = slot_x + (w.saturating_sub(label_width)) / 2;
        let label_style = if i == state.selected {
            Style::default().fg(theme::focus())
        } else {
            Style::default().fg(theme::dim())
        };
        buf.set_stringn(label_x, label_y, &label, w as usize, label_style);
    }
}

fn render_summary(state: &BarSelector, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    if area.width < state.slots.len() as u16 * 4 {
        render_selected_summary(state, area, buf);
        return;
    }

    let mut x = area.x;
    let right = area.x + area.width;
    for (i, slot) in state.slots.iter().enumerate() {
        if x >= right {
            break;
        }

        if i > 0 {
            if x >= right {
                break;
            }
            buf[(x, area.y)].set_symbol(" ");
            x += 1;
        }

        let available = right.saturating_sub(x) as usize;
        if available == 0 {
            break;
        }

        let token = if i == state.selected {
            let inner_width = available.saturating_sub(2).max(1);
            format!("[{}]", ellipsize_text(slot.label, inner_width))
        } else {
            ellipsize_text(slot.label, available)
        };
        let token_width = display_width(&token).min(available);
        let style = if i == state.selected {
            Style::default().fg(theme::focus())
        } else {
            Style::default().fg(theme::dim())
        };
        buf.set_stringn(x, area.y, &token, available, style);
        x += token_width as u16;
    }
}

fn render_selected_summary(state: &BarSelector, area: Rect, buf: &mut Buffer) {
    let prefix = format!("{}/{} ", state.selected + 1, state.slots.len());
    let summary = summarize_text(
        &prefix,
        state.slots[state.selected].label,
        "",
        area.width as usize,
    );
    let prefix_width = display_width(&prefix).min(area.width as usize) as u16;

    buf.set_stringn(
        area.x,
        area.y,
        &summary,
        area.width as usize,
        Style::default().fg(theme::focus()),
    );
    if prefix_width > 0 {
        buf.set_stringn(
            area.x,
            area.y,
            &prefix,
            prefix_width as usize,
            Style::default().fg(theme::dim()),
        );
    }
}

fn detailed_width(state: &BarSelector) -> u16 {
    let slot_width = state
        .slots
        .iter()
        .map(|slot| display_width(slot.label))
        .max()
        .unwrap_or(0)
        .max(MIN_SLOT_WIDTH_FOR_BARS as usize);
    saturating_width(slot_width.saturating_mul(state.slots.len()))
}

fn summary_width(state: &BarSelector) -> u16 {
    let token_row_width = state
        .slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            if i == state.selected {
                display_width(slot.label).saturating_add(2)
            } else {
                display_width(slot.label)
            }
        })
        .sum::<usize>()
        .saturating_add(state.slots.len().saturating_sub(1));
    let selected_width = display_width(&format!(
        "{}/{} {}",
        state.selected + 1,
        state.slots.len(),
        state.slots[state.selected].label
    ));
    saturating_width(token_row_width.max(selected_width))
}

fn saturating_width(width: usize) -> u16 {
    width.min(u16::MAX as usize) as u16
}

/// Render function matching the `fn(&S, Rect, &mut Buffer)` signature.
pub fn render_bar_selector(state: &BarSelector, area: Rect, buf: &mut Buffer) {
    state.render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect};

    #[test]
    fn new_starts_at_slot_zero() {
        let sel = BarSelector::new(&["A", "B", "C"]);
        assert_eq!(sel.selected(), 0);
        assert!((sel.value(0) - 1.0).abs() < SNAP_EPSILON);
        assert!(sel.value(1).abs() < SNAP_EPSILON);
        assert!(sel.value(2).abs() < SNAP_EPSILON);
    }

    #[test]
    fn select_sets_targets() {
        let mut sel = BarSelector::new(&["A", "B", "C"]);
        sel.select(2);
        assert_eq!(sel.selected(), 2);
        // Values haven't changed yet (no tick).
        assert!((sel.value(0) - 1.0).abs() < SNAP_EPSILON);
    }

    #[test]
    fn tick_converges_to_target() {
        let mut sel = BarSelector::new(&["A", "B", "C"]);
        sel.select(1);
        // Tick for 512ms in 16ms steps (well past transition_ms of 300ms).
        for _ in 0..32 {
            sel.tick(Duration::from_millis(16));
        }
        assert!((sel.value(0)).abs() < SNAP_EPSILON);
        assert!((sel.value(1) - 1.0).abs() < SNAP_EPSILON);
        assert!((sel.value(2)).abs() < SNAP_EPSILON);
    }

    #[test]
    fn tick_is_deterministic() {
        let run = || {
            let mut sel = BarSelector::new(&["A", "B", "C"]);
            sel.select(1);
            for _ in 0..10 {
                sel.tick(Duration::from_millis(16));
            }
            (sel.value(0), sel.value(1), sel.value(2))
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn mid_transition_interrupt() {
        let mut sel = BarSelector::new(&["A", "B", "C"]);
        sel.select(1);
        // Tick partway through transition.
        for _ in 0..6 {
            // ~96ms
            sel.tick(Duration::from_millis(16));
        }
        // Interrupt: change to slot 2.
        sel.select(2);
        // Tick to completion.
        for _ in 0..32 {
            sel.tick(Duration::from_millis(16));
        }
        assert!((sel.value(0)).abs() < SNAP_EPSILON);
        assert!((sel.value(1)).abs() < SNAP_EPSILON);
        assert!((sel.value(2) - 1.0).abs() < SNAP_EPSILON);
    }

    #[test]
    fn select_out_of_bounds_is_noop() {
        let mut sel = BarSelector::new(&["A", "B", "C"]);
        sel.select(99);
        assert_eq!(sel.selected(), 0);
    }

    #[test]
    fn select_next_wraps() {
        let mut sel = BarSelector::new(&["A", "B", "C"]);
        sel.select_next(); // 0 -> 1
        assert_eq!(sel.selected(), 1);
        sel.select_next(); // 1 -> 2
        assert_eq!(sel.selected(), 2);
        sel.select_next(); // 2 -> 0
        assert_eq!(sel.selected(), 0);
    }

    #[test]
    fn desired_height_switches_to_summary_for_narrow_widths() {
        let sel = BarSelector::new(&["Alpha", "Beta", "Gamma"]);
        assert_eq!(
            sel.measure(Constraints::tight_width(30)).height,
            DETAILED_HEIGHT
        );
        assert_eq!(sel.measure(Constraints::tight_width(9)).height, 1);
    }

    #[test]
    fn measure_prefers_label_driven_width_when_constraints_are_loose() {
        let sel = BarSelector::new(&["One", "Two", "Three"]);
        assert_eq!(sel.measure(Constraints::loose(40, 6)), Size::new(15, 4));
    }

    #[test]
    fn measure_switches_to_summary_for_short_heights() {
        let sel = BarSelector::new(&["Alpha", "Beta", "Gamma"]);
        assert_eq!(sel.measure(Constraints::loose(30, 1)), Size::new(18, 1));
        assert_eq!(
            sel.measure(Constraints::loose(30, DETAILED_HEIGHT)),
            Size::new(15, DETAILED_HEIGHT)
        );
    }

    #[test]
    fn summary_render_reports_selected_slot_when_width_is_tight() {
        let mut sel = BarSelector::new(&["Alpha", "Beta", "Gamma"]);
        sel.select(1);

        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        sel.render(area, &mut buf);

        let text = (0..area.width)
            .map(|x| buf[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string();
        assert!(text.contains("2/3"), "expected index summary, got {text:?}");
        assert!(
            text.contains("Beta"),
            "expected selected label, got {text:?}"
        );
    }
}

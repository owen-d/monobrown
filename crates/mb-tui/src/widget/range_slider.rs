use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::display_width;
use crate::theme;

/// Filled track character (left of thumb): ━ (U+2501).
const FILLED_CHAR: &str = "\u{2501}";
/// Thumb character: ◆ (U+25C6).
const THUMB_CHAR: &str = "\u{25C6}";
/// Empty track character (right of thumb): ─ (U+2500).
const EMPTY_CHAR: &str = "\u{2500}";

/// Width reserved for the value display: space + up to 3 digits + '%' = 5 chars.
const VALUE_WIDTH: u16 = 5;

/// Default step size for continuous increment/decrement.
const CONTINUOUS_STEP: f64 = 0.1;

/// A horizontal slider with a thumb on a track.
///
/// Supports both continuous values and discrete steps.
///
/// ```text
/// [label ] [━━━━━◆─────────] [50%]
/// ```
#[derive(Clone)]
pub struct RangeSlider {
    value: f64,
    steps: Option<u16>,
    label: Option<String>,
    show_value: bool,
}

impl RangeSlider {
    pub fn new(value: f64) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            steps: None,
            label: None,
            show_value: true,
        }
    }

    pub fn steps(mut self, steps: u16) -> Self {
        self.steps = Some(steps);
        self
    }

    pub fn label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    pub fn show_value(mut self, show: bool) -> Self {
        self.show_value = show;
        self
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = value.clamp(0.0, 1.0);
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn increment(&mut self) {
        let step = match self.steps {
            Some(n) if n > 0 => 1.0 / n as f64,
            _ => CONTINUOUS_STEP,
        };
        self.value = (self.value + step).clamp(0.0, 1.0);
    }

    pub fn decrement(&mut self) {
        let step = match self.steps {
            Some(n) if n > 0 => 1.0 / n as f64,
            _ => CONTINUOUS_STEP,
        };
        self.value = (self.value - step).clamp(0.0, 1.0);
    }

    /// Return the effective value, snapped to the nearest step if discrete.
    fn snapped_value(&self) -> f64 {
        match self.steps {
            Some(n) if n > 0 => (self.value * n as f64).round() / n as f64,
            _ => self.value,
        }
    }
}

impl LayoutRenderable for RangeSlider {
    fn measure(&self, constraints: Constraints) -> Size {
        Size::new(constraints.fill_width(), 1)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut x = area.x;
        let right = area.x + area.width;

        // 1. Render label at left if set.
        if let Some(ref label_text) = self.label {
            let label_style = Style::default().fg(theme::dim());
            let label_w = display_width(label_text) as u16;
            let available = right.saturating_sub(x);
            if available == 0 {
                return;
            }
            buf.set_stringn(x, area.y, label_text, available as usize, label_style);
            // Advance past label + trailing space.
            x += label_w.min(available);
            if x < right {
                x += 1;
            }
        }

        // 2. Reserve space for value display at right if enabled.
        let val_width = if self.show_value { VALUE_WIDTH } else { 0 };
        let track_width = right.saturating_sub(x).saturating_sub(val_width);

        if track_width == 0 {
            return;
        }

        // 3. Compute thumb position.
        let snapped = self.snapped_value();
        let thumb_pos = (snapped * (track_width as f64 - 1.0)).round() as u16;
        let thumb_pos = thumb_pos.min(track_width.saturating_sub(1));

        // 4. Render track and thumb.
        let focus_style = Style::default().fg(theme::focus());
        let thumb_style = Style::default()
            .fg(theme::focus())
            .add_modifier(Modifier::BOLD);
        let dim_style = Style::default().fg(theme::dim());

        for i in 0..track_width {
            if i < thumb_pos {
                buf.set_stringn(x, area.y, FILLED_CHAR, 1, focus_style);
            } else if i == thumb_pos {
                buf.set_stringn(x, area.y, THUMB_CHAR, 1, thumb_style);
            } else {
                buf.set_stringn(x, area.y, EMPTY_CHAR, 1, dim_style);
            }
            x += 1;
        }

        // 5. Render percentage at right if enabled.
        if self.show_value {
            let pct = (snapped * 100.0).round() as u32;
            let pct_text = format!("{pct:>3}%");
            let pct_style = Style::default().fg(theme::dim());
            // Leading space before percentage.
            x += 1;
            let remaining = right.saturating_sub(x) as usize;
            buf.set_stringn(x, area.y, &pct_text, remaining, pct_style);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    /// Extract the symbol string for a single row from the buffer.
    fn buf_symbols(buf: &Buffer, area: Rect) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf[(x, area.y)].symbol().to_string())
            .collect()
    }

    /// Count occurrences of a character in the rendered row.
    fn count_char(buf: &Buffer, area: Rect, ch: &str) -> usize {
        (area.x..area.x + area.width)
            .filter(|&x| buf[(x, area.y)].symbol() == ch)
            .count()
    }

    #[test]
    fn height_always_one() {
        let slider = RangeSlider::new(0.5);
        assert_eq!(slider.measure(Constraints::loose(80, 10)).height, 1);
        assert_eq!(slider.measure(Constraints::tight(40, 5)).height, 1);
        assert_eq!(slider.measure(Constraints::unbounded()).height, 1);
    }

    #[test]
    fn thumb_at_zero() {
        let slider = RangeSlider::new(0.0).show_value(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        // Thumb should be at position 0.
        assert_eq!(buf[(0, 0)].symbol(), THUMB_CHAR);
        // No filled chars before thumb.
        assert_eq!(count_char(&buf, area, FILLED_CHAR), 0);
    }

    #[test]
    fn thumb_at_one() {
        let slider = RangeSlider::new(1.0).show_value(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        // Thumb should be at the last position.
        assert_eq!(buf[(19, 0)].symbol(), THUMB_CHAR);
        // No empty chars after thumb.
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 0);
    }

    #[test]
    fn thumb_at_half() {
        let slider = RangeSlider::new(0.5).show_value(false);
        let area = Rect::new(0, 0, 21, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        // track_width=21, thumb_pos = (0.5 * 20).round() = 10
        assert_eq!(buf[(10, 0)].symbol(), THUMB_CHAR);
        assert_eq!(count_char(&buf, area, FILLED_CHAR), 10);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 10);
    }

    #[test]
    fn discrete_snapping() {
        // steps=4, value=0.3 -> snapped = round(0.3*4)/4 = round(1.2)/4 = 1/4 = 0.25
        let slider = RangeSlider::new(0.3).steps(4).show_value(false);
        let area = Rect::new(0, 0, 21, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        // snapped=0.25, thumb_pos = (0.25 * 20).round() = 5
        assert_eq!(buf[(5, 0)].symbol(), THUMB_CHAR);
        assert_eq!(count_char(&buf, area, FILLED_CHAR), 5);
    }

    #[test]
    fn value_shown() {
        let slider = RangeSlider::new(0.5);
        // 25 wide: 20 track + 5 value display.
        let area = Rect::new(0, 0, 25, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        let text = buf_symbols(&buf, area);
        assert!(
            text.ends_with(" 50%"),
            "expected ' 50%' suffix, got: {text:?}"
        );
    }

    #[test]
    fn label_rendered() {
        let slider = RangeSlider::new(0.5)
            .label("Vol".to_string())
            .show_value(false);
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        let text = buf_symbols(&buf, area);
        assert!(
            text.starts_with("Vol"),
            "expected label at left, got: {text:?}"
        );

        // Label should use dim color.
        let dim_color = crate::theme::dim();
        assert_eq!(buf[(0, 0)].fg, dim_color, "label should use dim color");
    }

    #[test]
    fn increment_continuous() {
        let mut slider = RangeSlider::new(0.5);
        slider.increment();
        let expected = 0.6;
        assert!(
            (slider.value() - expected).abs() < 1e-9,
            "expected {expected}, got {}",
            slider.value()
        );

        // Clamped at 1.0.
        slider.set_value(0.95);
        slider.increment();
        assert!(
            (slider.value() - 1.0).abs() < 1e-9,
            "expected 1.0, got {}",
            slider.value()
        );
    }

    #[test]
    fn decrement_continuous() {
        let mut slider = RangeSlider::new(0.5);
        slider.decrement();
        let expected = 0.4;
        assert!(
            (slider.value() - expected).abs() < 1e-9,
            "expected {expected}, got {}",
            slider.value()
        );

        // Clamped at 0.0.
        slider.set_value(0.05);
        slider.decrement();
        assert!(
            slider.value().abs() < 1e-9,
            "expected 0.0, got {}",
            slider.value()
        );
    }

    #[test]
    fn increment_discrete() {
        let mut slider = RangeSlider::new(0.0).steps(4);
        slider.increment();
        assert!(
            (slider.value() - 0.25).abs() < 1e-9,
            "expected 0.25, got {}",
            slider.value()
        );
        slider.increment();
        assert!(
            (slider.value() - 0.5).abs() < 1e-9,
            "expected 0.5, got {}",
            slider.value()
        );
    }

    #[test]
    fn value_clamped() {
        let slider = RangeSlider::new(1.5);
        assert!(
            (slider.value() - 1.0).abs() < 1e-9,
            "value > 1.0 should clamp to 1.0"
        );

        let slider = RangeSlider::new(-0.5);
        assert!(
            slider.value().abs() < 1e-9,
            "value < 0.0 should clamp to 0.0"
        );

        let mut slider = RangeSlider::new(0.5);
        slider.set_value(2.0);
        assert!(
            (slider.value() - 1.0).abs() < 1e-9,
            "set_value > 1.0 should clamp to 1.0"
        );
        slider.set_value(-1.0);
        assert!(
            slider.value().abs() < 1e-9,
            "set_value < 0.0 should clamp to 0.0"
        );
    }

    #[test]
    fn track_chars() {
        let slider = RangeSlider::new(0.5).show_value(false);
        let area = Rect::new(0, 0, 21, 1);
        let mut buf = Buffer::empty(area);
        slider.render(area, &mut buf);

        let text = buf_symbols(&buf, area);
        assert!(
            text.contains(FILLED_CHAR),
            "should contain filled track char"
        );
        assert!(text.contains(THUMB_CHAR), "should contain thumb char");
        assert!(text.contains(EMPTY_CHAR), "should contain empty track char");
    }
}

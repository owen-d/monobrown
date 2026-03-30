use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::{display_width, ellipsize_text};
use crate::theme;

/// Left arrow glyph (U+25C0).
const LEFT_ARROW: &str = "\u{25C0}";
/// Right arrow glyph (U+25B6).
const RIGHT_ARROW: &str = "\u{25B6}";

/// Width consumed by each arrow region: the arrow character + one space.
const ARROW_REGION_WIDTH: u16 = 2;

/// Discrete value selector rendered as a single row: `◀ value ▶`.
#[derive(Clone)]
pub struct Stepper {
    options: Vec<String>,
    selected: usize,
    wrap: bool,
}

impl Stepper {
    pub fn new(options: Vec<String>) -> Self {
        Self {
            options,
            selected: 0,
            wrap: true,
        }
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_label(&self) -> &str {
        &self.options[self.selected]
    }

    pub fn select(&mut self, index: usize) {
        if index < self.options.len() {
            self.selected = index;
        }
    }

    pub fn next(&mut self) {
        if self.options.is_empty() {
            return;
        }
        if self.selected + 1 < self.options.len() {
            self.selected += 1;
        } else if self.wrap {
            self.selected = 0;
        }
    }

    pub fn prev(&mut self) {
        if self.options.is_empty() {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        } else if self.wrap {
            self.selected = self.options.len() - 1;
        }
    }

    pub fn len(&self) -> usize {
        self.options.len()
    }

    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }

    fn show_left_arrow(&self) -> bool {
        self.wrap || self.selected > 0
    }

    fn show_right_arrow(&self) -> bool {
        self.wrap || self.selected + 1 < self.options.len()
    }

    fn max_label_width(&self) -> usize {
        self.options
            .iter()
            .map(|o| display_width(o))
            .max()
            .unwrap_or(0)
    }
}

impl LayoutRenderable for Stepper {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = if constraints.max_width.is_some() {
            constraints.fill_width()
        } else {
            let preferred =
                ARROW_REGION_WIDTH as usize + self.max_label_width() + ARROW_REGION_WIDTH as usize;
            constraints.constrain(Size::new(preferred as u16, 1)).width
        };
        Size::new(width, 1)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.options.is_empty() {
            return;
        }

        let arrow_style = Style::default().fg(theme::focus());
        let label_style = Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD);

        let width = area.width;

        // Render left arrow region (2 chars: arrow + space).
        if width >= 1 && self.show_left_arrow() {
            buf.set_stringn(area.x, area.y, LEFT_ARROW, 1, arrow_style);
        }

        // Render right arrow region (2 chars: space + arrow).
        if width >= 2 {
            let right_arrow_x = area.x + width - 1;
            if self.show_right_arrow() {
                buf.set_stringn(right_arrow_x, area.y, RIGHT_ARROW, 1, arrow_style);
            }
        }

        // Render the label in the interior space.
        let interior_start = area.x + ARROW_REGION_WIDTH.min(width);
        let interior_end = if width > ARROW_REGION_WIDTH {
            area.x + width - ARROW_REGION_WIDTH
        } else {
            interior_start
        };

        if interior_end > interior_start {
            let interior_width = (interior_end - interior_start) as usize;
            let label = ellipsize_text(self.selected_label(), interior_width);
            let label_display = display_width(&label);
            // Center the label in the interior.
            let pad_left = (interior_width.saturating_sub(label_display)) / 2;
            let label_x = interior_start + pad_left as u16;
            buf.set_stringn(label_x, area.y, &label, interior_width, label_style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_text(buf: &Buffer, area: Rect) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf[(x, area.y)].symbol().to_string())
            .collect::<String>()
    }

    #[test]
    fn height_always_one() {
        let s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]);
        assert_eq!(s.measure(Constraints::loose(80, 10)).height, 1);
        assert_eq!(s.measure(Constraints::tight(40, 5)).height, 1);
        assert_eq!(s.measure(Constraints::unbounded()).height, 1);
    }

    #[test]
    fn renders_arrows_and_label() {
        let s = Stepper::new(vec!["Alpha".into(), "Beta".into(), "Gamma".into()]);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        s.render(area, &mut buf);
        let text = buf_text(&buf, area);
        assert!(text.contains(LEFT_ARROW), "missing left arrow in {text:?}");
        assert!(text.contains("Alpha"), "missing label in {text:?}");
        assert!(
            text.contains(RIGHT_ARROW),
            "missing right arrow in {text:?}"
        );
    }

    #[test]
    fn next_wraps() {
        let mut s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]);
        s.select(2);
        s.next();
        assert_eq!(s.selected(), 0);
    }

    #[test]
    fn next_stops() {
        let mut s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]).wrap(false);
        s.select(2);
        s.next();
        assert_eq!(s.selected(), 2);
    }

    #[test]
    fn prev_wraps() {
        let mut s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]);
        assert_eq!(s.selected(), 0);
        s.prev();
        assert_eq!(s.selected(), 2);
    }

    #[test]
    fn prev_stops() {
        let mut s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]).wrap(false);
        assert_eq!(s.selected(), 0);
        s.prev();
        assert_eq!(s.selected(), 0);
    }

    #[test]
    fn no_wrap_hides_arrow() {
        let s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]).wrap(false);
        // At first option: left arrow should be hidden.
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        s.render(area, &mut buf);
        // The first cell should be a space, not the left arrow.
        let first_symbol = buf[(area.x, area.y)].symbol().to_string();
        assert_eq!(
            first_symbol, " ",
            "left arrow should be hidden at first option with wrap=false"
        );
    }

    #[test]
    fn select_out_of_bounds() {
        let mut s = Stepper::new(vec!["A".into(), "B".into(), "C".into()]);
        s.select(1);
        assert_eq!(s.selected(), 1);
        s.select(99);
        assert_eq!(s.selected(), 1); // unchanged
    }

    #[test]
    fn narrow_width_truncates() {
        let s = Stepper::new(vec!["LongOptionLabel".into()]);
        // Width 7: 2 for left arrow region + 2 for right arrow region = 4, leaving 3 for label.
        // "LongOptionLabel" (14 chars) should be truncated with ellipsis to 3 chars: "Lo…"
        let area = Rect::new(0, 0, 7, 1);
        let mut buf = Buffer::empty(area);
        s.render(area, &mut buf);
        let text = buf_text(&buf, area);
        assert!(
            text.contains('\u{2026}'),
            "expected ellipsis in narrow render, got {text:?}"
        );
    }

    #[test]
    fn selected_label_returns_current() {
        let mut s = Stepper::new(vec!["Alpha".into(), "Beta".into(), "Gamma".into()]);
        assert_eq!(s.selected_label(), "Alpha");
        s.select(1);
        assert_eq!(s.selected_label(), "Beta");
        s.select(2);
        assert_eq!(s.selected_label(), "Gamma");
    }
}

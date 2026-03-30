use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use super::{Constraints, LayoutRenderable, Size};
use crate::theme;

/// Renders a full-width horizontal rule in dim gray.
pub struct SeparatorRenderable;

impl LayoutRenderable for SeparatorRenderable {
    fn measure(&self, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(constraints.fill_width(), 1))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let line = Line::from(Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(theme::dim()),
        ));
        Widget::render(Paragraph::new(line), area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_measures_to_one_row() {
        let size = SeparatorRenderable.measure(Constraints::tight_width(12));
        assert_eq!(size, Size::new(12, 1));
    }
}

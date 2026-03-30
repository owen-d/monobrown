use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use super::{Constraints, LayoutRenderable, Size};
use crate::input::modal::{HotkeyHint, HotkeySection};
use crate::theme;

const HELP_KEY_COLUMN_WIDTH: usize = 14;

/// Format hotkey hints into a space-separated `key:action` string.
pub fn format_hint_string(hints: &[HotkeyHint]) -> String {
    hints
        .iter()
        .map(|hint| format!("{}:{}", hint.key, hint.action))
        .collect::<Vec<_>>()
        .join("  ")
}

/// Renders context-sensitive keyboard shortcut hints with word wrapping.
pub struct HotkeyBarRenderable {
    pub hints: Vec<HotkeyHint>,
}

impl LayoutRenderable for HotkeyBarRenderable {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.fill_width();
        if width == 0 {
            return Size::new(0, 0);
        }
        let paragraph = Paragraph::new(format_hint_string(&self.hints)).wrap(Wrap { trim: false });
        let height = paragraph.line_count(width).max(1) as u16;
        constraints.constrain(Size::new(width, height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let paragraph = Paragraph::new(format_hint_string(&self.hints))
            .style(Style::default().fg(theme::dim()))
            .wrap(Wrap { trim: false });
        Widget::render(paragraph, area, buf);
    }
}

/// A bordered help pane rendering grouped hotkey reference content.
pub struct HelpPaneRenderable<'a> {
    pub title: &'a str,
    pub sections: &'a [HotkeySection],
    pub appendix: &'a [Line<'static>],
}

impl HelpPaneRenderable<'_> {
    fn lines(&self) -> Vec<Line<'static>> {
        let key_style = Style::default().fg(theme::warning());
        let heading_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED);
        let body_style = Style::default().fg(theme::text());

        let mut lines = Vec::new();
        for (index, section) in self.sections.iter().enumerate() {
            if index > 0 {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(section.title, heading_style)));
            for hint in &section.hints {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:>HELP_KEY_COLUMN_WIDTH$}", hint.key), key_style),
                    Span::styled(format!("  {}", hint.description), body_style),
                ]));
            }
        }

        // Append custom content if present.
        if !self.appendix.is_empty() {
            lines.push(Line::default());
            lines.extend(self.appendix.iter().cloned());
        }

        lines
    }
}

impl LayoutRenderable for HelpPaneRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.fill_width();
        if width == 0 {
            return Size::new(0, 0);
        }
        let block = Block::default().title(self.title).borders(Borders::ALL);
        let height = Paragraph::new(self.lines())
            .block(block)
            .wrap(Wrap { trim: false })
            .line_count(width)
            .max(2) as u16;
        constraints.constrain(Size::new(width, height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let block = Block::default()
            .title(self.title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::focus()));

        Widget::render(Clear, area, buf);
        Widget::render(
            Paragraph::new(self.lines())
                .block(block)
                .style(Style::default().fg(theme::text()))
                .wrap(Wrap { trim: false }),
            area,
            buf,
        );
    }
}

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::{
    Anchor, Constraints, FlexFit, LayoutFlexColumn, LayoutRenderable, Size, StackRenderable,
};
use crate::theme;
use crate::widget::sparkline::Sparkline;

#[derive(Clone)]
pub struct State {
    pub sparkline: Sparkline,
    pub scroll: usize,
    pub tick_count: u64,
}

pub fn initial_state() -> State {
    State {
        sparkline: Sparkline::new(vec![]).label("tok/s".to_string()),
        scroll: 0,
        tick_count: 0,
    }
}

// ---------------------------------------------------------------------------
// TextBlock — multi-line text body
// ---------------------------------------------------------------------------

struct TextBlock {
    lines: Vec<String>,
    style: Style,
}

impl LayoutRenderable for TextBlock {
    fn measure(&self, constraints: Constraints) -> Size {
        Size::new(constraints.fill_width(), self.lines.len() as u16)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        for (i, line) in self.lines.iter().enumerate() {
            if i as u16 >= area.height {
                break;
            }
            buf.set_stringn(
                area.x,
                area.y + i as u16,
                line,
                area.width as usize,
                self.style,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// StatusPanel — bordered overlay
// ---------------------------------------------------------------------------

struct StatusPanel {
    items: Vec<&'static str>,
}

impl LayoutRenderable for StatusPanel {
    fn measure(&self, _: Constraints) -> Size {
        Size::new(20, self.items.len() as u16 + 2)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 3 {
            return;
        }

        let border_style = Style::default().fg(theme::border());
        let text_style = Style::default().fg(theme::success());

        // Top border: ┌─ status ──...─┐
        let label = " status ";
        let top = format!(
            "\u{250c}\u{2500}{}{}\u{2510}",
            label,
            "\u{2500}".repeat((area.width as usize).saturating_sub(2 + label.len()))
        );
        buf.set_stringn(area.x, area.y, &top, area.width as usize, border_style);

        // Item rows: │ {item}    │
        for (i, item) in self.items.iter().enumerate() {
            let row = area.y + 1 + i as u16;
            if row >= area.y + area.height.saturating_sub(1) {
                break;
            }
            buf.set_stringn(area.x, row, "\u{2502}", 1, border_style);
            let inner = format!(
                " {:<width$}",
                item,
                width = (area.width as usize).saturating_sub(3)
            );
            buf.set_stringn(
                area.x + 1,
                row,
                &inner,
                (area.width as usize).saturating_sub(2),
                text_style,
            );
            buf.set_stringn(
                area.x + area.width.saturating_sub(1),
                row,
                "\u{2502}",
                1,
                border_style,
            );
        }

        // Bottom border: └──...─┘
        let bottom_row = area.y + area.height.saturating_sub(1);
        let bottom = format!(
            "\u{2514}{}\u{2518}",
            "\u{2500}".repeat((area.width as usize).saturating_sub(2))
        );
        buf.set_stringn(
            area.x,
            bottom_row,
            &bottom,
            area.width as usize,
            border_style,
        );
    }
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 2 || area.width == 0 {
        return;
    }

    // Reserve bottom row for help.
    let content = Rect::new(area.x, area.y, area.width, area.height - 1);
    let help_y = area.y + area.height - 1;
    buf.set_stringn(
        area.x,
        help_y,
        " j/k scroll  a add data point",
        area.width as usize,
        Style::default().fg(theme::dim()),
    );

    // Build the text body.
    let lines: Vec<String> = (0..30)
        .map(|i| format!("  session line {}", i + state.scroll + 1))
        .collect();
    let body = TextBlock {
        lines,
        style: Style::default().fg(theme::text()),
    };

    // Build the status overlay.
    let status = StatusPanel {
        items: vec![
            "\u{2713} tests pass",
            "\u{2713} committed",
            "\u{2713} pushed",
        ],
    };

    // Flex column: body (flex=1) + sparkline (flex=0).
    let mut column = LayoutFlexColumn::new();
    column.push(1, FlexFit::Tight, body);
    column.push_ref(0, FlexFit::Loose, &state.sparkline);

    // Stack: column as base, status panel as overlay.
    let mut stack = StackRenderable::new(column);
    stack.overlay(status, Anchor::TopRight, (1, 1));
    stack.render(content, buf);
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            state.scroll += 1;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.scroll = state.scroll.saturating_sub(1);
        }
        KeyCode::Char('a') => {
            let value = ((state.tick_count * 7 + 13) % 100) as f64;
            state.sparkline.push(value);
            state.tick_count += 1;
        }
        _ => {}
    }
}

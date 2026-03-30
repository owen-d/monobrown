use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use tui_lib::devkit::playground;
use tui_lib::render::{Anchor, Constraints, LayoutRenderable, Size, StackRenderable};

const LINE_COUNT: usize = 50;
const OVERLAY_WIDTH: u16 = 20;
const OVERLAY_HEIGHT: u16 = 5;

struct DemoState {
    scroll: usize,
    overlay_visible: bool,
}

// -- Base: scrollable text body -----------------------------------------------

struct TextBody {
    scroll: usize,
    height: u16,
    width: u16,
}

impl LayoutRenderable for TextBody {
    fn measure(&self, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(self.width, self.height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let style = Style::default().fg(Color::White);
        for row in 0..area.height as usize {
            let line_num = self.scroll + row + 1;
            if line_num > LINE_COUNT {
                break;
            }
            let text = format!("Line {line_num}");
            buf.set_stringn(
                area.x,
                area.y + row as u16,
                &text,
                area.width as usize,
                style,
            );
        }
    }
}

// -- Overlay: status panel ----------------------------------------------------

struct StatusPanel;

impl LayoutRenderable for StatusPanel {
    fn measure(&self, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(OVERLAY_WIDTH, OVERLAY_HEIGHT))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let border_style = Style::default().fg(Color::DarkGray);
        let text_style = Style::default().fg(Color::Green);

        // Top border.
        let top = "\u{250c}".to_string()
            + &"\u{2500}".repeat((area.width as usize).saturating_sub(2))
            + "\u{2510}";
        buf.set_stringn(area.x, area.y, &top, area.width as usize, border_style);

        let lines = [
            "\u{2714} tests pass",
            "\u{2714} committed",
            "\u{2714} pushed",
        ];
        for (i, line) in lines.iter().enumerate() {
            let row = area.y + 1 + i as u16;
            if row >= area.y + area.height.saturating_sub(1) {
                break;
            }
            buf.set_stringn(area.x, row, "\u{2502}", 1, border_style);
            buf.set_stringn(
                area.x + 1,
                row,
                format!(
                    " {line:<width$}",
                    width = (area.width as usize).saturating_sub(3)
                ),
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

        // Bottom border.
        let bottom_row = area.y + area.height.saturating_sub(1);
        let bottom = "\u{2514}".to_string()
            + &"\u{2500}".repeat((area.width as usize).saturating_sub(2))
            + "\u{2518}";
        buf.set_stringn(
            area.x,
            bottom_row,
            &bottom,
            area.width as usize,
            border_style,
        );
    }
}

// -- Wiring -------------------------------------------------------------------

fn render(state: &DemoState, area: Rect, buf: &mut Buffer) {
    let body = TextBody {
        scroll: state.scroll,
        height: area.height,
        width: area.width,
    };

    if state.overlay_visible {
        let mut stack = StackRenderable::new(body);
        stack.overlay(StatusPanel, Anchor::TopRight, (1, 1));
        stack.render(area, buf);
    } else {
        body.render(area, buf);
    }
}

fn tick(_state: &mut DemoState, _dt: Duration) {}

fn main() -> std::io::Result<()> {
    let state = DemoState {
        scroll: 0,
        overlay_visible: true,
    };

    playground::run_animated_interactive(
        state,
        "Stack (Rearview Mirror)",
        render,
        tick,
        |s, key| match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                s.scroll = s.scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if s.scroll + 1 < LINE_COUNT => {
                s.scroll += 1;
            }
            KeyCode::Char('v') => {
                s.overlay_visible = !s.overlay_visible;
            }
            _ => {}
        },
        Duration::from_millis(16),
    )
}

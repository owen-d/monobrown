use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use tui_lib::devkit::playground;
use tui_lib::render::LayoutRenderable;
use tui_lib::theme;
use tui_lib::widget::queue_list::QueueList;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

enum Focus {
    Body,
    Sidebar,
}

struct DemoState {
    queue: QueueList,
    body_lines: Vec<String>,
    next_id: u32,
    focus: Focus,
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn render(state: &DemoState, area: Rect, buf: &mut Buffer) {
    if area.height < 2 || area.width < 30 {
        return;
    }

    // Help line at bottom.
    let help = match state.focus {
        Focus::Body => " Tab focus sidebar  n add notification",
        Focus::Sidebar => " Tab focus body  j/k navigate  d dismiss  J/K reorder",
    };
    buf.set_stringn(
        area.x,
        area.y + area.height - 1,
        help,
        area.width as usize,
        Style::default().fg(theme::dim()),
    );

    let content_height = area.height - 1;
    let sidebar_width: u16 = 24;
    let body_width = area.width.saturating_sub(sidebar_width + 1);
    let text_style = Style::default().fg(theme::text());
    let dim_style = Style::default().fg(theme::dim());
    let border_style = Style::default().fg(theme::border());

    // Body: render text lines.
    let body_area = Rect::new(area.x, area.y, body_width, content_height);
    for (i, line) in state.body_lines.iter().enumerate() {
        if i as u16 >= body_area.height {
            break;
        }
        buf.set_stringn(
            body_area.x,
            body_area.y + i as u16,
            line,
            body_area.width as usize,
            text_style,
        );
    }

    // Separator: vertical line of │.
    let sep_x = area.x + body_width;
    for row in 0..content_height {
        buf.set_stringn(sep_x, area.y + row, "\u{2502}", 1, border_style);
    }

    // Sidebar.
    let sidebar_x = sep_x + 1;
    let sidebar_area = Rect::new(sidebar_x, area.y, sidebar_width, content_height);

    // Header.
    let header_style = match state.focus {
        Focus::Sidebar => Style::default().fg(theme::focus()),
        Focus::Body => dim_style,
    };
    buf.set_stringn(
        sidebar_area.x,
        sidebar_area.y,
        "Notifications",
        sidebar_area.width as usize,
        header_style,
    );

    // QueueList below header.
    if sidebar_area.height > 1 {
        let list_area = Rect::new(
            sidebar_area.x,
            sidebar_area.y + 1,
            sidebar_area.width,
            sidebar_area.height.saturating_sub(1),
        );
        state.queue.render(list_area, buf);
    }
}

fn tick(_state: &mut DemoState, _dt: Duration) {}

fn apply(state: &mut DemoState, key: &crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Tab => {
            state.focus = match state.focus {
                Focus::Body => Focus::Sidebar,
                Focus::Sidebar => Focus::Body,
            };
        }
        _ => match state.focus {
            Focus::Body => {
                if key.code == KeyCode::Char('n') {
                    state.queue.push(format!("[i] Event #{}", state.next_id));
                    state.next_id += 1;
                }
            }
            Focus::Sidebar => match key.code {
                KeyCode::Down | KeyCode::Char('j') => state.queue.next(),
                KeyCode::Up | KeyCode::Char('k') => state.queue.prev(),
                KeyCode::Char('d') if !state.queue.is_empty() => {
                    let idx = state.queue.selected();
                    state.queue.remove(idx);
                }
                KeyCode::Char('J') => state.queue.move_down(),
                KeyCode::Char('K') => state.queue.move_up(),
                _ => {}
            },
        },
    }
}

fn main() -> std::io::Result<()> {
    let body_lines: Vec<String> = (0..20)
        .map(|i| format!("  Document line {}", i + 1))
        .collect();

    let state = DemoState {
        queue: QueueList::new(vec![
            "[!] Build failed".into(),
            "[i] PR merged".into(),
            "[i] Tests pass".into(),
        ]),
        body_lines,
        next_id: 4,
        focus: Focus::Body,
    };

    playground::run_animated_interactive(
        state,
        "Notification Sidebar",
        render,
        tick,
        apply,
        Duration::from_millis(100),
    )
}

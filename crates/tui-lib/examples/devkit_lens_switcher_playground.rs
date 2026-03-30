use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use mb_tui::devkit::playground;
use mb_tui::render::LayoutRenderable;
use mb_tui::theme;
use mb_tui::widget::flashcard::Flashcard;
use mb_tui::widget::gauge::Gauge;
use mb_tui::widget::hotkey_coach::{HotkeyCoach, Orientation, Shortcut};
use mb_tui::widget::tab_bar::TabBar;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

enum Focus {
    Tabs,
    Content,
}

struct DemoState {
    tab_bar: TabBar,
    gauge: Gauge,
    flashcard: Flashcard,
    coach: HotkeyCoach,
    focus: Focus,
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn render(state: &DemoState, area: Rect, buf: &mut Buffer) {
    if area.height < 4 || area.width == 0 {
        return;
    }

    // Help line at bottom.
    let help = match state.focus {
        Focus::Tabs => " Tab focus content  h/l switch tabs",
        Focus::Content => match state.tab_bar.selected() {
            0 => " Tab focus tabs  h/l adjust gauge",
            1 => " Tab focus tabs  Space flip card",
            _ => " Tab focus tabs",
        },
    };
    buf.set_stringn(
        area.x,
        area.y + area.height - 1,
        help,
        area.width as usize,
        Style::default().fg(theme::dim()),
    );

    // Row 0: tab bar.
    let tab_area = Rect::new(area.x, area.y, area.width, 1);
    state.tab_bar.render(tab_area, buf);

    // Row 1: separator line.
    let sep_style = Style::default().fg(theme::border());
    for x in area.x..area.x + area.width {
        buf.set_stringn(x, area.y + 1, "\u{2500}", 1, sep_style);
    }

    // Row 2+: content area (minus help row).
    let content_area = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(3),
    );

    match state.tab_bar.selected() {
        0 => state.gauge.render(content_area, buf),
        1 => state.flashcard.render(content_area, buf),
        2 => state.coach.render(content_area, buf),
        _ => {}
    }
}

fn tick(_state: &mut DemoState, _dt: Duration) {}

fn apply(state: &mut DemoState, key: &crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Tab => {
            state.focus = match state.focus {
                Focus::Tabs => Focus::Content,
                Focus::Content => Focus::Tabs,
            };
        }
        _ => match state.focus {
            Focus::Tabs => match key.code {
                KeyCode::Left | KeyCode::Char('h') => state.tab_bar.prev(),
                KeyCode::Right | KeyCode::Char('l') => state.tab_bar.next(),
                _ => {}
            },
            Focus::Content => match state.tab_bar.selected() {
                0 => match key.code {
                    KeyCode::Char('h') | KeyCode::Left => {
                        state.gauge.set_value(state.gauge.value() - 0.05);
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        state.gauge.set_value(state.gauge.value() + 0.05);
                    }
                    _ => {}
                },
                1 if key.code == KeyCode::Char(' ') => {
                    state.flashcard.flip();
                }
                _ => {}
            },
        },
    }
}

fn main() -> std::io::Result<()> {
    let state = DemoState {
        tab_bar: TabBar::new(vec!["Gauge".into(), "Flashcard".into(), "Hotkeys".into()]),
        gauge: Gauge::new(0.5)
            .label("Confidence".to_string())
            .low_label("low".to_string())
            .high_label("high".to_string())
            .gradient(),
        flashcard: Flashcard::new(
            "What is a trait?".into(),
            "A collection of methods defined for an unknown type".into(),
        ),
        coach: HotkeyCoach::new(vec![
            Shortcut::new("Tab", "switch focus"),
            Shortcut::new("h/l", "navigate"),
            Shortcut::new("Space", "flip card"),
        ])
        .orientation(Orientation::Vertical),
        focus: Focus::Tabs,
    };

    playground::run_animated_interactive(
        state,
        "Lens Switcher",
        render,
        tick,
        apply,
        Duration::from_millis(100),
    )
}

use std::time::Duration;

use crossterm::event::KeyCode;
use tui_lib::devkit::playground;
use tui_lib::widget::bar_selector::{BarSelector, render_bar_selector};

fn main() -> std::io::Result<()> {
    let state = BarSelector::new(&["Alpha", "Beta", "Gamma"]);

    playground::run_animated_interactive(
        state,
        "Bar Selector",
        render_bar_selector,
        BarSelector::tick,
        |s, key| match key.code {
            KeyCode::Char('1') => s.select(0),
            KeyCode::Char('2') => s.select(1),
            KeyCode::Char('3') => s.select(2),
            KeyCode::Left | KeyCode::Char('h') => s.select_prev(),
            KeyCode::Right | KeyCode::Char('l') => s.select_next(),
            _ => {}
        },
        Duration::from_millis(16),
    )
}

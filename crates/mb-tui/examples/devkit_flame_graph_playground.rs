use std::time::Duration;

use mb_tui::devkit::flame_graph::test_flame_graph;
use mb_tui::devkit::playground;
use mb_tui::widget::flame_graph::{BarStyle, FlameGraph, render_flame_graph};

fn main() -> std::io::Result<()> {
    let state = test_flame_graph();

    playground::run_animated_interactive(
        state,
        "Flame Graph",
        render_flame_graph,
        FlameGraph::tick,
        |s, key| {
            use crossterm::event::KeyCode;
            if key.code == KeyCode::Char('b') {
                let next = match s.bar_style() {
                    BarStyle::ThinLine => BarStyle::Dotted,
                    BarStyle::Dotted => BarStyle::ThinLine,
                };
                s.set_bar_style(next);
            } else {
                s.handle_key(key);
            }
        },
        Duration::from_millis(16),
    )
}

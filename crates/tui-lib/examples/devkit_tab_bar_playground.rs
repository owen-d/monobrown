use std::time::Duration;

use mb_tui::devkit::{playground, tab_bar_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        tab_bar_demo::initial_state(),
        "Tab Bar \u{2014} Lens Switcher",
        tab_bar_demo::render,
        tab_bar_demo::tick,
        tab_bar_demo::apply,
        Duration::from_millis(16),
    )
}

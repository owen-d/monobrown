use std::time::Duration;

use tui_lib::devkit::{playground, sparkline_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        sparkline_demo::initial_state(),
        "Sparkline",
        sparkline_demo::render,
        sparkline_demo::tick,
        sparkline_demo::apply,
        Duration::from_millis(100),
    )
}

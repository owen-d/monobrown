use std::time::Duration;

use tui_lib::devkit::{playground, progress_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        progress_demo::initial_state(),
        "Progress Bar Demo",
        progress_demo::render,
        progress_demo::tick,
        progress_demo::apply,
        Duration::from_millis(50),
    )
}

use std::time::Duration;

use tui_lib::devkit::{playground, queue_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        queue_demo::initial_state(),
        "Queue List",
        queue_demo::render,
        queue_demo::tick,
        queue_demo::apply,
        Duration::from_millis(100),
    )
}

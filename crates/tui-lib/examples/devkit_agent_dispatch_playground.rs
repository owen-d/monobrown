use std::time::Duration;

use tui_lib::devkit::{dispatch_demo, playground};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        dispatch_demo::initial_state(),
        "Agent Dispatch",
        dispatch_demo::render,
        dispatch_demo::tick,
        dispatch_demo::apply,
        Duration::from_millis(100),
    )
}

use std::time::Duration;

use tui_lib::devkit::{gauge_demo, playground};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        gauge_demo::initial_state(),
        "Gauge",
        gauge_demo::render,
        gauge_demo::tick,
        gauge_demo::apply,
        Duration::from_millis(50),
    )
}

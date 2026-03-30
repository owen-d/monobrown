use std::time::Duration;

use tui_lib::devkit::simple_widgets::render_spinner;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tui_lib::devkit::playground::run_animated(
        Duration::ZERO,
        "spinner",
        render_spinner,
        |elapsed, dt| *elapsed += dt,
        Duration::from_millis(100),
    )
}

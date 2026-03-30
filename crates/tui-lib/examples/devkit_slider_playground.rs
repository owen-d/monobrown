use std::time::Duration;

use mb_tui::devkit::{playground, slider_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        slider_demo::initial_state(),
        "Range Slider",
        slider_demo::render,
        slider_demo::tick,
        slider_demo::apply,
        Duration::from_millis(100),
    )
}

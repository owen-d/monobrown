use std::time::Duration;

use tui_lib::devkit::{playground, stepper_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        stepper_demo::initial_state(),
        "Stepper",
        stepper_demo::render,
        stepper_demo::tick,
        stepper_demo::apply,
        Duration::from_millis(100),
    )
}

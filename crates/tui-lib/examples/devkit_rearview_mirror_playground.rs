use std::time::Duration;

use tui_lib::devkit::{playground, rearview_mirror_demo};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        rearview_mirror_demo::initial_state(),
        "Rearview Mirror",
        rearview_mirror_demo::render,
        rearview_mirror_demo::tick,
        rearview_mirror_demo::apply,
        Duration::from_millis(100),
    )
}

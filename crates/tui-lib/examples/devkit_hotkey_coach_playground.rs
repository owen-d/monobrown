use std::time::Duration;

use mb_tui::devkit::{hotkey_coach_demo, playground};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        hotkey_coach_demo::initial_state(),
        "Hotkey Coach",
        hotkey_coach_demo::render,
        hotkey_coach_demo::tick,
        hotkey_coach_demo::apply,
        Duration::from_millis(100),
    )
}

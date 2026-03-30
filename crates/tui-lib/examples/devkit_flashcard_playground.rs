use std::time::Duration;

use mb_tui::devkit::{flashcard_demo, playground};

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        flashcard_demo::initial_state(),
        "Flashcard",
        flashcard_demo::render,
        flashcard_demo::tick,
        flashcard_demo::apply,
        Duration::from_millis(100),
    )
}

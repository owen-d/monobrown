use std::time::Duration;

use mb_tui::devkit::frame_tape::FrameTape;
use mb_tui::devkit::simple_widgets::{ShimmerDemo, render_shimmer};

const TEXT: &str = "Loading";
const WIDTH: u16 = 20;
const HEIGHT: u16 = 1;
const STEP_MS: u64 = 16;
const CYCLE_MS: u64 = 2000;
const TERMINAL_BG: (u8, u8, u8) = (0, 0, 0);

fn shimmer_tape() -> FrameTape {
    let num_steps = CYCLE_MS / STEP_MS;
    let states = (0..=num_steps).map(|i| {
        let ms = i * STEP_MS;
        (
            format!("t={ms}ms"),
            ShimmerDemo {
                text: TEXT,
                elapsed: Duration::from_millis(ms),
                has_true_color: true,
            },
        )
    });
    FrameTape::record_states(states, render_shimmer, WIDTH, HEIGHT)
}

#[test]
fn shimmer_contrast_large_text() {
    let tape = shimmer_tape();
    // Shimmer base color (100,100,100) on black gives ~3.5:1 ratio.
    // Use large-text AA threshold (3.0:1) since shimmer is decorative bold text.
    tape.assert_contrast_aa_large(TERMINAL_BG);
}

#[test]
fn shimmer_smoothness() {
    let tape = shimmer_tape();
    // Shimmer should animate smoothly. CV below 1.0 means frame-to-frame
    // deltas are reasonably uniform (constant sweep speed).
    tape.assert_smooth(1.0);
}

#[test]
fn shimmer_coverage() {
    let tape = shimmer_tape();
    // Over a full cycle, the shimmer band should visit all text characters.
    // The text "Loading" is 7 chars in a 20-wide surface.
    tape.assert_coverage(0.3);
}

#[test]
fn shimmer_periodicity() {
    let tape = shimmer_tape();
    // After one full cycle, the animation should loop back.
    tape.assert_periodic(0.1);
}

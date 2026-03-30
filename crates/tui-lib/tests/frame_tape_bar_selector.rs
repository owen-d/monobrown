use std::time::Duration;

use tui_lib::devkit::frame_tape::FrameTape;
use tui_lib::widget::bar_selector::{BarSelector, render_bar_selector};

const WIDTH: u16 = 30;
const HEIGHT: u16 = 10;
const TERMINAL_BG: (u8, u8, u8) = (0, 0, 0);

/// Record a transition from slot 0 to slot 1.
fn transition_tape() -> FrameTape {
    let mut state = BarSelector::new(&["A", "B", "C"]);
    let dt = Duration::from_millis(16);

    // Generate states: first frame at rest, then select slot 1, then tick
    // through transition.
    let mut states: Vec<(String, BarSelector)> = Vec::new();
    states.push(("initial".to_string(), state.clone()));

    state.select(1);
    for i in 1..=25 {
        // 400ms of transition
        state.tick(dt);
        states.push((format!("t={}ms", i * 16), state.clone()));
    }

    FrameTape::record_states(states, render_bar_selector, WIDTH, HEIGHT)
}

#[test]
fn transition_smoothness() {
    let tape = transition_tape();
    // Animation should be smooth -- exponential decay produces uniform-ish
    // deltas.
    tape.assert_smooth(1.5);
}

#[test]
fn transition_contrast() {
    let tape = transition_tape();
    // All rendered content should be readable.
    tape.assert_contrast_aa_large(TERMINAL_BG);
}

#[test]
fn round_trip_periodicity() {
    let mut state = BarSelector::new(&["A", "B", "C"]);
    let dt = Duration::from_millis(16);

    let mut states: Vec<(String, BarSelector)> = Vec::new();
    states.push(("start".to_string(), state.clone()));

    // Transition to slot 1.
    state.select(1);
    for i in 1..=25 {
        state.tick(dt);
        states.push((format!("fwd-{}ms", i * 16), state.clone()));
    }

    // Transition back to slot 0.
    state.select(0);
    for i in 1..=25 {
        state.tick(dt);
        states.push((format!("back-{}ms", i * 16), state.clone()));
    }

    let tape = FrameTape::record_states(states, render_bar_selector, WIDTH, HEIGHT);
    // Start and end should be near-identical (we're back to slot 0 fully
    // converged).
    tape.assert_periodic(0.05);
}

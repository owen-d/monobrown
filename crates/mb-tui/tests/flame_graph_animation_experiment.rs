//! Experiment: measure and compare animation parameters for the flame graph.
//!
//! The current constants (TRANSITION_MS=300, DECAY_TIME_CONSTANTS=5.0) produce
//! an animation that is "barely perceptible." This file records visual output
//! at each tick and computes metrics to evaluate four candidate parameter sets.
//!
//! Run with:
//! ```sh
//! cargo test -p mb-tui --test flame_graph_animation_experiment -- --nocapture
//! ```

#![cfg(feature = "devkit")]

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use mb_tui::devkit::flame_graph::test_flame_graph;
use mb_tui::devkit::frame_tape::FrameTape;
use mb_tui::widget::flame_graph::{FlameGraph, render_flame_graph};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const WIDTH: u16 = 60;
const HEIGHT: u16 = 12;
const STEP_MS: u64 = 16;
/// Total recording window per phase (expand or collapse).
const RECORD_MS: u64 = 1200;
const NUM_STEPS: u64 = RECORD_MS / STEP_MS;
/// A frame delta is "meaningful" if this fraction of cells changed color.
const MEANINGFUL_THRESHOLD: f64 = 0.05;

// ---------------------------------------------------------------------------
// Easing curve variants (pure math)
// ---------------------------------------------------------------------------

struct Variant {
    name: &'static str,
    transition_ms: f64,
    time_constants: f64,
}

const VARIANTS: [Variant; 4] = [
    Variant {
        name: "A (current)",
        transition_ms: 300.0,
        time_constants: 5.0,
    },
    Variant {
        name: "B (slower)",
        transition_ms: 600.0,
        time_constants: 5.0,
    },
    Variant {
        name: "C (underdamped)",
        transition_ms: 500.0,
        time_constants: 3.0,
    },
    Variant {
        name: "D (floaty)",
        transition_ms: 800.0,
        time_constants: 2.5,
    },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Simulate the exponential-decay easing for one step.
///
/// Returns the new value after applying the decay formula:
///   value += (target - value) * (1 - exp(-k * dt))
/// where k = time_constants / (transition_ms / 1000).
fn ease_step(
    value: f64,
    target: f64,
    dt_secs: f64,
    transition_ms: f64,
    time_constants: f64,
) -> f64 {
    let k = time_constants / (transition_ms / 1000.0);
    let factor = 1.0 - (-k * dt_secs).exp();
    let new = value + (target - value) * factor;
    if (new - target).abs() < 0.001 {
        target
    } else {
        new
    }
}

/// Record a FrameTape for an expand followed by ticking. Uses `fg.tick()`
/// (variant A parameters only, since those are the compiled-in constants).
fn record_expand_tape(fg: &mut FlameGraph) -> FrameTape {
    let dt = Duration::from_millis(STEP_MS);

    let mut states: Vec<(String, FlameGraph)> = Vec::new();
    states.push(("t=0ms (pre-expand)".to_string(), fg.clone()));

    // Press Right to expand.
    fg.handle_key(&make_key(KeyCode::Right));
    states.push(("t=0ms (post-key)".to_string(), fg.clone()));

    for i in 1..=NUM_STEPS {
        fg.tick(dt);
        let ms = i * STEP_MS;
        states.push((format!("t={ms}ms"), fg.clone()));
    }

    FrameTape::record_states(states, render_flame_graph, WIDTH, HEIGHT)
}

/// Record a FrameTape for a collapse following an already-expanded state.
fn record_collapse_tape(fg: &mut FlameGraph) -> FrameTape {
    let dt = Duration::from_millis(STEP_MS);

    let mut states: Vec<(String, FlameGraph)> = Vec::new();
    states.push(("t=0ms (pre-collapse)".to_string(), fg.clone()));

    // Press Left to collapse.
    fg.handle_key(&make_key(KeyCode::Left));
    states.push(("t=0ms (post-key)".to_string(), fg.clone()));

    for i in 1..=NUM_STEPS {
        fg.tick(dt);
        let ms = i * STEP_MS;
        states.push((format!("t={ms}ms"), fg.clone()));
    }

    FrameTape::record_states(states, render_flame_graph, WIDTH, HEIGHT)
}

/// Count the number of frames with "meaningful" visual change.
///
/// A transition between two frames is meaningful if the fraction of cells
/// whose foreground color changed exceeds `MEANINGFUL_THRESHOLD`.
fn count_perceptible_frames(tape: &FrameTape) -> usize {
    let total_cells = WIDTH as usize * HEIGHT as usize;
    if total_cells == 0 {
        return 0;
    }
    tape.frame_deltas()
        .iter()
        .filter(|d| d.cells_changed as f64 / total_cells as f64 > MEANINGFUL_THRESHOLD)
        .count()
}

/// Print a summary report for a recorded tape.
fn report_tape(label: &str, tape: &FrameTape) {
    let smoothness = tape.smoothness();
    let coverage = tape.coverage();
    let perceptible = count_perceptible_frames(tape);
    let deltas = tape.frame_deltas();

    eprintln!("=== {label} ===");
    eprintln!("  frames:      {}", tape.len());
    eprintln!("  smoothness:  {smoothness:.4} (CV of frame deltas; lower = smoother)");
    eprintln!(
        "  coverage:    {:.1}% of cells changed at least once",
        coverage * 100.0
    );
    eprintln!(
        "  perceptible: {perceptible} frames with >{:.0}% cell change",
        MEANINGFUL_THRESHOLD * 100.0
    );

    // Show first 10 delta magnitudes to visualize the decay curve.
    let first_n = deltas.len().min(20);
    eprintln!("  delta magnitudes (first {first_n}):");
    for d in deltas.iter().take(first_n) {
        let bar_len = (d.total_distance / 50.0).min(40.0) as usize;
        let bar: String = "#".repeat(bar_len);
        eprintln!(
            "    {:<22} cells={:<4} dist={:>8.1}  {bar}",
            d.from_label, d.cells_changed, d.total_distance,
        );
    }
    eprintln!();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Record expand and collapse with the current parameters (variant A) and
/// report visual metrics.
#[test]
fn test_variant_a_current_expand() {
    let mut fg = test_flame_graph();
    let tape = record_expand_tape(&mut fg);
    report_tape("Variant A — Expand (TRANSITION_MS=300, k=5.0)", &tape);

    // Sanity: the animation should have produced at least some visual change.
    assert!(
        tape.coverage() > 0.0,
        "expand should change at least one cell"
    );
}

#[test]
fn test_variant_a_current_collapse() {
    let mut fg = test_flame_graph();

    // First expand fully so we have something to collapse.
    fg.handle_key(&make_key(KeyCode::Right));
    for _ in 0..80 {
        fg.tick(Duration::from_millis(STEP_MS));
    }

    let tape = record_collapse_tape(&mut fg);
    report_tape("Variant A — Collapse (TRANSITION_MS=300, k=5.0)", &tape);

    assert!(
        tape.coverage() > 0.0,
        "collapse should change at least one cell"
    );
}

/// Pure-math comparison of all four easing curves.
///
/// For each variant, simulates the animation value from 0.0 to 1.0 at 16ms
/// steps, printing the progression and convergence tick.
#[test]
#[allow(clippy::too_many_lines)]
fn test_animation_curve_comparison() {
    let dt_secs = STEP_MS as f64 / 1000.0;
    let max_ticks = 100usize;

    eprintln!("=== Easing Curve Comparison (0 -> 1 transition) ===");
    eprintln!();

    for variant in &VARIANTS {
        let mut value = 0.002; // matches SNAP_EPSILON * 2.0 starting value
        let target = 1.0;
        let mut converge_tick: Option<usize> = None;
        let mut samples: Vec<String> = Vec::new();

        for tick in 0..max_ticks {
            if converge_tick.is_none() {
                samples.push(format!("{value:.3}"));
            }
            if value == target && converge_tick.is_none() {
                converge_tick = Some(tick);
            }
            value = ease_step(
                value,
                target,
                dt_secs,
                variant.transition_ms,
                variant.time_constants,
            );
        }

        let conv = converge_tick
            .map(|t| format!("tick {t} ({:.0}ms)", t as f64 * STEP_MS as f64))
            .unwrap_or_else(|| "not converged".to_string());

        eprintln!(
            "  {:20} TRANSITION_MS={:<6.0} TIME_CONSTANTS={:<4.1}  converges at {conv}",
            variant.name, variant.transition_ms, variant.time_constants,
        );

        // Print sampled values (up to convergence + 2).
        let display_count = samples.len().min(30);
        let displayed: Vec<&str> = samples
            .iter()
            .take(display_count)
            .map(String::as_str)
            .collect();
        eprintln!("    values: {}", displayed.join(" -> "));
        eprintln!();
    }

    // Also show the collapse direction (1 -> 0).
    eprintln!("=== Easing Curve Comparison (1 -> 0 transition) ===");
    eprintln!();

    for variant in &VARIANTS {
        let mut value = 1.0;
        let target = 0.0;
        let mut converge_tick: Option<usize> = None;
        let mut samples: Vec<String> = Vec::new();

        for tick in 0..max_ticks {
            if converge_tick.is_none() {
                samples.push(format!("{value:.3}"));
            }
            if value == target && converge_tick.is_none() {
                converge_tick = Some(tick);
            }
            value = ease_step(
                value,
                target,
                dt_secs,
                variant.transition_ms,
                variant.time_constants,
            );
        }

        let conv = converge_tick
            .map(|t| format!("tick {t} ({:.0}ms)", t as f64 * STEP_MS as f64))
            .unwrap_or_else(|| "not converged".to_string());

        eprintln!(
            "  {:20} TRANSITION_MS={:<6.0} TIME_CONSTANTS={:<4.1}  converges at {conv}",
            variant.name, variant.transition_ms, variant.time_constants,
        );

        let display_count = samples.len().min(30);
        let displayed: Vec<&str> = samples
            .iter()
            .take(display_count)
            .map(String::as_str)
            .collect();
        eprintln!("    values: {}", displayed.join(" -> "));
        eprintln!();
    }
}

/// Measure how many frames of visible animation the user actually sees with
/// the current parameters.
///
/// This renders the flame graph at every 16ms tick during an expand and a
/// collapse, counting frames where more than 5% of cells changed color.
#[test]
fn test_visual_perceptibility_current() {
    let mut fg = test_flame_graph();

    // --- Expand phase ---
    let expand_tape = record_expand_tape(&mut fg);
    let expand_perceptible = count_perceptible_frames(&expand_tape);

    // --- Settle, then collapse ---
    for _ in 0..80 {
        fg.tick(Duration::from_millis(STEP_MS));
    }
    let collapse_tape = record_collapse_tape(&mut fg);
    let collapse_perceptible = count_perceptible_frames(&collapse_tape);

    eprintln!("=== Visual Perceptibility (Variant A, current) ===");
    eprintln!(
        "  Expand:   {expand_perceptible} perceptible frames out of {}",
        expand_tape.len() - 1
    );
    eprintln!(
        "  Collapse: {collapse_perceptible} perceptible frames out of {}",
        collapse_tape.len() - 1
    );
    eprintln!();

    // Print per-frame cell change percentages for expand.
    let total_cells = WIDTH as usize * HEIGHT as usize;
    eprintln!("  Expand frame-by-frame change %:");
    for d in expand_tape.frame_deltas().iter().take(30) {
        let pct = d.cells_changed as f64 / total_cells as f64 * 100.0;
        let bar_len = (pct / 2.0).min(40.0) as usize;
        let bar: String = "|".repeat(bar_len);
        let marker = if pct > MEANINGFUL_THRESHOLD * 100.0 {
            "*"
        } else {
            " "
        };
        eprintln!("   {marker} {:<22} {pct:>5.1}%  {bar}", d.from_label,);
    }
    eprintln!();

    eprintln!("  Collapse frame-by-frame change %:");
    for d in collapse_tape.frame_deltas().iter().take(30) {
        let pct = d.cells_changed as f64 / total_cells as f64 * 100.0;
        let bar_len = (pct / 2.0).min(40.0) as usize;
        let bar: String = "|".repeat(bar_len);
        let marker = if pct > MEANINGFUL_THRESHOLD * 100.0 {
            "*"
        } else {
            " "
        };
        eprintln!("   {marker} {:<22} {pct:>5.1}%  {bar}", d.from_label,);
    }
    eprintln!();
}

/// Side-by-side summary table of all four variants' theoretical behaviour.
///
/// Since variants B-D require different constants than the compiled-in ones,
/// this test computes per-tick animation values mathematically and derives
/// metrics that predict visual behaviour without rendering.
#[test]
fn test_variant_comparison_table() {
    let dt_secs = STEP_MS as f64 / 1000.0;

    eprintln!("=== Variant Comparison Table ===");
    eprintln!();
    eprintln!(
        "  {:<20} {:>6} {:>4} {:>10} {:>10} {:>12} {:>14}",
        "Variant", "T(ms)", "k", "Conv.Tick", "Conv.Time", "50% at tick", "90% at tick"
    );
    eprintln!("  {}", "-".repeat(80));

    for variant in &VARIANTS {
        let mut value = 0.002;
        let target = 1.0;
        let mut converge_tick: Option<usize> = None;
        let mut half_tick: Option<usize> = None;
        let mut ninety_tick: Option<usize> = None;

        for tick in 0..200 {
            if value >= 0.5 && half_tick.is_none() {
                half_tick = Some(tick);
            }
            if value >= 0.9 && ninety_tick.is_none() {
                ninety_tick = Some(tick);
            }
            if value == target && converge_tick.is_none() {
                converge_tick = Some(tick);
            }
            value = ease_step(
                value,
                target,
                dt_secs,
                variant.transition_ms,
                variant.time_constants,
            );
        }

        let conv_str = converge_tick
            .map(|t| format!("{t}"))
            .unwrap_or_else(|| ">200".to_string());
        let conv_time = converge_tick
            .map(|t| format!("{:.0}ms", t as f64 * STEP_MS as f64))
            .unwrap_or_else(|| ">3200ms".to_string());
        let half_str = half_tick
            .map(|t| format!("{t} ({:.0}ms)", t as f64 * STEP_MS as f64))
            .unwrap_or_else(|| ">200".to_string());
        let ninety_str = ninety_tick
            .map(|t| format!("{t} ({:.0}ms)", t as f64 * STEP_MS as f64))
            .unwrap_or_else(|| ">200".to_string());

        eprintln!(
            "  {:<20} {:>6.0} {:>4.1} {:>10} {:>10} {:>12} {:>14}",
            variant.name,
            variant.transition_ms,
            variant.time_constants,
            conv_str,
            conv_time,
            half_str,
            ninety_str,
        );
    }

    eprintln!();
    eprintln!("  Key insight: at 60fps (16ms/frame), the user needs at least");
    eprintln!("  3-5 perceptible frames to perceive smooth motion. Fewer than");
    eprintln!("  3 reads as an instant jump.");
    eprintln!();
}

/// Record the actual rendered expand tape and print the filmstrip for the
/// first few frames, so a human reviewer can see what the animation looks
/// like in text form.
#[test]
fn test_expand_filmstrip_sample() {
    let mut fg = test_flame_graph();
    let dt = Duration::from_millis(STEP_MS);

    // Record just the first 8 frames of an expand for a compact filmstrip.
    let mut states: Vec<(String, FlameGraph)> = Vec::new();
    states.push(("before".to_string(), fg.clone()));

    fg.handle_key(&make_key(KeyCode::Right));
    states.push(("after-key".to_string(), fg.clone()));

    for i in 1..=6 {
        fg.tick(dt);
        let ms = i * STEP_MS;
        states.push((format!("t={ms}ms"), fg.clone()));
    }

    let tape = FrameTape::record_states(states, render_flame_graph, WIDTH, HEIGHT);

    eprintln!("=== Expand Filmstrip (first 8 frames, {WIDTH}x{HEIGHT}) ===");
    eprintln!();
    eprintln!("{}", tape.to_filmstrip());

    // Also show the diff strip for the same range.
    eprintln!("=== Expand Diff Strip ===");
    eprintln!();
    eprintln!("{}", tape.to_diff_strip());
}

/// Round-trip test: expand then collapse should return to approximately the
/// same visual state.
#[test]
fn test_expand_collapse_round_trip() {
    let mut fg = test_flame_graph();
    let dt = Duration::from_millis(STEP_MS);

    // Record initial state.
    let mut states: Vec<(String, FlameGraph)> = Vec::new();
    states.push(("start".to_string(), fg.clone()));

    // Expand.
    fg.handle_key(&make_key(KeyCode::Right));
    for i in 1..=40 {
        fg.tick(dt);
        states.push((format!("expand-t={}ms", i * STEP_MS), fg.clone()));
    }

    // Collapse.
    fg.handle_key(&make_key(KeyCode::Left));
    for i in 1..=40 {
        fg.tick(dt);
        states.push((format!("collapse-t={}ms", i * STEP_MS), fg.clone()));
    }

    let tape = FrameTape::record_states(states, render_flame_graph, WIDTH, HEIGHT);
    let periodicity = tape.periodicity();

    eprintln!("=== Round-Trip (expand + collapse) ===");
    eprintln!("  periodicity: {periodicity:.4} (0 = identical start/end)");
    eprintln!("  smoothness:  {:.4}", tape.smoothness());
    eprintln!("  coverage:    {:.1}%", tape.coverage() * 100.0);
    eprintln!();

    // The round trip should end close to where it started. We use a generous
    // threshold because the cursor position may differ slightly.
    assert!(
        periodicity < 0.5,
        "round trip should return near-original state, got periodicity={periodicity:.4}"
    );
}

#![cfg(feature = "devkit")]

use mb_tui::devkit::frame_tape::FrameTape;
use mb_tui::devkit::trace_view::trace_view_interactive_catalog;
use mb_tui::widget::trace_view::render_trace_view;

/// Wide goldens lock geometry, labels, semantic styles, and interaction outcomes.
#[test]
fn trace_view_wide_snapshots() {
    let catalog = trace_view_interactive_catalog();
    catalog.assert_all_snapshots(100, 14);
    catalog.assert_all_styled_snapshots(100, 14);
    catalog.assert_all_snapshots_after_inputs(100, 14);
    catalog.assert_all_styled_snapshots_after_inputs(100, 14);
}

/// Narrow goldens prove semantic labels and uncertainty survive responsive degradation.
#[test]
fn trace_view_narrow_snapshots() {
    let catalog = trace_view_interactive_catalog();
    catalog.assert_all_snapshots(28, 8);
    catalog.assert_all_styled_snapshots(28, 8);
}

/// Visible semantic marks meet the same large-text contrast floor as the flamegraph.
#[test]
fn trace_view_semantic_palette_has_dark_terminal_contrast() {
    let catalog = trace_view_interactive_catalog();
    let states: Vec<_> = (0..catalog.len())
        .map(|index| {
            (
                catalog.name(index).to_owned(),
                catalog.initial_state(index).clone(),
            )
        })
        .collect();
    let tape = FrameTape::record_states(states, render_trace_view, 100, 14);
    tape.assert_contrast_aa_large((0, 0, 0));
}

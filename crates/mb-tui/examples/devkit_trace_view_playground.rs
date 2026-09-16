//! Explore trace projection over the flame-graph presentation engine.

use std::time::Duration;

use mb_tui::devkit::playground;
use mb_tui::devkit::trace_view::trace_view_interactive_catalog;
use mb_tui::widget::trace_view::{TraceView, render_trace_view};

fn main() -> std::io::Result<()> {
    let catalog = trace_view_interactive_catalog();
    playground::run_animated_interactive(
        catalog.initial_state(0).clone(),
        "Trace View",
        render_trace_view,
        TraceView::tick,
        |state, key| {
            state.handle_key(key);
        },
        Duration::from_millis(16),
    )
}

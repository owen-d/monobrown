//! Explore the neutral timeline: `cargo run -p mb-tui --example devkit_trace_view_playground`.

use mb_tui::devkit::playground;
use mb_tui::devkit::{Scenario, ScenarioCatalog};
use mb_tui::widget::trace_view::{
    GroupId, ItemId, TraceData, TraceGroup, TraceItem, TraceTimeUnit, TraceTiming, TraceTrack,
    TraceView, TrackId, render_trace_view,
};

/// Reuse the shared terminal lifecycle with wholly owned, domain-neutral fixtures.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut catalog = ScenarioCatalog::new_interactive(render_trace_view, TraceView::handle_key);
    let mut state = fixture()?;
    state.select_item(ItemId(0));
    catalog.add(Scenario {
        name: "concurrent activity",
        description: "j/k rows; Tab items; h/l pan; +/- zoom; Space group; Enter details",
        state,
        inputs: vec![],
    });
    playground::run(&catalog)
}

/// Include overlapping work, equal-time observations, and visibly incomplete timing.
fn fixture() -> Result<TraceView, mb_tui::widget::trace_view::TraceError> {
    let timings = [
        ("prepare", TraceTiming::Interval { start: 0, end: 40 }),
        ("background", TraceTiming::Interval { start: 15, end: 70 }),
        ("finish", TraceTiming::Interval { start: 45, end: 90 }),
        ("checkpoint", TraceTiming::Instant(35)),
        ("same time", TraceTiming::Instant(35)),
        ("start absent", TraceTiming::MissingStart { end: 55 }),
        ("end absent", TraceTiming::MissingEnd { start: 75 }),
        (
            "clock regression",
            TraceTiming::Interval { start: 80, end: 65 },
        ),
        ("time unavailable", TraceTiming::Untimed),
    ];
    TraceView::new(TraceData {
        time_unit: TraceTimeUnit::Milliseconds,
        groups: vec![TraceGroup {
            id: GroupId(0),
            label: "example".into(),
        }],
        tracks: vec![TraceTrack {
            id: TrackId(0),
            group: Some(GroupId(0)),
            label: "worker".into(),
            items: timings
                .into_iter()
                .enumerate()
                .map(|(index, (label, timing))| TraceItem {
                    id: ItemId(index as u64),
                    label: label.into(),
                    timing,
                    details: vec![("source".into(), "owned fixture".into())],
                })
                .collect(),
        }],
    })
}

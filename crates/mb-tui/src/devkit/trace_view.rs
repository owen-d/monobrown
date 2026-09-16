//! Representative trace scenarios shared by snapshots and the playground.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Scenario, ScenarioCatalog};
use crate::widget::trace_view::{
    CategoryId, GroupId, ItemId, TraceCategory, TraceData, TraceGroup, TraceItem, TraceTimeUnit,
    TraceTiming, TraceTrack, TraceView, TraceVisualRole, TrackId, render_trace_view,
};

const NEUTRAL: CategoryId = CategoryId(0);
const SCHEDULED: CategoryId = CategoryId(1);
const SUCCESS: CategoryId = CategoryId(2);
const WARNING: CategoryId = CategoryId(3);
const FAILURE: CategoryId = CategoryId(4);

/// Build the interactive catalog used by both human inspection and golden tests.
pub fn trace_view_interactive_catalog() -> ScenarioCatalog<TraceView> {
    let mut catalog = ScenarioCatalog::new_interactive(render_trace_view, TraceView::handle_key);
    catalog.add(Scenario {
        name: "semantic-overview",
        description: "Trace hierarchy and every conventional semantic role",
        state: overview(),
        inputs: vec![key(KeyCode::Down), key(KeyCode::Enter)],
    });
    catalog.add(Scenario {
        name: "uncertainty-and-density",
        description: "Missing endpoints, regression, and untimed evidence as distinct leaves",
        state: uncertainty(),
        inputs: vec![key(KeyCode::Down)],
    });
    catalog.add(Scenario {
        name: "navigation-focus-collapse",
        description: "Interaction-derived selection, focus, and animated collapse",
        state: overview(),
        inputs: vec![
            key(KeyCode::Down),
            key(KeyCode::Char('f')),
            key(KeyCode::Char('h')),
        ],
    });
    catalog.add(Scenario {
        name: "detail-inspector",
        description: "Selected trace timing and payload details",
        state: overview(),
        inputs: vec![key(KeyCode::Enter)],
    });
    catalog
}

/// Stable category vocabulary with familiar status colors supplied by the widget theme.
fn categories() -> Vec<TraceCategory> {
    [
        (NEUTRAL, "activity", TraceVisualRole::Neutral),
        (SCHEDULED, "scheduled", TraceVisualRole::Scheduled),
        (SUCCESS, "success", TraceVisualRole::Success),
        (WARNING, "warning", TraceVisualRole::Warning),
        (FAILURE, "failure", TraceVisualRole::Failure),
    ]
    .into_iter()
    .map(|(id, label, role)| TraceCategory {
        id,
        label: label.into(),
        role,
    })
    .collect()
}

/// Show semantic outcome and concurrency without coupling the fixture to MMUI.
fn overview() -> TraceView {
    let base = 1_789_508_908_185_750_000_i64;
    let mut view = TraceView::new(TraceData {
        time_unit: TraceTimeUnit::Nanoseconds,
        categories: categories(),
        groups: vec![TraceGroup {
            id: GroupId(0),
            label: "request 01a0a709".into(),
        }],
        tracks: vec![
            TraceTrack {
                id: TrackId(0),
                group: Some(GroupId(0)),
                label: "query".into(),
                details: vec![],
                items: vec![
                    item(0, NEUTRAL, "admitted", TraceTiming::Instant(base)),
                    item(
                        1,
                        SCHEDULED,
                        "planning",
                        TraceTiming::Interval {
                            start: base + 1_000_000,
                            end: base + 220_000_000,
                        },
                    ),
                    item(
                        2,
                        SUCCESS,
                        "completed",
                        TraceTiming::Instant(base + 690_000_000),
                    ),
                ],
            },
            TraceTrack {
                id: TrackId(1),
                group: Some(GroupId(0)),
                label: "stage 0".into(),
                details: vec![],
                items: vec![
                    item(
                        3,
                        SCHEDULED,
                        "work 0",
                        TraceTiming::Interval {
                            start: base + 40_000_000,
                            end: base + 190_000_000,
                        },
                    ),
                    item(
                        4,
                        SUCCESS,
                        "cached",
                        TraceTiming::Instant(base + 210_000_000),
                    ),
                    item(
                        5,
                        WARNING,
                        "cancelled",
                        TraceTiming::Instant(base + 260_000_000),
                    ),
                    item(
                        6,
                        FAILURE,
                        "failed",
                        TraceTiming::Instant(base + 300_000_000),
                    ),
                ],
            },
        ],
    })
    .expect("devkit trace fixture is valid");
    view.select_item(ItemId(1));
    settle(&mut view);
    view
}

/// Exercise partial and invalid timing without hiding observations.
fn uncertainty() -> TraceView {
    let mut view = TraceView::new(TraceData {
        time_unit: TraceTimeUnit::Milliseconds,
        categories: categories(),
        groups: vec![TraceGroup {
            id: GroupId(0),
            label: "partial capture".into(),
        }],
        tracks: vec![TraceTrack {
            id: TrackId(0),
            group: Some(GroupId(0)),
            label: "evidence".into(),
            details: vec![],
            items: vec![
                item(0, NEUTRAL, "observed", TraceTiming::Instant(20)),
                item(1, FAILURE, "failed", TraceTiming::Instant(20)),
                item(
                    2,
                    WARNING,
                    "missing start",
                    TraceTiming::MissingStart { end: 40 },
                ),
                item(
                    3,
                    WARNING,
                    "missing end",
                    TraceTiming::MissingEnd { start: 60 },
                ),
                item(
                    4,
                    FAILURE,
                    "clock regression",
                    TraceTiming::Interval { start: 80, end: 50 },
                ),
                item(5, WARNING, "partial coverage", TraceTiming::Untimed),
            ],
        }],
    })
    .expect("devkit trace fixture is valid");
    view.select_item(ItemId(0));
    settle(&mut view);
    view
}

/// Stabilize initial fixtures; interaction-derived snapshots retain transitions.
fn settle(view: &mut TraceView) {
    for _ in 0..96 {
        view.tick(Duration::from_millis(16));
    }
}

/// Construct one selectable item with detail content for the expanded pane.
fn item(id: u64, category: CategoryId, label: &str, timing: TraceTiming) -> TraceItem {
    TraceItem {
        id: ItemId(id),
        category,
        label: label.into(),
        timing,
        details: vec![
            ("source".into(), "owned fixture".into()),
            ("payload".into(), r#"{"ok":true,"attempt":2}"#.into()),
        ],
    }
}

/// Construct an ordinary key press for an interaction-derived scenario.
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

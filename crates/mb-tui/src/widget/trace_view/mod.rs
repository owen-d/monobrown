//! Hierarchical trace events using the established flame-graph presentation.
//!
//! ```text
//! supplied display facts -> checked trace hierarchy
//! keys                   -> shared flame-graph navigation + animation
//!                                      -> labels + semantic bars
//! ```
//!
//! Callers own correlation and time meaning. This widget never pairs events or
//! invents missing endpoints. Recorded intervals contribute proportional bar
//! weight; individual observations remain distinct labeled leaves.
//!
//! Controls match the flame graph: j/k select rows, h/l collapse/expand, f/F
//! focus/unfocus, u/r undo/redo, and Enter toggles the semantic legend.
//!
//! ```rust
//! use mb_tui::widget::trace_view::{
//!     CategoryId, ItemId, TraceCategory, TraceData, TraceItem, TraceTimeUnit, TraceTiming,
//!     TraceTrack, TraceView, TraceVisualRole, TrackId,
//! };
//! let view = TraceView::new(TraceData {
//!     time_unit: TraceTimeUnit::Milliseconds,
//!     categories: vec![TraceCategory {
//!         id: CategoryId(0), label: "activity".into(), role: TraceVisualRole::Neutral,
//!     }],
//!     groups: vec![],
//!     tracks: vec![TraceTrack {
//!         id: TrackId(0), group: None, label: "worker".into(),
//!         items: vec![TraceItem {
//!             id: ItemId(0), category: CategoryId(0), label: "work".into(),
//!             timing: TraceTiming::Interval { start: 100, end: 120 },
//!             details: vec![("source".into(), "fixture".into())],
//!         }],
//!     }],
//! })?;
//! assert_eq!(view.selected_item().unwrap().label, "work");
//! # Ok::<(), mb_tui::widget::trace_view::TraceError>(())
//! ```

mod data;
mod layout;
mod render;
mod state;

pub use data::{
    CategoryId, GroupId, ItemId, TraceCategory, TraceData, TraceError, TraceGroup, TraceItem,
    TraceTimeUnit, TraceTiming, TraceTrack, TraceVisualRole, TrackId,
};
pub use render::{render_trace_view, render_trace_view_mut};
pub use state::TraceView;

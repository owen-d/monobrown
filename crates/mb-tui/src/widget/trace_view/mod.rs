//! Time-scaled tracks with explicit observations and concurrent intervals.
//!
//! ```text
//! supplied display facts -> checked data + fixed overlap lanes
//! keys                   -> selection + time window + collapsed groups
//!                                      -> terminal cells and details
//! ```
//!
//! Callers own correlation and time meaning. This widget never pairs events,
//! invents missing endpoints, or infers elapsed work. All coordinates use the
//! dataset's explicit unit; no floating-point timestamps or clock reads occur.
//!
//! Controls: j/k or Up/Down select rows; Tab/BackTab select items within a lane;
//! h/l or Left/Right pan; +/- zoom; 0 fits all recorded times; Space collapses
//! the selected group; Enter toggles details. Modified and release keys pass
//! through to the caller. The caller also owns terminal lifecycle and exit keys.
//!
//! ```rust
//! use mb_tui::widget::trace_view::{
//!     ItemId, TraceData, TraceItem, TraceTimeUnit, TraceTiming, TraceTrack,
//!     TraceView, TrackId,
//! };
//! let view = TraceView::new(TraceData {
//!     time_unit: TraceTimeUnit::Milliseconds,
//!     groups: vec![],
//!     tracks: vec![TraceTrack {
//!         id: TrackId(0), group: None, label: "worker".into(),
//!         items: vec![TraceItem {
//!             id: ItemId(0), label: "work".into(),
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
    GroupId, ItemId, TraceData, TraceError, TraceGroup, TraceItem, TraceTimeUnit, TraceTiming,
    TraceTrack, TrackId,
};
pub use render::{render_trace_view, render_trace_view_mut};
pub use state::TraceView;

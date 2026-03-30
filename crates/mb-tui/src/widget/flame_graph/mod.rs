//! Horizontal flame graph widget with expand/collapse navigation.
//!
//! The flame graph displays a tree of spans as stacked horizontal bars.
//! Each bar is segmented by cost type (e.g. cpu, io, mem) with proportional
//! widths and distinct colors. Navigation uses vim-style keys:
//!
//! - **j/k** or **Up/Down**: move cursor between visible rows
//! - **l** or **Right**: expand children or descend into first child
//! - **h** or **Left**: collapse children or ascend to parent
//! - **Enter**: toggle cost legend for the selected span

mod data;
mod layout;
mod render;
mod state;

pub use data::{CostBreakdown, CostType, SpanId, SpanNode, SpanNodeBuilder};
pub use layout::{FlameRow, RowKind};
pub use render::BarStyle;
pub use state::FlameGraph;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Render function matching the `fn(&S, Rect, &mut Buffer)` signature for playgrounds.
pub fn render_flame_graph(state: &FlameGraph, area: Rect, buf: &mut Buffer) {
    render::render(state, area, buf);
}

/// Render with mutable state: updates viewport height for scroll math, then renders.
pub fn render_flame_graph_mut(state: &mut FlameGraph, area: Rect, buf: &mut Buffer) {
    render::render_mut(state, area, buf);
}

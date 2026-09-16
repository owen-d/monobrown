//! Trace rendering delegated to the established flame-graph presentation.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::state::TraceView;
use crate::render::{Constraints, LayoutRenderable, Size};
use crate::widget::flame_graph::{render_flame_graph, render_flame_graph_mut};

impl LayoutRenderable for TraceView {
    fn measure(&self, constraints: Constraints) -> Size {
        self.graph().measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_trace_view(self, area, buf);
    }
}

/// Render through the same hierarchy, labels, bars, colors, and focus grammar
/// used by Descendit's flame graph.
pub fn render_trace_view(state: &TraceView, area: Rect, buf: &mut Buffer) {
    render_flame_graph(state.graph(), area, buf);
}

/// Update viewport state and render through the shared flame-graph engine.
pub fn render_trace_view_mut(state: &mut TraceView, area: Rect, buf: &mut Buffer) {
    render_flame_graph_mut(state.graph_mut(), area, buf);
}

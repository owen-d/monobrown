//! Tree rendering pipeline: flatten, build renderables, and render via pager.
//!
//! Contains [`LinesRenderable`], [`build_renderables`], [`compute_path_relations`],
//! [`ancestor_trail_relations`], and [`render_tree_pane`] — the shared rendering
//! pipeline used by the events, transitions, inspector, and detail panes.

use std::collections::HashSet;
use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Widget},
};

use crate::render::line_utils::format_duration_compact;
use crate::render::{Constraints, LayoutPagerView, LayoutRenderable, Size};

use super::state::TreeState;
use super::{
    FlatNode, NodeId, PathRelation, RenderContext, TreeNode, apply_cursor_style, flatten_visible,
    prepend_tree_prefix, tree_prefix_width,
};

// ---------------------------------------------------------------------------
// LinesRenderable -- owned lines wrapper for pager
// ---------------------------------------------------------------------------

/// Owned wrapper around `Vec<Line<'static>>` implementing [`LayoutRenderable`].
pub struct LinesRenderable {
    pub lines: Vec<Line<'static>>,
}

impl LayoutRenderable for LinesRenderable {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints
            .constrain(Size::new(self.max_line_width(), 0))
            .width;
        let height = self.lines.len().min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let paragraph = Paragraph::new(self.lines.clone());
        Widget::render(paragraph, area, buf);
    }
}

impl LinesRenderable {
    fn max_line_width(&self) -> u16 {
        self.lines
            .iter()
            .map(|line| line.width().min(u16::MAX as usize) as u16)
            .max()
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// render_tree_pane
// ---------------------------------------------------------------------------

/// Shared pipeline: flatten a tree, build renderables with tree prefixes,
/// and render through a pager.
///
/// Used by both the events pane (`render_chat_view`) and the transitions
/// pane (`render_transitions`). Handles cursor styling, auto-scroll, and
/// tree caching.
pub fn render_tree_pane(
    tree: Vec<TreeNode>,
    state: &mut TreeState,
    focused: bool,
    inner: Rect,
    generation: u64,
    buf: &mut Buffer,
    workflow_start_time: Option<Instant>,
) {
    // Flatten once from the local tree, build renderables, then store tree.
    // The block ensures flat_nodes (which borrows tree) is dropped before
    // we move tree into state.
    let (renderables, cursor) = {
        let expanded = state.expanded().clone();
        let flat_nodes = flatten_visible(&tree, &expanded);
        state.refresh_cache(&flat_nodes);
        let cursor = state.cursor();
        let renderables = build_renderables(
            &flat_nodes,
            cursor,
            focused,
            inner.width,
            &expanded,
            workflow_start_time,
        );
        (renderables, cursor)
    };

    // Store tree in state for detail viewer access between frames.
    state.set_tree(tree, generation);

    // Cache viewport height for half-page scroll calculations.
    state.set_viewport_height(inner.height);

    // Auto-scroll: set offset to MAX so the pager clamps to bottom.
    if state.auto_scroll() {
        state.set_scroll_offset(usize::MAX);
    }

    let mut pager = LayoutPagerView::new(
        renderables.into_iter().map(Into::into).collect(),
        state.scroll_offset(),
    );

    // Ensure the cursored node is visible.
    pager.scroll_chunk_into_view(cursor);

    // Render the pager into the inner area.
    pager.render(inner, buf);

    // Copy scroll offset back to state for persistence across frames.
    state.set_scroll_offset(pager.scroll_offset);
}

// ---------------------------------------------------------------------------
// compute_path_relations — cursor path membership for each flat node
// ---------------------------------------------------------------------------

/// Compute the [`PathRelation`] of every flat node relative to the cursor.
///
/// - `Direct`: the cursor node itself and its ancestors (depth > 0).
/// - `OnRoute`: earlier siblings of a Direct node (cursor path passes
///   through their column).
/// - `Outside`: all other nodes.
pub fn compute_path_relations(
    flat_nodes: &[FlatNode],
    cursor: usize,
    focused: bool,
) -> Vec<PathRelation> {
    let mut relations = vec![PathRelation::Outside; flat_nodes.len()];

    if !focused || flat_nodes.is_empty() {
        return relations;
    }

    // Build direct_set: walk from cursor upward via parent_index.
    // Include only indices where depth > 0 (roots have no connector to highlight).
    let mut direct_set = Vec::new();
    let mut current = cursor;
    loop {
        if flat_nodes[current].depth > 0 {
            direct_set.push(current);
        }
        match flat_nodes[current].parent_index {
            Some(p) => current = p,
            None => break,
        }
    }

    // Mark Direct.
    for &d in &direct_set {
        relations[d] = PathRelation::Direct;
    }

    // Mark OnRoute: earlier siblings of each Direct node.
    for &d in &direct_set {
        for j in 0..d {
            if flat_nodes[j].parent_index == flat_nodes[d].parent_index
                && flat_nodes[j].depth == flat_nodes[d].depth
                && relations[j] == PathRelation::Outside
            {
                relations[j] = PathRelation::OnRoute;
            }
        }
    }

    relations
}

// ---------------------------------------------------------------------------
// build_renderables — flat nodes to renderable lines
// ---------------------------------------------------------------------------

/// Compute the [`PathRelation`] of each ancestor in a node's parent trail.
///
/// Walks up the parent chain, collecting the relation of each ancestor at
/// depth > 0. Returns in parent-trail order (outermost first).
pub fn ancestor_trail_relations(
    flat_nodes: &[FlatNode],
    relations: &[PathRelation],
    node_idx: usize,
) -> Vec<PathRelation> {
    let depth = flat_nodes[node_idx].depth;
    if depth <= 1 {
        return vec![];
    }
    let mut result = Vec::with_capacity(depth - 1);
    let mut current = flat_nodes[node_idx].parent_index;
    while let Some(idx) = current {
        if flat_nodes[idx].depth > 0 {
            result.push(relations[idx]);
        }
        current = flat_nodes[idx].parent_index;
    }
    // Collected innermost-first; reverse to match parent_trail order.
    result.reverse();
    result
}

/// Convert flattened visible nodes into renderables with tree prefixes and
/// cursor path highlighting.
pub fn build_renderables(
    flat_nodes: &[FlatNode],
    cursor: usize,
    focused: bool,
    inner_width: u16,
    expanded_set: &HashSet<NodeId>,
    workflow_start_time: Option<Instant>,
) -> Vec<Box<dyn LayoutRenderable>> {
    let mut renderables: Vec<Box<dyn LayoutRenderable>> = Vec::with_capacity(flat_nodes.len());
    let relations = compute_path_relations(flat_nodes, cursor, focused);

    for (i, flat) in flat_nodes.iter().enumerate() {
        let prefix_w = tree_prefix_width(flat.depth);
        let content_w = (inner_width as usize).saturating_sub(prefix_w);

        let is_expanded = expanded_set.contains(&flat.node.id);
        let elapsed = match (flat.node.content.created_at(), workflow_start_time) {
            (Some(created), Some(start)) => {
                let duration = created.saturating_duration_since(start);
                Some(format_duration_compact(duration))
            }
            _ => None,
        };
        let ctx = RenderContext {
            width: content_w.max(1) as u16,
            expanded: is_expanded,
            elapsed,
        };

        let mut lines = flat.node.content.render(&ctx);

        // Apply cursor content styling before prepending tree prefix.
        if focused && i == cursor {
            apply_cursor_style(&mut lines, flat.node.content.cursor_style());
        }

        // Compute ancestor relations for parent-trail highlighting.
        let ancestor_rels = ancestor_trail_relations(flat_nodes, &relations, i);

        // Prepend tree connector characters with cursor path highlighting.
        prepend_tree_prefix(
            &mut lines,
            flat.depth,
            flat.is_last_sibling,
            &flat.parent_trail,
            &ancestor_rels,
            relations[i],
        );

        renderables.push(Box::new(LinesRenderable { lines }));
    }

    renderables
}

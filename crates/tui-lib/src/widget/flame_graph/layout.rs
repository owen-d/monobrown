use std::collections::HashMap;

use super::data::{SpanId, SpanNode};
use super::state::ExpandAnimation;

const ANIM_VISIBLE_THRESHOLD: f64 = 0.001;
pub(crate) const MAX_LABEL_ZONE: u16 = 18;
pub(crate) const MIN_LABEL_ZONE: u16 = 6;
pub(crate) const MIN_BAR_ZONE: u16 = 4;

/// Width-derived row layout for the flame graph.
///
/// Wide terminals get the full two-zone layout with a dedicated bar region.
/// Narrow terminals collapse the bar zone and use text summaries instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowLayout {
    pub label_zone: u16,
    pub gap: u16,
    pub bar_zone: u16,
}

impl RowLayout {
    pub const MIN_LABEL_ZONE: u16 = MIN_LABEL_ZONE;
    pub const MAX_LABEL_ZONE: u16 = MAX_LABEL_ZONE;
    pub const MIN_BAR_ZONE: u16 = MIN_BAR_ZONE;
    pub const MIN_DETAILED_WIDTH: u16 = MIN_LABEL_ZONE + 1 + MIN_BAR_ZONE;

    pub fn for_width(total_width: u16) -> Self {
        if total_width == 0 {
            return Self {
                label_zone: 0,
                gap: 0,
                bar_zone: 0,
            };
        }

        let desired_label_zone = (total_width / 3).clamp(MIN_LABEL_ZONE, MAX_LABEL_ZONE);
        let max_label_zone = total_width.saturating_sub(1).saturating_sub(MIN_BAR_ZONE);

        if max_label_zone < MIN_LABEL_ZONE {
            return Self {
                label_zone: total_width,
                gap: 0,
                bar_zone: 0,
            };
        }

        let label_zone = desired_label_zone.min(max_label_zone);
        let bar_zone = total_width.saturating_sub(1).saturating_sub(label_zone);

        Self {
            label_zone,
            gap: 1,
            bar_zone,
        }
    }

    pub fn bar_start(self) -> u16 {
        self.label_zone + self.gap
    }

    pub fn shows_bars(self) -> bool {
        self.bar_zone >= MIN_BAR_ZONE
    }
}

/// Width of the bar zone given the total terminal width.
pub fn bar_zone_width(total_width: u16) -> u16 {
    RowLayout::for_width(total_width).bar_zone
}

/// What kind of row this is in the flattened layout.
#[derive(Clone, Debug)]
pub enum RowKind {
    /// A span bar.
    Span {
        span_id: SpanId,
        depth: u16,
        bar_width: u16,
    },
    /// A cost-type legend line for a span.
    Legend { span_id: SpanId },
}

/// One row in the flattened, visible flame graph layout.
#[derive(Clone, Debug)]
pub struct FlameRow {
    pub kind: RowKind,
}

/// Shared context for the recursive flattening walk.
struct FlattenCtx<'a> {
    root: &'a SpanNode,
    path: &'a [SpanId],
    animations: &'a HashMap<SpanId, ExpandAnimation>,
    legend_for: Option<SpanId>,
    focus: Option<SpanId>,
    rows: Vec<FlameRow>,
}

/// Flatten the span tree into visible rows for rendering.
///
/// `total_width` determines bar sizing. Row ordering is stable regardless of
/// width, so callers doing navigation can pass any value.
///
/// A node's children are shown if the node is on the `path` and is not the
/// last element (the path leaf), OR if the node has an active animation
/// (collapse in progress).
///
/// When `focus` is set, the focused node becomes the visual root at depth 0
/// with full bar width. Its siblings are appended below the subtree.
pub fn flatten_visible_rows(
    root: &SpanNode,
    path: &[SpanId],
    animations: &HashMap<SpanId, ExpandAnimation>,
    legend_for: Option<SpanId>,
    focus: Option<SpanId>,
    total_width: u16,
) -> Vec<FlameRow> {
    let mut ctx = FlattenCtx {
        root,
        path,
        animations,
        legend_for,
        focus,
        rows: Vec::new(),
    };
    let root_bar_width = bar_zone_width(total_width);

    if let Some(focus_id) = focus {
        flatten_focused(&mut ctx, focus_id, root_bar_width);
    } else {
        flatten_node(root, 0, root_bar_width, &mut ctx);
    }
    ctx.rows
}

/// Flatten in focus mode: only the focus node's subtree, no siblings.
fn flatten_focused(ctx: &mut FlattenCtx<'_>, focus_id: SpanId, bar_width: u16) {
    use super::state::find_node;

    let Some(focus_node) = find_node(ctx.root, focus_id) else {
        // Focus node not found; fall back to normal flattening.
        flatten_node(ctx.root, 0, bar_width, ctx);
        return;
    };

    // Render the focus subtree at depth 0 with full bar width.
    flatten_node(focus_node, 0, bar_width, ctx);
}

fn flatten_node(node: &SpanNode, depth: u16, bar_width: u16, ctx: &mut FlattenCtx<'_>) {
    ctx.rows.push(FlameRow {
        kind: RowKind::Span {
            span_id: node.id,
            depth,
            bar_width,
        },
    });

    // Legend row if this node is selected.
    if ctx.legend_for == Some(node.id) {
        ctx.rows.push(FlameRow {
            kind: RowKind::Legend { span_id: node.id },
        });
    }

    // Recurse into children if this node is expanded.
    // A node is expanded if it's on the path but not the leaf, OR if it has
    // an active animation (collapse still in progress), OR if it is the
    // focus node (always expanded in focus mode).
    let on_path_expanded = ctx.path.contains(&node.id) && ctx.path.last() != Some(&node.id);
    let is_focus_root = ctx.focus == Some(node.id);
    let animating = ctx.animations.contains_key(&node.id);
    let show_children = on_path_expanded || is_focus_root || animating;

    if !show_children || node.children.is_empty() {
        return;
    }

    let anim_scale = ctx.animations.get(&node.id).map_or(1.0, |a| a.value);
    let parent_total = node.costs.total();

    // Path-first ordering: the path child renders first among its siblings,
    // keeping the visual path going straight down. Only applies to non-leaf
    // path nodes (expanded ancestors), so j/k sibling browsing stays stable.
    let path_child_idx = node
        .children
        .iter()
        .position(|c| ctx.path.contains(&c.id) && ctx.path.last() != Some(&c.id));

    let iter_order: Vec<usize> = if let Some(idx) = path_child_idx {
        std::iter::once(idx)
            .chain((0..node.children.len()).filter(|&i| i != idx))
            .collect()
    } else {
        (0..node.children.len()).collect()
    };

    for i in iter_order {
        let child = &node.children[i];
        let fraction = if parent_total > 0.0 {
            child.costs.total() / parent_total
        } else {
            0.0
        };
        let child_width = compute_child_width(bar_width, fraction, anim_scale);
        flatten_node(child, depth + 1, child_width, ctx);
    }
}

/// Compute child bar width from parent bar width, cost fraction, and animation scale.
fn compute_child_width(parent_bar_width: u16, fraction: f64, anim_scale: f64) -> u16 {
    if parent_bar_width == 0 || fraction <= 0.0 {
        return 0;
    }
    let raw = (parent_bar_width as f64) * fraction * anim_scale;
    let rounded = raw.floor() as u16;
    // Ensure at least 1 pixel when the animation is visible.
    if anim_scale > ANIM_VISIBLE_THRESHOLD {
        rounded.max(1)
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::flame_graph::data::SpanNodeBuilder;

    #[test]
    fn wide_layout_preserves_classic_two_zone_shape() {
        let layout = RowLayout::for_width(80);
        assert_eq!(layout.label_zone, 18);
        assert_eq!(layout.gap, 1);
        assert_eq!(layout.bar_start(), 19);
        assert_eq!(layout.bar_zone, 61);
        assert!(layout.shows_bars());
    }

    #[test]
    fn narrow_layout_collapses_bar_zone() {
        let layout = RowLayout::for_width(10);
        assert_eq!(layout.gap, 0);
        assert_eq!(layout.bar_zone, 0);
        assert_eq!(layout.label_zone, 10);
        assert!(!layout.shows_bars());
    }

    #[test]
    fn single_root_produces_one_row() {
        let mut b = SpanNodeBuilder::new();
        let root = b.leaf("root", vec![10.0]);
        let rows = flatten_visible_rows(&root, &[root.id], &HashMap::new(), None, None, 80);
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].kind, RowKind::Span { bar_width: 61, .. }));
    }

    #[test]
    fn unexpanded_parent_shows_only_root() {
        let mut b = SpanNodeBuilder::new();
        let child = b.leaf("child", vec![5.0]);
        let root = b.span("root", vec![10.0], vec![child]);
        // path = [root.id] means root is the leaf, so it's not expanded.
        let rows = flatten_visible_rows(&root, &[root.id], &HashMap::new(), None, None, 80);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn expanded_parent_shows_children() {
        let mut b = SpanNodeBuilder::new();
        let c1 = b.leaf("big", vec![7.0]);
        let c2 = b.leaf("small", vec![3.0]);
        let root = b.span("root", vec![10.0], vec![c1.clone(), c2]);

        // path = [root.id, c1.id] means root is expanded, c1 is the leaf.
        let path = vec![root.id, c1.id];
        let rows = flatten_visible_rows(&root, &path, &HashMap::new(), None, None, 80);
        // Root + 2 children.
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn legend_row_inserted_after_selected_span() {
        let mut b = SpanNodeBuilder::new();
        let root = b.leaf("root", vec![10.0]);
        let legend_for = Some(root.id);
        let rows = flatten_visible_rows(&root, &[root.id], &HashMap::new(), legend_for, None, 80);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[1].kind, RowKind::Legend { .. }));
    }

    #[test]
    fn child_bar_narrower_than_parent() {
        let mut b = SpanNodeBuilder::new();
        let child = b.leaf("child", vec![5.0]);
        let root = b.span("root", vec![10.0], vec![child.clone()]);

        let path = vec![root.id, child.id];
        let rows = flatten_visible_rows(&root, &path, &HashMap::new(), None, None, 80);

        assert_eq!(rows.len(), 2);
        let root_width = match rows[0].kind {
            RowKind::Span { bar_width, .. } => bar_width,
            _ => panic!("expected Span row"),
        };
        let child_width = match rows[1].kind {
            RowKind::Span { bar_width, .. } => bar_width,
            _ => panic!("expected Span row"),
        };
        // child cost (5) / parent cost (10) = 0.5, so child is half of parent.
        assert_eq!(child_width, root_width / 2);
    }

    #[test]
    fn expanded_children_remain_visible_when_bar_zone_disappears() {
        let mut b = SpanNodeBuilder::new();
        let a = b.leaf("a", vec![7.0]);
        let c = b.leaf("b", vec![3.0]);
        let root = b.span("root", vec![10.0], vec![a.clone(), c]);
        let rows = flatten_visible_rows(&root, &[root.id, a.id], &HashMap::new(), None, None, 10);
        assert_eq!(
            rows.len(),
            3,
            "children should remain visible in summary mode"
        );
        assert!(matches!(rows[1].kind, RowKind::Span { bar_width: 0, .. }));
        assert!(matches!(rows[2].kind, RowKind::Span { bar_width: 0, .. }));
    }

    #[test]
    fn layout_is_deterministic() {
        let run = || {
            let mut b = SpanNodeBuilder::new();
            let c1 = b.leaf("a", vec![3.0, 2.0]);
            let c2 = b.leaf("b", vec![1.0, 4.0]);
            let root = b.span("root", vec![4.0, 6.0], vec![c1.clone(), c2]);
            let path = vec![root.id, c1.id];
            let rows = flatten_visible_rows(&root, &path, &HashMap::new(), None, None, 100);
            rows.iter()
                .map(|r| format!("{:?}", r.kind))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}

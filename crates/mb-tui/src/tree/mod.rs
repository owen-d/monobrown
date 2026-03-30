//! Core tree types shared by the events, transitions, and inspector panes.
//!
//! Defines [`NodeContent`] (the rendering trait), [`TreeNode`] (the tree
//! structure), [`FlatNode`] (flattened visible node), and the flatten/render
//! pipeline. The tree infrastructure handles indentation, tree-style connector
//! lines (`├──`, `└──`, `│`), and cursor path highlighting — node
//! implementations only produce their own content lines.

pub mod render;
pub mod state;

use std::collections::HashSet;

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::theme;

// ---------------------------------------------------------------------------
// NodeId
// ---------------------------------------------------------------------------

/// Stable identity for a tree node.
///
/// Assigned monotonically during tree construction. Expansion state
/// is keyed by NodeId.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

// ---------------------------------------------------------------------------
// RenderContext
// ---------------------------------------------------------------------------

/// Context passed to [`NodeContent::render`] for rendering decisions.
pub struct RenderContext {
    /// Available content width (after tree prefixes are subtracted).
    pub width: u16,
    /// Whether this node is currently expanded (consumed by node renderers).
    pub expanded: bool,
    /// Compact elapsed time string (e.g. "3s", "1m 02s"), if available
    /// (consumed by node renderers).
    pub elapsed: Option<String>,
}

// ---------------------------------------------------------------------------
// Cursor path highlighting types
// ---------------------------------------------------------------------------

// Cursor highlight color is defined in theme::cursor().

/// Relationship of a flat node to the cursor path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRelation {
    /// This node is the cursor or one of its ancestors (depth > 0).
    Direct,
    /// This node is an earlier sibling of a Direct node (cursor path passes
    /// through this column).
    OnRoute,
    /// No relationship to the cursor path.
    Outside,
}

/// Style of a prefix segment span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStyle {
    Dim,
    Highlight,
}

/// A single styled segment of a tree-line prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixSegment {
    pub text: &'static str,
    pub style: SegmentStyle,
}

impl PrefixSegment {
    pub fn to_span(&self) -> Span<'static> {
        let color = match self.style {
            SegmentStyle::Dim => theme::dim(),
            SegmentStyle::Highlight => theme::cursor(),
        };
        Span::styled(self.text, Style::default().fg(color))
    }
}

/// Cursor styling to apply to the selected node's content spans.
pub enum CursorStyle {
    /// Amber foreground (for tool nodes).
    Highlight,
    /// Underline (for leaf text nodes).
    Underline,
}

// ---------------------------------------------------------------------------
// NodeContent trait
// ---------------------------------------------------------------------------

/// Content of a tree node. Knows how to render itself.
///
/// The tree infrastructure handles indentation, tree lines, and cursor
/// indicators. Implementations only produce their own content lines.
pub trait NodeContent: std::fmt::Debug + Send + Sync {
    /// Render this node's own line(s). No tree-line prefixes — those are
    /// added by the tree renderer. Just the content.
    fn render(&self, ctx: &RenderContext) -> Vec<Line<'static>>;

    /// Full untruncated text for the detail viewer (Enter key).
    fn full_text(&self) -> Option<String> {
        None
    }

    /// How to style the content when this node is the cursor.
    fn cursor_style(&self) -> CursorStyle {
        CursorStyle::Underline
    }

    /// If this node represents an agent lifecycle event, return the agent ID.
    fn agent_id(&self) -> Option<&str> {
        None
    }

    /// Human-readable source label for detail viewer attribution (e.g. "orch", "designer").
    fn source_label(&self) -> Option<&str> {
        None
    }

    /// Timestamp string for detail viewer attribution.
    fn timestamp(&self) -> Option<&str> {
        None
    }

    /// Instant when the underlying event was created, for elapsed time display.
    fn created_at(&self) -> Option<std::time::Instant> {
        None
    }

    /// Build a tree for the detail pane (Enter key on this node).
    ///
    /// Default: wraps `full_text()` in a single `DetailTextNode` leaf.
    /// Override for richer detail pane trees.
    fn detail_tree(&self) -> Option<Vec<TreeNode>> {
        let text = self.full_text()?;
        let mut allocator = NodeIdAllocator::new();
        let node = TreeNode {
            id: allocator.allocate(),
            content: Box::new(DetailTextNode { text }),
            children: vec![],
        };
        Some(vec![node])
    }
}

// ---------------------------------------------------------------------------
// DetailTextNode — simple text leaf for the detail pane
// ---------------------------------------------------------------------------

/// Leaf node for the detail pane that renders full text with word-wrapping.
///
/// Created by the default `NodeContent::detail_tree()` implementation.
/// Renders each line of the text as a separate `Line` (the tree/pager
/// pipeline handles viewport scrolling).
#[derive(Debug)]
pub struct DetailTextNode {
    pub text: String,
}

impl NodeContent for DetailTextNode {
    fn render(&self, ctx: &RenderContext) -> Vec<Line<'static>> {
        let max_w = ctx.width as usize;
        let mut lines = Vec::new();
        for paragraph in self.text.split('\n') {
            if paragraph.is_empty() {
                lines.push(Line::from(""));
                continue;
            }
            // Simple word-wrap within each paragraph.
            let mut current = String::new();
            for word in paragraph.split_whitespace() {
                if current.is_empty() {
                    current = word.to_string();
                } else if current.len() + 1 + word.len() <= max_w {
                    current.push(' ');
                    current.push_str(word);
                } else {
                    lines.push(Line::from(std::mem::take(&mut current)));
                    current = word.to_string();
                }
            }
            if !current.is_empty() {
                lines.push(Line::from(current));
            }
        }
        lines
    }

    fn full_text(&self) -> Option<String> {
        Some(self.text.clone())
    }

    fn cursor_style(&self) -> CursorStyle {
        CursorStyle::Underline
    }
}

// ---------------------------------------------------------------------------
// HighlightedCodeNode — syntax-highlighted code leaf for the detail pane
// ---------------------------------------------------------------------------

/// Leaf node that renders syntax-highlighted code (e.g. JSON schemas).
///
/// Unlike `DetailTextNode`, this preserves the original line breaks and
/// indentation — no word-wrapping. Uses `highlight_code` from the highlight
/// module for colorized output with a plain-text fallback.
#[derive(Debug)]
pub struct HighlightedCodeNode {
    /// Label shown before the code (e.g. "Schema").
    pub label: String,
    /// The raw source code.
    pub code: String,
    /// Language hint for syntax highlighting (e.g. "json").
    pub lang: String,
}

impl NodeContent for HighlightedCodeNode {
    fn render(&self, ctx: &RenderContext) -> Vec<Line<'static>> {
        use crate::highlight::highlight_code;

        let mut lines = Vec::new();

        // Try syntax highlighting; fall back to unstyled.
        if let Some(highlighted) = highlight_code(&self.code, &self.lang) {
            // First line: "Label:" then first code line on the next line.
            let dim = Style::default().fg(theme::dim());
            lines.push(Line::from(Span::styled(format!("{}:", self.label), dim)));
            for hl_line in highlighted {
                let spans: Vec<Span<'static>> = hl_line
                    .into_iter()
                    .map(|(t, s)| Span::styled(t, s))
                    .collect();
                lines.push(Line::from(spans));
            }
        } else {
            // Fallback: render as plain text, one line per source line.
            let dim = Style::default().fg(theme::dim());
            lines.push(Line::from(Span::styled(format!("{}:", self.label), dim)));
            for line in self.code.lines() {
                let max_chars = ctx.width as usize;
                let truncated = if line.chars().count() > max_chars {
                    let byte_end = line
                        .char_indices()
                        .nth(max_chars)
                        .map_or(line.len(), |(idx, _)| idx);
                    &line[..byte_end]
                } else {
                    line
                };
                lines.push(Line::from(truncated.to_string()));
            }
        }
        lines
    }

    fn full_text(&self) -> Option<String> {
        Some(format!("{}: {}", self.label, self.code))
    }

    fn cursor_style(&self) -> CursorStyle {
        CursorStyle::Underline
    }
}

// ---------------------------------------------------------------------------
// TreeNode
// ---------------------------------------------------------------------------

/// A node in the event tree.
pub struct TreeNode {
    pub id: NodeId,
    pub content: Box<dyn NodeContent>,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    /// Whether this node has no children.
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

impl std::fmt::Debug for TreeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeNode")
            .field("id", &self.id)
            .field("content", &self.content)
            .field("children_count", &self.children.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TreeSource — declarative tree structure for generic materialization
// ---------------------------------------------------------------------------

/// Describes the structure of a tree node without materializing it.
/// Domain types implement this to declare their content and children.
/// The `materialize()` function walks `TreeSource` implementations to
/// produce the concrete `TreeNode` trees used by the rendering pipeline.
pub trait TreeSource {
    /// The rendering content for this node.
    fn content(&self) -> Box<dyn NodeContent>;
    /// Child sources. Empty = leaf node (no expand indicator rendered).
    fn children(&self) -> Vec<Box<dyn TreeSource>>;
}

// ---------------------------------------------------------------------------
// NodeIdAllocator — monotonic allocator for NodeId values
// ---------------------------------------------------------------------------

/// Monotonic allocator for `NodeId` values. Replaces the scattered `&mut u64`
/// pattern in tree construction, making allocation explicit and assertable.
#[derive(Default)]
pub struct NodeIdAllocator {
    next: u64,
}

impl NodeIdAllocator {
    /// Create a new allocator starting from `NodeId(0)`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate the next `NodeId` in monotonic sequence.
    ///
    /// Panics in debug builds if the counter would overflow `u64::MAX`.
    pub fn allocate(&mut self) -> NodeId {
        debug_assert!(self.next < u64::MAX, "NodeId allocator overflow");
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    /// Return the total number of `NodeId`s allocated so far.
    pub fn total_allocated(&self) -> u64 {
        self.next
    }
}

// ---------------------------------------------------------------------------
// materialize — walk TreeSource to produce concrete TreeNode trees
// ---------------------------------------------------------------------------

/// Walk `TreeSource` implementations in depth-first pre-order to produce
/// a concrete `Vec<TreeNode>` with monotonically increasing `NodeId`s.
///
/// Parent IDs are always less than their children's IDs. This invariant
/// is required by `flatten_visible()` and the cursor navigation system.
pub fn materialize(
    sources: Vec<Box<dyn TreeSource>>,
    allocator: &mut NodeIdAllocator,
) -> Vec<TreeNode> {
    let before = allocator.total_allocated();
    let result = materialize_inner(sources, allocator, 0);
    let after = allocator.total_allocated();

    debug_assert_eq!(
        after - before,
        count_tree_nodes(&result),
        "allocator count ({}) must match materialized node count ({})",
        after - before,
        count_tree_nodes(&result),
    );
    debug_assert!(
        verify_preorder_ids(&result),
        "NodeIds must be monotonically increasing in depth-first pre-order",
    );

    result
}

/// Recursive helper for [`materialize`].
fn materialize_inner(
    sources: Vec<Box<dyn TreeSource>>,
    allocator: &mut NodeIdAllocator,
    depth: usize,
) -> Vec<TreeNode> {
    debug_assert!(
        depth <= MAX_TREE_DEPTH,
        "Tree depth {depth} exceeds MAX_TREE_DEPTH {MAX_TREE_DEPTH}",
    );

    sources
        .into_iter()
        .map(|source| {
            let id = allocator.allocate();
            let content = source.content();
            let child_sources = source.children();
            let children = materialize_inner(child_sources, allocator, depth + 1);
            TreeNode {
                id,
                content,
                children,
            }
        })
        .collect()
}

/// Count total nodes in a tree. Used in assertions to verify allocator
/// consistency with the materialized tree.
fn count_tree_nodes(tree: &[TreeNode]) -> u64 {
    count_tree_nodes_inner(tree, 0)
}

fn count_tree_nodes_inner(tree: &[TreeNode], depth: usize) -> u64 {
    debug_assert!(depth <= MAX_TREE_DEPTH);
    tree.iter()
        .map(|node| 1 + count_tree_nodes_inner(&node.children, depth + 1))
        .sum()
}

/// Verify that all NodeIds in the tree are monotonically increasing
/// in depth-first pre-order. Used as a debug assertion.
fn verify_preorder_ids(tree: &[TreeNode]) -> bool {
    let mut last_id = None;
    verify_preorder_inner(tree, &mut last_id, 0)
}

fn verify_preorder_inner(tree: &[TreeNode], last_id: &mut Option<u64>, depth: usize) -> bool {
    debug_assert!(depth <= MAX_TREE_DEPTH);
    for node in tree {
        if let Some(prev) = *last_id
            && node.id.0 <= prev
        {
            return false;
        }
        *last_id = Some(node.id.0);
        if !verify_preorder_inner(&node.children, last_id, depth + 1) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// FlatNode — flattened visible node for rendering and cursor mapping
// ---------------------------------------------------------------------------

/// A visible node in the flattened tree walk.
///
/// Produced by [`flatten_visible`] for rendering and cursor resolution.
pub struct FlatNode<'a> {
    /// Reference to the tree node.
    pub node: &'a TreeNode,
    /// Indentation depth (0 for roots).
    pub depth: usize,
    /// Whether this is the last sibling at its level (for └── vs ├──).
    pub is_last_sibling: bool,
    /// For each ancestor level, whether that ancestor was the last sibling.
    /// Used to decide between "│   " and "    " continuation lines.
    /// Length is `depth - 1` (roots contribute no tree lines).
    pub parent_trail: Vec<bool>,
    /// Flat index of the parent node, or None for roots.
    pub parent_index: Option<usize>,
}

// ---------------------------------------------------------------------------
// CachedFlatEntry — lightweight cache for navigation between frames
// ---------------------------------------------------------------------------

/// Lightweight cache entry for cursor/navigation resolution between frames.
///
/// Stored in [`TreeState`](super::tree_state::TreeState) so that navigation
/// methods (drill_in, drill_out) can resolve parent/child relationships
/// without access to the full tree.
#[derive(Debug, Clone)]
pub struct CachedFlatEntry {
    pub node_id: NodeId,
    pub parent_flat_index: Option<usize>,
    pub has_children: bool,
}

// ---------------------------------------------------------------------------
// flatten_visible — walk the tree producing visible nodes
// ---------------------------------------------------------------------------

/// Flatten the tree into a list of visible nodes.
///
/// A node is visible if all its ancestors are in the `expanded` set.
/// Roots are always visible. Children of expanded nodes are visible.
pub fn flatten_visible<'a>(roots: &'a [TreeNode], expanded: &HashSet<NodeId>) -> Vec<FlatNode<'a>> {
    let mut result = Vec::new();
    let root_count = roots.len();

    for (i, root) in roots.iter().enumerate() {
        let is_last = i == root_count - 1;
        flatten_node(root, 0, is_last, &[], None, expanded, &mut result);
    }

    debug_assert!(
        result.len() >= roots.len() || roots.is_empty(),
        "visible count ({}) must be >= root count ({})",
        result.len(),
        roots.len(),
    );
    result
}

/// Recursive helper for [`flatten_visible`].
fn flatten_node<'a>(
    node: &'a TreeNode,
    depth: usize,
    is_last_sibling: bool,
    parent_trail: &[bool],
    parent_index: Option<usize>,
    expanded: &HashSet<NodeId>,
    result: &mut Vec<FlatNode<'a>>,
) {
    debug_assert!(depth < MAX_TREE_DEPTH, "tree depth {depth} exceeds bound");
    let my_index = result.len();
    result.push(FlatNode {
        node,
        depth,
        is_last_sibling,
        parent_trail: parent_trail.to_vec(),
        parent_index,
    });

    if !expanded.contains(&node.id) || node.children.is_empty() {
        return;
    }

    // Build child trail. Root nodes (depth 0) don't contribute tree lines,
    // so we skip pushing their is_last_sibling.
    let child_trail = if depth == 0 {
        vec![]
    } else {
        let mut t = parent_trail.to_vec();
        t.push(is_last_sibling);
        t
    };

    let child_count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == child_count - 1;
        flatten_node(
            child,
            depth + 1,
            child_is_last,
            &child_trail,
            Some(my_index),
            expanded,
            result,
        );
    }
}

/// Build a lightweight cache from a flat node list for navigation.
pub fn build_flat_cache(flat_nodes: &[FlatNode<'_>]) -> Vec<CachedFlatEntry> {
    debug_assert!(
        flat_nodes
            .iter()
            .enumerate()
            .all(|(i, f)| { f.parent_index.is_none_or(|p| p < i) }),
        "parent indices must be less than child indices"
    );
    flat_nodes
        .iter()
        .map(|f| CachedFlatEntry {
            node_id: f.node.id,
            parent_flat_index: f.parent_index,
            has_children: !f.node.children.is_empty(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tree-line prefix generation
// ---------------------------------------------------------------------------

/// Maximum allowed tree depth. Trees deeper than this indicate a bug.
const MAX_TREE_DEPTH: usize = 8;

/// Width of tree connector characters per depth level: "├── " = 4 chars.
pub const TREE_INDENT_WIDTH: usize = 4;

/// Build the styled prefix segments for one line of a tree node.
///
/// Returns an empty vec for depth 0 (roots have no connector).
///
/// # Arguments
/// - `depth` — tree depth of the node (0 = root).
/// - `is_last_sibling` — whether this node is the last child of its parent.
/// - `parent_trail` — for each ancestor level (depth 1..depth-1), whether
///   that ancestor was the last sibling; controls `│` vs space columns.
/// - `ancestor_relations` — [`PathRelation`] for each ancestor in the parent
///   trail. When an ancestor is Direct or OnRoute, the `│` continuation in
///   that column is highlighted amber (split: `│` highlight + spaces dim) so
///   the cursor path remains visually connected through children.
/// - `line_idx` — which rendered line of the node this prefix is for (0 =
///   first, >0 = continuation).
/// - `relation` — this node's relationship to the cursor path.
pub fn build_prefix_segments(
    depth: usize,
    is_last_sibling: bool,
    parent_trail: &[bool],
    ancestor_relations: &[PathRelation],
    line_idx: usize,
    relation: PathRelation,
) -> Vec<PrefixSegment> {
    if depth == 0 {
        return vec![];
    }

    let mut segments = Vec::with_capacity(depth);

    // Parent-trail columns. When the ancestor at this column is on the cursor
    // path (Direct or OnRoute), the │ char is highlighted so the amber path
    // visually connects through children. Otherwise dim.
    for (i, &ancestor_is_last) in parent_trail.iter().enumerate() {
        let ancestor_rel = ancestor_relations
            .get(i)
            .copied()
            .unwrap_or(PathRelation::Outside);

        if ancestor_is_last {
            // No continuation line — always spaces.
            segments.push(PrefixSegment {
                text: "    ",
                style: SegmentStyle::Dim,
            });
        } else {
            match ancestor_rel {
                PathRelation::OnRoute => {
                    // Split: │ highlighted, spaces dim.
                    segments.push(PrefixSegment {
                        text: "│",
                        style: SegmentStyle::Highlight,
                    });
                    segments.push(PrefixSegment {
                        text: "   ",
                        style: SegmentStyle::Dim,
                    });
                }
                PathRelation::Direct | PathRelation::Outside => {
                    segments.push(PrefixSegment {
                        text: "│   ",
                        style: SegmentStyle::Dim,
                    });
                }
            }
        }
    }

    // Own connector: determined by line_idx and is_last_sibling.
    let connector: &'static str = if line_idx == 0 {
        if is_last_sibling {
            "└── "
        } else {
            "├── "
        }
    } else if is_last_sibling {
        "    "
    } else {
        "│   "
    };

    push_connector_segments(&mut segments, connector, relation);
    segments
}

/// Push the own-connector segment(s) for a node, styled by cursor path relation.
///
/// - `Outside` / `Direct`: single segment with uniform styling.
/// - `OnRoute`: split the junction character (highlighted) from the dash/space
///   tail (dim). Spaces-only connectors have no junction to split.
fn push_connector_segments(
    segments: &mut Vec<PrefixSegment>,
    connector: &'static str,
    relation: PathRelation,
) {
    match relation {
        PathRelation::Outside => {
            segments.push(PrefixSegment {
                text: connector,
                style: SegmentStyle::Dim,
            });
        }
        PathRelation::Direct => {
            segments.push(PrefixSegment {
                text: connector,
                style: SegmentStyle::Highlight,
            });
        }
        PathRelation::OnRoute => {
            // Split: junction char as Highlight, rest as Dim.
            // For spaces-only connectors there is no junction to highlight.
            let (junction, tail) = match connector {
                "├── " => ("├", "── "),
                "└── " => ("└", "── "),
                "│   " => ("│", "   "),
                _ => {
                    // "    " — no junction to split; render dim.
                    segments.push(PrefixSegment {
                        text: connector,
                        style: SegmentStyle::Dim,
                    });
                    return;
                }
            };
            segments.push(PrefixSegment {
                text: junction,
                style: SegmentStyle::Highlight,
            });
            segments.push(PrefixSegment {
                text: tail,
                style: SegmentStyle::Dim,
            });
        }
    }
}

/// Apply cursor styling to all content spans of a node's rendered lines.
///
/// Called for the cursor node before prefix prepending.
pub fn apply_cursor_style(lines: &mut Vec<Line<'static>>, style: CursorStyle) {
    for line in lines.iter_mut() {
        for span in line.spans.iter_mut() {
            span.style = match style {
                CursorStyle::Highlight => span.style.fg(theme::cursor()),
                CursorStyle::Underline => span.style.add_modifier(Modifier::UNDERLINED),
            };
        }
    }
}

/// Prepend tree connector characters to a node's rendered lines.
///
/// The first line gets the branch connector (├── or └──). Continuation
/// lines (for multi-line nodes) get the vertical continuation (│   or
/// spaces). The connector color depends on the node's [`PathRelation`] to
/// the cursor: Direct → amber, OnRoute → split amber/dim, Outside → dim.
/// Parent-trail `│` chars are also highlighted when their ancestor is on the
/// cursor path.
pub fn prepend_tree_prefix(
    lines: &mut Vec<Line<'static>>,
    depth: usize,
    is_last_sibling: bool,
    parent_trail: &[bool],
    ancestor_relations: &[PathRelation],
    relation: PathRelation,
) {
    if depth == 0 || lines.is_empty() {
        return;
    }

    for (line_idx, line) in lines.iter_mut().enumerate() {
        let segments = build_prefix_segments(
            depth,
            is_last_sibling,
            parent_trail,
            ancestor_relations,
            line_idx,
            relation,
        );
        let mut prefix_spans: Vec<Span<'static>> =
            segments.iter().map(PrefixSegment::to_span).collect();
        prefix_spans.append(&mut line.spans);
        line.spans = prefix_spans;
    }
}

/// Calculate the total tree prefix width for a node at the given depth.
pub fn tree_prefix_width(depth: usize) -> usize {
    depth * TREE_INDENT_WIDTH
}

// ---------------------------------------------------------------------------
// Tree search
// ---------------------------------------------------------------------------

/// Find a node by its [`NodeId`] in a tree.
///
/// Performs a depth-first search. Returns `None` if the id is not found.
pub fn find_node_by_id(roots: &[TreeNode], id: NodeId) -> Option<&TreeNode> {
    for root in roots {
        if let Some(found) = find_in_subtree(root, id, 0) {
            return Some(found);
        }
    }
    None
}

fn find_in_subtree(node: &TreeNode, id: NodeId, depth: usize) -> Option<&TreeNode> {
    debug_assert!(depth < MAX_TREE_DEPTH, "tree depth {depth} exceeds bound");
    if node.id == id {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_in_subtree(child, id, depth + 1) {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use ratatui::text::Line;

    #[derive(Debug)]
    pub(crate) struct TestNode(pub String);

    impl NodeContent for TestNode {
        fn render(&self, _ctx: &RenderContext) -> Vec<Line<'static>> {
            vec![Line::from(self.0.clone())]
        }
    }

    pub(crate) fn make_leaf(id: u64, label: &str) -> TreeNode {
        TreeNode {
            id: NodeId(id),
            content: Box::new(TestNode(label.to_string())),
            children: vec![],
        }
    }

    pub(crate) fn make_parent(id: u64, label: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            id: NodeId(id),
            content: Box::new(TestNode(label.to_string())),
            children,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::test_helpers::{make_leaf, make_parent};
    use super::*;

    // --- NodeId tests ---

    #[test]
    fn node_id_equality() {
        assert_eq!(NodeId(1), NodeId(1));
        assert_ne!(NodeId(1), NodeId(2));
    }

    #[test]
    fn node_id_hash_works() {
        let mut set = HashSet::new();
        set.insert(NodeId(42));
        assert!(set.contains(&NodeId(42)));
        assert!(!set.contains(&NodeId(43)));
    }

    // --- TreeNode tests ---

    #[test]
    fn leaf_node_is_leaf() {
        let node = make_leaf(0, "leaf");
        assert!(node.is_leaf());
    }

    #[test]
    fn parent_node_is_not_leaf() {
        let node = make_parent(0, "parent", vec![make_leaf(1, "child")]);
        assert!(!node.is_leaf());
    }

    // --- flatten_visible tests ---

    #[test]
    fn flatten_empty_roots() {
        let flat = flatten_visible(&[], &HashSet::new());
        assert!(flat.is_empty());
    }

    #[test]
    fn flatten_single_root_collapsed() {
        let roots = vec![make_parent(0, "root", vec![make_leaf(1, "child")])];
        let flat = flatten_visible(&roots, &HashSet::new());
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].parent_index.is_none());
    }

    #[test]
    fn flatten_single_root_expanded() {
        let roots = vec![make_parent(
            0,
            "root",
            vec![make_leaf(1, "a"), make_leaf(2, "b")],
        )];
        let mut expanded = HashSet::new();
        expanded.insert(NodeId(0));
        let flat = flatten_visible(&roots, &expanded);

        assert_eq!(flat.len(), 3);
        // Root.
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].parent_index.is_none());
        // First child.
        assert_eq!(flat[1].depth, 1);
        assert_eq!(flat[1].parent_index, Some(0));
        assert!(!flat[1].is_last_sibling);
        // Second child (last).
        assert_eq!(flat[2].depth, 1);
        assert_eq!(flat[2].parent_index, Some(0));
        assert!(flat[2].is_last_sibling);
    }

    #[test]
    fn flatten_nested_expansion() {
        // root -> child -> grandchild
        let roots = vec![make_parent(
            0,
            "root",
            vec![make_parent(1, "child", vec![make_leaf(2, "grandchild")])],
        )];
        let mut expanded = HashSet::new();
        expanded.insert(NodeId(0));
        expanded.insert(NodeId(1));
        let flat = flatten_visible(&roots, &expanded);

        assert_eq!(flat.len(), 3);
        assert_eq!(flat[2].depth, 2);
        assert_eq!(flat[2].parent_index, Some(1));
    }

    #[test]
    fn flatten_partial_expansion_hides_grandchildren() {
        let roots = vec![make_parent(
            0,
            "root",
            vec![make_parent(1, "child", vec![make_leaf(2, "grandchild")])],
        )];
        // Only expand root, not child.
        let mut expanded = HashSet::new();
        expanded.insert(NodeId(0));
        let flat = flatten_visible(&roots, &expanded);

        assert_eq!(flat.len(), 2); // root + child, grandchild hidden
    }

    #[test]
    fn flatten_multiple_roots() {
        let roots = vec![make_leaf(0, "a"), make_leaf(1, "b"), make_leaf(2, "c")];
        let flat = flatten_visible(&roots, &HashSet::new());
        assert_eq!(flat.len(), 3);
        assert!(!flat[0].is_last_sibling);
        assert!(!flat[1].is_last_sibling);
        assert!(flat[2].is_last_sibling);
    }

    // --- build_flat_cache tests ---

    #[test]
    fn flat_cache_matches_flat_nodes() {
        let roots = vec![make_parent(0, "root", vec![make_leaf(1, "child")])];
        let mut expanded = HashSet::new();
        expanded.insert(NodeId(0));
        let flat = flatten_visible(&roots, &expanded);
        let cache = build_flat_cache(&flat);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache[0].node_id, NodeId(0));
        assert!(cache[0].has_children);
        assert!(cache[0].parent_flat_index.is_none());
        assert_eq!(cache[1].node_id, NodeId(1));
        assert!(!cache[1].has_children);
        assert_eq!(cache[1].parent_flat_index, Some(0));
    }

    // --- prepend_tree_prefix tests ---

    #[test]
    fn no_prefix_at_depth_zero() {
        let mut lines = vec![Line::from("root content")];
        prepend_tree_prefix(&mut lines, 0, false, &[], &[], PathRelation::Outside);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "root content");
    }

    #[test]
    fn prefix_at_depth_one_not_last() {
        let mut lines = vec![Line::from("child")];
        prepend_tree_prefix(&mut lines, 1, false, &[], &[], PathRelation::Outside);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "├── child");
    }

    #[test]
    fn prefix_at_depth_one_last() {
        let mut lines = vec![Line::from("last child")];
        prepend_tree_prefix(&mut lines, 1, true, &[], &[], PathRelation::Outside);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "└── last child");
    }

    #[test]
    fn prefix_at_depth_two_with_continuation() {
        let mut lines = vec![Line::from("grandchild")];
        // Parent (depth 1) is NOT last sibling → vertical line continues.
        prepend_tree_prefix(
            &mut lines,
            2,
            true,
            &[false],
            &[PathRelation::Outside],
            PathRelation::Outside,
        );
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "│   └── grandchild");
    }

    #[test]
    fn prefix_at_depth_two_parent_is_last() {
        let mut lines = vec![Line::from("grandchild")];
        // Parent (depth 1) IS last sibling → spaces instead of │.
        prepend_tree_prefix(
            &mut lines,
            2,
            false,
            &[true],
            &[PathRelation::Outside],
            PathRelation::Outside,
        );
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "    ├── grandchild");
    }

    #[test]
    fn multiline_node_continuation() {
        let mut lines = vec![Line::from("line1"), Line::from("line2")];
        prepend_tree_prefix(&mut lines, 1, false, &[], &[], PathRelation::Outside);
        let l1: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let l2: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l1, "├── line1");
        assert_eq!(l2, "│   line2");
    }

    #[test]
    fn multiline_last_sibling_continuation() {
        let mut lines = vec![Line::from("line1"), Line::from("line2")];
        prepend_tree_prefix(&mut lines, 1, true, &[], &[], PathRelation::Outside);
        let l1: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let l2: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l1, "└── line1");
        assert_eq!(l2, "    line2");
    }

    // --- tree_prefix_width tests ---

    #[test]
    fn prefix_width_at_various_depths() {
        assert_eq!(tree_prefix_width(0), 0);
        assert_eq!(tree_prefix_width(1), 4);
        assert_eq!(tree_prefix_width(2), 8);
        assert_eq!(tree_prefix_width(3), 12);
    }

    // --- find_node_by_id tests ---

    #[test]
    fn find_node_by_id_finds_root() -> Result<(), String> {
        let roots = vec![make_leaf(0, "root")];
        let found = find_node_by_id(&roots, NodeId(0)).ok_or("expected root node")?;
        assert_eq!(found.id, NodeId(0));
        Ok(())
    }

    #[test]
    fn find_node_by_id_finds_nested_child() -> Result<(), String> {
        let roots = vec![make_parent(
            0,
            "root",
            vec![make_parent(1, "child", vec![make_leaf(2, "grandchild")])],
        )];
        let found = find_node_by_id(&roots, NodeId(2)).ok_or("expected grandchild node")?;
        assert_eq!(found.id, NodeId(2));
        Ok(())
    }

    #[test]
    fn find_node_by_id_returns_none_for_missing() {
        let roots = vec![make_leaf(0, "root")];
        assert!(find_node_by_id(&roots, NodeId(99)).is_none());
    }

    #[test]
    fn find_node_by_id_empty_tree_returns_none() {
        assert!(find_node_by_id(&[], NodeId(42)).is_none());
    }

    // --- build_prefix_segments tests ---

    #[test]
    fn depth_zero_returns_empty() {
        let segs = build_prefix_segments(0, false, &[], &[], 0, PathRelation::Outside);
        assert!(segs.is_empty());
    }

    #[test]
    fn outside_dim_not_last() {
        let segs = build_prefix_segments(1, false, &[], &[], 0, PathRelation::Outside);
        assert_eq!(
            segs,
            vec![PrefixSegment {
                text: "├── ",
                style: SegmentStyle::Dim
            }]
        );
    }

    #[test]
    fn outside_dim_last() {
        let segs = build_prefix_segments(1, true, &[], &[], 0, PathRelation::Outside);
        assert_eq!(
            segs,
            vec![PrefixSegment {
                text: "└── ",
                style: SegmentStyle::Dim
            }]
        );
    }

    #[test]
    fn direct_highlight_not_last() {
        let segs = build_prefix_segments(1, false, &[], &[], 0, PathRelation::Direct);
        assert_eq!(
            segs,
            vec![PrefixSegment {
                text: "├── ",
                style: SegmentStyle::Highlight
            }]
        );
    }

    #[test]
    fn direct_highlight_last() {
        let segs = build_prefix_segments(1, true, &[], &[], 0, PathRelation::Direct);
        assert_eq!(
            segs,
            vec![PrefixSegment {
                text: "└── ",
                style: SegmentStyle::Highlight
            }]
        );
    }

    #[test]
    fn on_route_split_not_last() {
        let segs = build_prefix_segments(1, false, &[], &[], 0, PathRelation::OnRoute);
        assert_eq!(
            segs,
            vec![
                PrefixSegment {
                    text: "├",
                    style: SegmentStyle::Highlight
                },
                PrefixSegment {
                    text: "── ",
                    style: SegmentStyle::Dim
                },
            ]
        );
    }

    #[test]
    fn on_route_split_last() {
        let segs = build_prefix_segments(1, true, &[], &[], 0, PathRelation::OnRoute);
        assert_eq!(
            segs,
            vec![
                PrefixSegment {
                    text: "└",
                    style: SegmentStyle::Highlight
                },
                PrefixSegment {
                    text: "── ",
                    style: SegmentStyle::Dim
                },
            ]
        );
    }

    #[test]
    fn on_route_continuation_split() {
        // Continuation line (line_idx > 0), not last sibling → "│   "
        let segs = build_prefix_segments(1, false, &[], &[], 1, PathRelation::OnRoute);
        assert_eq!(
            segs,
            vec![
                PrefixSegment {
                    text: "│",
                    style: SegmentStyle::Highlight
                },
                PrefixSegment {
                    text: "   ",
                    style: SegmentStyle::Dim
                },
            ]
        );
    }

    #[test]
    fn parent_trail_dim_when_ancestor_outside() {
        // Parent trail entries are Dim when ancestor is Outside.
        for relation in [
            PathRelation::Outside,
            PathRelation::Direct,
            PathRelation::OnRoute,
        ] {
            let segs =
                build_prefix_segments(2, false, &[false], &[PathRelation::Outside], 0, relation);
            assert_eq!(
                segs[0],
                PrefixSegment {
                    text: "│   ",
                    style: SegmentStyle::Dim
                },
                "parent trail should be Dim when ancestor is Outside (node relation: {relation:?})"
            );
        }
    }

    #[test]
    fn parent_trail_highlight_when_ancestor_on_route() {
        // When ancestor is OnRoute, the │ in the parent trail should be
        // split: │ highlighted + spaces dim (path continues past this column).
        let segs = build_prefix_segments(
            2,
            false,
            &[false],
            &[PathRelation::OnRoute],
            0,
            PathRelation::Outside,
        );
        assert_eq!(
            segs[0],
            PrefixSegment {
                text: "│",
                style: SegmentStyle::Highlight
            },
        );
        assert_eq!(
            segs[1],
            PrefixSegment {
                text: "   ",
                style: SegmentStyle::Dim
            },
        );
    }

    #[test]
    fn parent_trail_dim_when_ancestor_direct() {
        // When ancestor is Direct, the │ in the parent trail is dim — the
        // path turns INTO the ancestor, not past it.
        let segs = build_prefix_segments(
            2,
            false,
            &[false],
            &[PathRelation::Direct],
            0,
            PathRelation::Outside,
        );
        assert_eq!(
            segs[0],
            PrefixSegment {
                text: "│   ",
                style: SegmentStyle::Dim
            },
        );
    }

    #[test]
    fn multiline_direct_continuation() {
        // Direct node, continuation line (line_idx=1), not last sibling → Highlight("│   ")
        let segs = build_prefix_segments(1, false, &[], &[], 1, PathRelation::Direct);
        assert_eq!(
            segs,
            vec![PrefixSegment {
                text: "│   ",
                style: SegmentStyle::Highlight
            }]
        );
    }

    #[test]
    fn full_scenario_prefix_segments() {
        // Simulate: depth=2, parent_trail=[false] (parent is not last),
        // ancestor is Outside, node is not last, line 0, Direct.
        // Expected: [Dim("│   "), Highlight("├── ")]
        let segs = build_prefix_segments(
            2,
            false,
            &[false],
            &[PathRelation::Outside],
            0,
            PathRelation::Direct,
        );
        assert_eq!(
            segs,
            vec![
                PrefixSegment {
                    text: "│   ",
                    style: SegmentStyle::Dim
                },
                PrefixSegment {
                    text: "├── ",
                    style: SegmentStyle::Highlight
                },
            ]
        );
    }

    #[test]
    fn full_scenario_ancestor_on_route() {
        // Simulate: depth=2, parent_trail=[false] (parent is not last),
        // ancestor is OnRoute, node is Outside, line 0.
        // Expected: [Highlight("│"), Dim("   "), Dim("├── ")]
        let segs = build_prefix_segments(
            2,
            false,
            &[false],
            &[PathRelation::OnRoute],
            0,
            PathRelation::Outside,
        );
        assert_eq!(
            segs,
            vec![
                PrefixSegment {
                    text: "│",
                    style: SegmentStyle::Highlight
                },
                PrefixSegment {
                    text: "   ",
                    style: SegmentStyle::Dim
                },
                PrefixSegment {
                    text: "├── ",
                    style: SegmentStyle::Dim
                },
            ]
        );
    }

    // --- apply_cursor_style tests ---

    #[test]
    fn highlight_sets_amber_fg() {
        let mut lines = vec![Line::from(Span::styled("hello", Style::default()))];
        apply_cursor_style(&mut lines, CursorStyle::Highlight);
        assert_eq!(lines[0].spans[0].style.fg, Some(theme::cursor()));
    }

    #[test]
    fn underline_adds_modifier() {
        let mut lines = vec![Line::from(Span::styled("hello", Style::default()))];
        apply_cursor_style(&mut lines, CursorStyle::Underline);
        assert!(
            lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    // --- Regression: sibling children parent_trail ---

    /// Reproduce the scenario from the screenshot: a root with two children
    /// where the first child (not-last) is expanded. Verify that the first
    /// child's grandchildren get parent_trail=[false] so the outer │ renders.
    #[test]
    fn expanded_non_last_sibling_children_have_correct_parent_trail() {
        // root(0) → [pair_a(1), pair_b(4)]
        // pair_a(1) → [input(2), output(3)]
        let tree = vec![make_parent(
            0,
            "root",
            vec![
                make_parent(
                    1,
                    "pair_a",
                    vec![make_leaf(2, "input"), make_leaf(3, "output")],
                ),
                make_leaf(4, "pair_b"),
            ],
        )];

        let mut expanded = HashSet::new();
        expanded.insert(NodeId(0));
        expanded.insert(NodeId(1));
        let flat = flatten_visible(&tree, &expanded);

        // Expected flat order:
        // 0: root       depth=0
        // 1: pair_a     depth=1  is_last=false
        // 2: input      depth=2  parent_trail=[false]
        // 3: output     depth=2  parent_trail=[false]
        // 4: pair_b     depth=1  is_last=true
        assert_eq!(flat.len(), 5);
        assert_eq!(flat[1].depth, 1);
        assert!(
            !flat[1].is_last_sibling,
            "pair_a should NOT be last sibling"
        );
        assert_eq!(flat[2].depth, 2);
        assert_eq!(flat[2].parent_trail, vec![false], "input parent_trail");
        assert_eq!(flat[3].depth, 2);
        assert_eq!(flat[3].parent_trail, vec![false], "output parent_trail");
        assert_eq!(flat[4].depth, 1);
        assert!(flat[4].is_last_sibling, "pair_b should be last sibling");
    }

    /// Verify the actual text output for children of a non-last expanded sibling
    /// includes the outer │ connector.
    #[test]
    fn expanded_non_last_sibling_children_render_outer_pipe() {
        // Tree: root(0) → [pair_a(1), pair_b(4)]
        // pair_a(1) → [input(2), output(3)]
        let tree = vec![make_parent(
            0,
            "root",
            vec![
                make_parent(
                    1,
                    "pair_a",
                    vec![make_leaf(2, "input"), make_leaf(3, "output")],
                ),
                make_leaf(4, "pair_b"),
            ],
        )];

        let mut expanded = HashSet::new();
        expanded.insert(NodeId(0));
        expanded.insert(NodeId(1));
        let flat = flatten_visible(&tree, &expanded);

        // Render input (flat[2]) with prepend_tree_prefix (ancestor Outside)
        let mut lines = vec![Line::from("input_content")];
        prepend_tree_prefix(
            &mut lines,
            flat[2].depth,
            flat[2].is_last_sibling,
            &flat[2].parent_trail,
            &[PathRelation::Outside],
            PathRelation::Outside,
        );
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "│   ├── input_content", "input should have outer │");

        // Render output (flat[3]) with prepend_tree_prefix (ancestor Outside)
        let mut lines = vec![Line::from("output_content")];
        prepend_tree_prefix(
            &mut lines,
            flat[3].depth,
            flat[3].is_last_sibling,
            &flat[3].parent_trail,
            &[PathRelation::Outside],
            PathRelation::Outside,
        );
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "│   └── output_content", "output should have outer │");
    }

    // --- NodeIdAllocator tests ---

    #[test]
    fn allocator_starts_at_zero() {
        let mut alloc = NodeIdAllocator::new();
        assert_eq!(alloc.total_allocated(), 0);
        assert_eq!(alloc.allocate(), NodeId(0));
        assert_eq!(alloc.total_allocated(), 1);
    }

    #[test]
    fn allocator_sequential_ids() {
        let mut alloc = NodeIdAllocator::new();
        assert_eq!(alloc.allocate(), NodeId(0));
        assert_eq!(alloc.allocate(), NodeId(1));
        assert_eq!(alloc.allocate(), NodeId(2));
        assert_eq!(alloc.total_allocated(), 3);
    }

    // --- TreeSource + materialize tests ---

    /// Minimal test content implementing NodeContent for materialize tests.
    #[derive(Debug, Clone)]
    struct TestContent {
        label: String,
    }

    impl NodeContent for TestContent {
        fn render(&self, _ctx: &RenderContext) -> Vec<Line<'static>> {
            vec![Line::from(self.label.clone())]
        }
    }

    /// Test source implementing TreeSource for materialize tests.
    #[derive(Clone)]
    struct TestSource {
        label: &'static str,
        child_sources: Vec<TestSource>,
    }

    impl TreeSource for TestSource {
        fn content(&self) -> Box<dyn NodeContent> {
            Box::new(TestContent {
                label: self.label.to_string(),
            })
        }

        fn children(&self) -> Vec<Box<dyn TreeSource>> {
            self.child_sources
                .iter()
                .map(|c| {
                    Box::new(TestSource {
                        label: c.label,
                        child_sources: c.child_sources.clone(),
                    }) as Box<dyn TreeSource>
                })
                .collect()
        }
    }

    #[test]
    fn materialize_empty_sources() {
        let mut alloc = NodeIdAllocator::new();
        let tree = materialize(vec![], &mut alloc);
        assert!(tree.is_empty());
        assert_eq!(alloc.total_allocated(), 0);
    }

    #[test]
    fn materialize_single_leaf() {
        let mut alloc = NodeIdAllocator::new();
        let sources: Vec<Box<dyn TreeSource>> = vec![Box::new(TestSource {
            label: "leaf",
            child_sources: vec![],
        })];
        let tree = materialize(sources, &mut alloc);

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, NodeId(0));
        assert!(tree[0].children.is_empty());
        assert_eq!(alloc.total_allocated(), 1);
    }

    #[test]
    fn materialize_nested_sources() {
        // parent -> [child_a, child_b -> [grandchild]]
        let mut alloc = NodeIdAllocator::new();
        let sources: Vec<Box<dyn TreeSource>> = vec![Box::new(TestSource {
            label: "parent",
            child_sources: vec![
                TestSource {
                    label: "child_a",
                    child_sources: vec![],
                },
                TestSource {
                    label: "child_b",
                    child_sources: vec![TestSource {
                        label: "grandchild",
                        child_sources: vec![],
                    }],
                },
            ],
        })];
        let tree = materialize(sources, &mut alloc);

        // Parent = 0, child_a = 1, child_b = 2, grandchild = 3.
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, NodeId(0));
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].id, NodeId(1));
        assert_eq!(tree[0].children[1].id, NodeId(2));
        assert_eq!(tree[0].children[1].children.len(), 1);
        assert_eq!(tree[0].children[1].children[0].id, NodeId(3));
        assert_eq!(alloc.total_allocated(), 4);
    }

    #[test]
    fn materialize_depth_first_preorder_ids() {
        // Two roots: root_a -> [child], root_b.
        // Expected DFS pre-order: root_a=0, child=1, root_b=2.
        let mut alloc = NodeIdAllocator::new();
        let sources: Vec<Box<dyn TreeSource>> = vec![
            Box::new(TestSource {
                label: "root_a",
                child_sources: vec![TestSource {
                    label: "child",
                    child_sources: vec![],
                }],
            }),
            Box::new(TestSource {
                label: "root_b",
                child_sources: vec![],
            }),
        ];
        let tree = materialize(sources, &mut alloc);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].id, NodeId(0));
        assert_eq!(tree[0].children[0].id, NodeId(1));
        assert_eq!(tree[1].id, NodeId(2));
        assert!(verify_preorder_ids(&tree));
    }

    #[test]
    fn materialize_total_allocated_matches_node_count() {
        let mut alloc = NodeIdAllocator::new();
        let sources: Vec<Box<dyn TreeSource>> = vec![
            Box::new(TestSource {
                label: "a",
                child_sources: vec![
                    TestSource {
                        label: "a1",
                        child_sources: vec![],
                    },
                    TestSource {
                        label: "a2",
                        child_sources: vec![],
                    },
                ],
            }),
            Box::new(TestSource {
                label: "b",
                child_sources: vec![],
            }),
        ];
        let tree = materialize(sources, &mut alloc);

        assert_eq!(alloc.total_allocated(), 4);
        assert_eq!(count_tree_nodes(&tree), 4);
    }

    // --- verify_preorder_ids tests ---

    #[test]
    fn verify_preorder_ids_empty_tree() {
        assert!(verify_preorder_ids(&[]));
    }

    #[test]
    fn verify_preorder_ids_valid_tree() {
        // Manually construct a valid pre-order tree.
        let tree = vec![make_parent(
            0,
            "root",
            vec![make_leaf(1, "a"), make_leaf(2, "b")],
        )];
        assert!(verify_preorder_ids(&tree));
    }

    #[test]
    fn verify_preorder_ids_rejects_out_of_order() {
        // Child ID (0) is less than parent ID (1) — violates pre-order.
        let tree = vec![make_parent(1, "root", vec![make_leaf(0, "child")])];
        assert!(!verify_preorder_ids(&tree));
    }

    #[test]
    fn verify_preorder_ids_rejects_duplicate() {
        // Duplicate IDs violate monotonicity.
        let tree = vec![make_parent(0, "root", vec![make_leaf(0, "child")])];
        assert!(!verify_preorder_ids(&tree));
    }

    // --- count_tree_nodes tests ---

    #[test]
    fn count_tree_nodes_empty() {
        assert_eq!(count_tree_nodes(&[]), 0);
    }

    #[test]
    fn count_tree_nodes_nested() {
        let tree = vec![make_parent(
            0,
            "root",
            vec![
                make_leaf(1, "a"),
                make_parent(2, "b", vec![make_leaf(3, "c")]),
            ],
        )];
        assert_eq!(count_tree_nodes(&tree), 4);
    }
}

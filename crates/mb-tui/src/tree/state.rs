//! Tree navigation state for the events pane.
//!
//! [`TreeState`] replaces `ChatViewState`, providing cursor, scroll, and
//! expansion tracking for the recursive tree architecture. Navigation is
//! tree-walking: `j`/`k` move through flattened visible nodes, `l` drills
//! in (expand or move to first child), `h` drills out (collapse and move
//! to parent).

use std::collections::HashSet;

use super::{CachedFlatEntry, FlatNode, NodeId, TreeNode, build_flat_cache, find_node_by_id};

// ---------------------------------------------------------------------------
// TreeState
// ---------------------------------------------------------------------------

/// Persistent state for tree navigation in the events pane.
///
/// Rebuilt each frame from events, but expansion state, cursor, and scroll
/// persist across frames. The cached tree is stored for detail viewer access
/// between frames.
pub struct TreeState {
    /// Which nodes are expanded (by NodeId). Absent = collapsed.
    expanded: HashSet<NodeId>,
    /// Cursor position as flat index into visible node list.
    cursor: usize,
    /// Row-level scroll offset for the layout pager.
    scroll_offset: usize,
    /// Whether to auto-follow tail when new events arrive.
    auto_scroll: bool,
    /// Cached visible node count (updated each frame).
    cached_visible_count: usize,
    /// Cached flat entries for navigation between frames.
    cached_flat_entries: Vec<CachedFlatEntry>,
    /// Cached tree for detail viewer access (Enter key).
    cached_tree: Vec<TreeNode>,
    /// Generation of the EventBuffer when cached_tree was built.
    cached_tree_generation: Option<u64>,
    /// Viewport height in rows, updated each frame by the renderer.
    cached_viewport_height: u16,
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeState {
    pub fn new() -> Self {
        Self {
            expanded: HashSet::new(),
            cursor: 0,
            scroll_offset: 0,
            auto_scroll: true,
            cached_visible_count: 0,
            cached_flat_entries: Vec::new(),
            cached_tree: Vec::new(),
            cached_tree_generation: None,
            cached_viewport_height: 0,
        }
    }

    // --- Accessors ---

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Set the cursor to a specific flat index.
    ///
    /// Clamps to the last visible node. Disables auto-scroll.
    pub fn set_cursor(&mut self, index: usize) {
        if self.cached_visible_count > 0 {
            self.cursor = index.min(self.cached_visible_count - 1);
        } else {
            self.cursor = 0;
        }
        self.auto_scroll = false;
    }

    pub fn cached_flat_entries(&self) -> &[CachedFlatEntry] {
        &self.cached_flat_entries
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    pub fn auto_scroll(&self) -> bool {
        self.auto_scroll
    }

    pub fn expanded(&self) -> &HashSet<NodeId> {
        &self.expanded
    }

    #[cfg(test)]
    pub fn cached_visible_count(&self) -> usize {
        self.cached_visible_count
    }

    /// Whether any nodes are expanded.
    pub fn has_expanded(&self) -> bool {
        !self.expanded.is_empty()
    }

    /// Ensure a specific node is expanded. Idempotent.
    pub fn ensure_expanded(&mut self, id: NodeId) {
        self.expanded.insert(id);
    }

    /// Get the node currently under the cursor.
    pub fn selected_node(&self) -> Option<&TreeNode> {
        let entry = self.cached_flat_entries.get(self.cursor)?;
        find_node_by_id(&self.cached_tree, entry.node_id)
    }

    /// Look up the full text of the node at the given flat index.
    ///
    /// Returns `None` if the index is out of range, the node is missing
    /// from the cached tree, or the node has no full text.
    pub fn full_text_at(&self, flat_idx: usize) -> Option<String> {
        let entry = self.cached_flat_entries.get(flat_idx)?;
        let node = find_node_by_id(&self.cached_tree, entry.node_id)?;
        node.content.full_text()
    }

    // --- Frame update ---

    /// Store a fresh tree for the current frame. Called once per frame before
    /// rendering. The renderer will call [`refresh_cache`] with the flattened
    /// result so we only flatten once.
    pub fn set_tree(&mut self, tree: Vec<TreeNode>, generation: u64) {
        self.cached_tree = tree;
        // u64::MAX is a sentinel meaning "always rebuild" — never cache it.
        self.cached_tree_generation = if generation == u64::MAX {
            None
        } else {
            Some(generation)
        };
    }

    /// Return the cached tree if it was built from the given generation.
    ///
    /// Takes ownership via `mem::take` so the caller can use it without
    /// re-borrowing `self`. Returns `None` if the generation doesn't match,
    /// signalling that a fresh `events_to_tree()` is needed.
    pub fn take_tree_if_current(&mut self, generation: u64) -> Option<Vec<TreeNode>> {
        if self.cached_tree_generation == Some(generation) && !self.cached_tree.is_empty() {
            Some(std::mem::take(&mut self.cached_tree))
        } else {
            None
        }
    }

    /// Update the navigation cache from a just-flattened visible node list.
    ///
    /// Called by the renderer after `flatten_visible`, so we piggyback on
    /// the single flatten rather than flattening twice.
    pub fn refresh_cache(&mut self, flat: &[FlatNode<'_>]) {
        self.cached_visible_count = flat.len();
        self.cached_flat_entries = build_flat_cache(flat);

        debug_assert_eq!(
            self.cached_flat_entries.len(),
            self.cached_visible_count,
            "flat cache size must match visible count"
        );

        // Clamp cursor.
        if self.cached_visible_count > 0 {
            self.cursor = self.cursor.min(self.cached_visible_count - 1);
        } else {
            self.cursor = 0;
        }
    }

    // --- Scroll / Jump ---

    /// Jump to the top. Disables auto-scroll.
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.cursor = 0;
        self.auto_scroll = false;
    }

    /// Jump to the bottom. Re-enables auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = usize::MAX;
        self.cursor = usize::MAX; // Clamped next frame.
        self.auto_scroll = true;
    }

    // --- Cursor movement ---

    /// Move cursor up one visible node.
    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.auto_scroll = false;
    }

    /// Move cursor down one visible node.
    pub fn cursor_down(&mut self) {
        if self.cached_visible_count > 0 {
            self.cursor = self
                .cursor
                .saturating_add(1)
                .min(self.cached_visible_count - 1);
        }
        self.auto_scroll = false;
    }

    /// Move cursor up by `n` visible nodes.
    pub fn cursor_up_n(&mut self, n: usize) {
        for _ in 0..n {
            self.cursor_up();
        }
    }

    /// Move cursor down by `n` visible nodes.
    pub fn cursor_down_n(&mut self, n: usize) {
        for _ in 0..n {
            self.cursor_down();
        }
    }

    // --- Viewport / half-page scroll ---

    /// Update the cached viewport height. Called by the renderer each frame.
    pub fn set_viewport_height(&mut self, height: u16) {
        self.cached_viewport_height = height;
    }

    /// Move cursor down by half the viewport height.
    pub fn half_page_down(&mut self) {
        let half = (self.cached_viewport_height / 2).max(1) as usize;
        self.cursor_down_n(half);
    }

    /// Move cursor up by half the viewport height.
    pub fn half_page_up(&mut self) {
        let half = (self.cached_viewport_height / 2).max(1) as usize;
        self.cursor_up_n(half);
    }

    // --- Tree navigation ---

    /// Drill into the current node (l key).
    ///
    /// - If node has children and is collapsed → expand.
    /// - If node has children and is expanded → move cursor to first child.
    /// - If leaf → no-op.
    ///
    /// Returns `true` if the action was handled (expanded or moved cursor),
    /// `false` if it was a no-op (caller should fall through to pane navigation).
    pub fn drill_in(&mut self) -> bool {
        let Some(entry) = self.cached_flat_entries.get(self.cursor) else {
            return false;
        };
        if !entry.has_children {
            return false;
        }

        let node_id = entry.node_id;
        if self.expanded.contains(&node_id) {
            // Already expanded → move cursor to first child (cursor + 1).
            let next = self.cursor + 1;
            debug_assert!(
                next < self.cached_visible_count,
                "expanded node must have visible children"
            );
            if next < self.cached_visible_count {
                self.cursor = next;
            }
        } else {
            // Collapsed → expand.
            self.expanded.insert(node_id);
        }
        self.auto_scroll = false;
        true
    }

    /// Drill out of the current node (h key).
    ///
    /// - If non-root: collapse current node (if expanded) and move to parent.
    /// - If root and expanded: collapse only.
    /// - If root and collapsed: return false (caller should switch panes).
    pub fn drill_out(&mut self) -> bool {
        let Some(entry) = self.cached_flat_entries.get(self.cursor) else {
            return false;
        };
        let node_id = entry.node_id;
        let parent_idx = entry.parent_flat_index;

        if let Some(parent) = parent_idx {
            // Non-root: collapse current and move to parent.
            self.expanded.remove(&node_id);
            self.cursor = parent;
            self.auto_scroll = false;
            true
        } else {
            // Root node.
            if self.expanded.contains(&node_id) {
                self.expanded.remove(&node_id);
                self.auto_scroll = false;
                true
            } else {
                false // Signal caller to switch panes.
            }
        }
    }

    // --- Expand/Collapse all ---

    /// Expand all root nodes one level.
    pub fn expand_all_roots(&mut self) {
        for entry in &self.cached_flat_entries {
            if entry.parent_flat_index.is_none() && entry.has_children {
                self.expanded.insert(entry.node_id);
            }
        }
    }

    /// Collapse all nodes.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{
        flatten_visible,
        test_helpers::{make_leaf, make_parent},
    };

    // --- Test helpers ---

    /// Flatten + refresh cache + store tree. Mirrors the render pipeline.
    fn sync_tree(state: &mut TreeState, tree: Vec<TreeNode>) {
        let flat = flatten_visible(&tree, state.expanded());
        state.refresh_cache(&flat);
        state.set_tree(tree, 0);
    }

    fn sample_tree() -> Vec<TreeNode> {
        vec![make_parent(
            0,
            "root",
            vec![
                make_parent(1, "child_a", vec![make_leaf(3, "grandchild")]),
                make_leaf(2, "child_b"),
            ],
        )]
    }

    // --- new defaults ---

    #[test]
    fn new_defaults() {
        let state = TreeState::new();
        assert!(state.auto_scroll);
        assert_eq!(state.cursor, 0);
        assert_eq!(state.scroll_offset, 0);
        assert!(!state.has_expanded());
    }

    // --- update_cache ---

    #[test]
    fn update_cache_sets_visible_count() {
        let mut state = TreeState::new();
        let tree = sample_tree();
        sync_tree(&mut state, tree);
        // Only root is visible (collapsed).
        assert_eq!(state.cached_visible_count(), 1);
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn update_cache_clamps_cursor() {
        let mut state = TreeState::new();
        state.cursor = 100; // Way out of range.
        let tree = sample_tree();
        sync_tree(&mut state, tree);
        assert_eq!(state.cursor(), 0); // Clamped to 0 (only 1 visible).
    }

    #[test]
    fn update_cache_with_expanded_root() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        let tree = sample_tree();
        sync_tree(&mut state, tree);
        // Root + 2 children visible.
        assert_eq!(state.cached_visible_count(), 3);
    }

    // --- scroll_to_top / scroll_to_bottom ---

    #[test]
    fn scroll_to_top_resets() {
        let mut state = TreeState::new();
        state.scroll_offset = 50;
        state.cursor = 10;
        state.scroll_to_top();
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(state.cursor(), 0);
        assert!(!state.auto_scroll());
    }

    #[test]
    fn scroll_to_bottom_reenables_auto_scroll() {
        let mut state = TreeState::new();
        state.auto_scroll = false;
        state.scroll_to_bottom();
        assert!(state.auto_scroll());
        assert_eq!(state.scroll_offset(), usize::MAX);
        assert_eq!(state.cursor(), usize::MAX);
    }

    // --- cursor_up / cursor_down ---

    #[test]
    fn cursor_up_saturates_at_zero() {
        let mut state = TreeState::new();
        state.cursor_up();
        assert_eq!(state.cursor(), 0);
        assert!(!state.auto_scroll());
    }

    #[test]
    fn cursor_down_clamps_to_max() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        // 3 visible: root, child_a, child_b
        assert_eq!(state.cached_visible_count(), 3);

        state.cursor_down(); // 0 -> 1
        assert_eq!(state.cursor(), 1);
        state.cursor_down(); // 1 -> 2
        assert_eq!(state.cursor(), 2);
        state.cursor_down(); // clamped at 2
        assert_eq!(state.cursor(), 2);
    }

    // --- cursor_up_n / cursor_down_n ---

    #[test]
    fn cursor_down_n_moves_by_count() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        state.expanded.insert(NodeId(1));
        sync_tree(&mut state, sample_tree());
        // 4 visible: root, child_a, grandchild, child_b
        assert_eq!(state.cached_visible_count(), 4);

        state.cursor_down_n(3); // 0 -> 3
        assert_eq!(state.cursor(), 3);
    }

    #[test]
    fn cursor_down_n_clamps_at_end() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        // 3 visible: root, child_a, child_b
        assert_eq!(state.cached_visible_count(), 3);

        state.cursor_down_n(100); // Clamps to 2
        assert_eq!(state.cursor(), 2);
    }

    #[test]
    fn cursor_up_n_moves_by_count() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        state.cursor = 2; // Start at child_b

        state.cursor_up_n(2); // 2 -> 0
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn cursor_up_n_clamps_at_zero() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        state.cursor = 1;

        state.cursor_up_n(100); // Clamps to 0
        assert_eq!(state.cursor(), 0);
    }

    // --- drill_in ---

    #[test]
    fn drill_in_on_leaf_is_noop() {
        let mut state = TreeState::new();
        sync_tree(&mut state, vec![make_leaf(0, "leaf")]);
        state.drill_in();
        assert!(!state.has_expanded());
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn drill_in_on_leaf_returns_false() {
        let mut state = TreeState::new();
        sync_tree(&mut state, vec![make_leaf(0, "leaf")]);
        let handled = state.drill_in();
        assert!(!handled);
        assert!(!state.has_expanded());
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn drill_in_on_collapsed_parent_expands() {
        let mut state = TreeState::new();
        sync_tree(&mut state, sample_tree());
        assert!(!state.has_expanded());
        state.drill_in();
        assert!(state.expanded().contains(&NodeId(0)));
        assert_eq!(state.cursor(), 0); // Stays on parent.
    }

    #[test]
    fn drill_in_on_collapsed_parent_returns_true() {
        let mut state = TreeState::new();
        sync_tree(&mut state, sample_tree());
        let handled = state.drill_in();
        assert!(handled);
        assert!(state.expanded().contains(&NodeId(0)));
    }

    #[test]
    fn drill_in_on_expanded_parent_moves_to_child() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        // Cursor at 0 (root, expanded). Drill in → move to first child.
        state.drill_in();
        assert_eq!(state.cursor(), 1);
    }

    #[test]
    fn drill_in_on_expanded_parent_returns_true() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        let handled = state.drill_in();
        assert!(handled);
        assert_eq!(state.cursor(), 1);
    }

    #[test]
    fn drill_in_on_empty_cache_returns_false() {
        let mut state = TreeState::new();
        let handled = state.drill_in();
        assert!(!handled);
    }

    // --- drill_out ---

    #[test]
    fn drill_out_from_child_collapses_and_moves_to_parent() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        // Move cursor to child_a (flat index 1).
        state.cursor = 1;
        let handled = state.drill_out();
        assert!(handled);
        assert_eq!(state.cursor(), 0); // Moved to parent.
        // child_a (NodeId(1)) should be removed from expanded (was never expanded anyway).
    }

    #[test]
    fn drill_out_from_expanded_root_collapses() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        // Cursor at root (flat index 0).
        let handled = state.drill_out();
        assert!(handled);
        assert!(!state.expanded().contains(&NodeId(0)));
    }

    #[test]
    fn drill_out_from_collapsed_root_returns_false() {
        let mut state = TreeState::new();
        sync_tree(&mut state, sample_tree());
        let handled = state.drill_out();
        assert!(!handled);
    }

    #[test]
    fn drill_out_from_deep_child_collapses_and_moves_to_parent() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        state.expanded.insert(NodeId(1));
        sync_tree(&mut state, sample_tree());
        // Visible: root(0), child_a(1), grandchild(2), child_b(3)
        assert_eq!(state.cached_visible_count(), 4);

        // Cursor at grandchild (flat index 2).
        state.cursor = 2;
        let handled = state.drill_out();
        assert!(handled);
        assert_eq!(state.cursor(), 1); // Moved to child_a.
    }

    // --- expand_all_roots / collapse_all ---

    #[test]
    fn expand_all_roots_expands_root_nodes() {
        let mut state = TreeState::new();
        let tree = vec![
            make_parent(0, "root_a", vec![make_leaf(1, "child")]),
            make_parent(2, "root_b", vec![make_leaf(3, "child")]),
        ];
        sync_tree(&mut state, tree);
        state.expand_all_roots();
        assert!(state.expanded().contains(&NodeId(0)));
        assert!(state.expanded().contains(&NodeId(2)));
    }

    #[test]
    fn expand_all_roots_skips_leaf_roots() {
        let mut state = TreeState::new();
        let tree = vec![
            make_leaf(0, "leaf_root"),
            make_parent(1, "parent_root", vec![make_leaf(2, "child")]),
        ];
        sync_tree(&mut state, tree);
        state.expand_all_roots();
        assert!(!state.expanded().contains(&NodeId(0)));
        assert!(state.expanded().contains(&NodeId(1)));
    }

    #[test]
    fn collapse_all_clears_expanded() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        state.expanded.insert(NodeId(1));
        state.collapse_all();
        assert!(!state.has_expanded());
        assert!(state.expanded().is_empty());
    }

    // --- selected_node ---

    #[test]
    fn selected_node_returns_current() -> Result<(), String> {
        let mut state = TreeState::new();
        sync_tree(&mut state, sample_tree());
        let node = state.selected_node().ok_or("expected selected node")?;
        assert_eq!(node.id, NodeId(0));
        Ok(())
    }

    #[test]
    fn selected_node_returns_none_on_empty_tree() {
        let state = TreeState::new();
        assert!(state.selected_node().is_none());
    }

    // --- full navigation sequence ---

    #[test]
    fn full_drill_in_out_sequence() {
        let mut state = TreeState::new();
        sync_tree(&mut state, sample_tree());

        // Start at root (collapsed).
        assert_eq!(state.cursor(), 0);

        // l: expand root.
        state.drill_in();
        assert!(state.expanded().contains(&NodeId(0)));

        // Refresh cache after expansion.
        sync_tree(&mut state, sample_tree());
        assert_eq!(state.cached_visible_count(), 3);

        // l: move to first child.
        state.drill_in();
        assert_eq!(state.cursor(), 1);

        // l: expand child_a.
        state.drill_in();
        assert!(state.expanded().contains(&NodeId(1)));

        // Refresh cache.
        sync_tree(&mut state, sample_tree());
        assert_eq!(state.cached_visible_count(), 4);

        // l: move to grandchild.
        state.drill_in();
        assert_eq!(state.cursor(), 2);

        // l: grandchild is leaf, no-op.
        state.drill_in();
        assert_eq!(state.cursor(), 2);

        // h: collapse grandchild (no-op) + move to parent child_a.
        let handled = state.drill_out();
        assert!(handled);
        assert_eq!(state.cursor(), 1);

        // h: collapse child_a + move to root.
        let handled = state.drill_out();
        assert!(handled);
        assert_eq!(state.cursor(), 0);

        // h: collapse root.
        let handled = state.drill_out();
        assert!(handled);
        assert!(!state.expanded().contains(&NodeId(0)));

        // h: root collapsed → return false.
        sync_tree(&mut state, sample_tree());
        let handled = state.drill_out();
        assert!(!handled);
    }

    // --- half_page_down / half_page_up ---

    #[test]
    fn half_page_down_moves_by_half_viewport() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        state.expanded.insert(NodeId(1));
        sync_tree(&mut state, sample_tree());
        // 4 visible: root, child_a, grandchild, child_b
        assert_eq!(state.cached_visible_count(), 4);

        state.set_viewport_height(6); // half = 3
        state.half_page_down();
        assert_eq!(state.cursor(), 3);
    }

    #[test]
    fn half_page_up_clamps_at_top() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        state.cursor = 1;
        state.set_viewport_height(10); // half = 5

        state.half_page_up();
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn half_page_with_zero_viewport_moves_by_one() {
        let mut state = TreeState::new();
        state.expanded.insert(NodeId(0));
        sync_tree(&mut state, sample_tree());
        // 3 visible: root, child_a, child_b
        assert_eq!(state.cached_visible_count(), 3);

        state.set_viewport_height(0); // half = max(0/2, 1) = 1
        state.half_page_down();
        assert_eq!(state.cursor(), 1);
    }
}

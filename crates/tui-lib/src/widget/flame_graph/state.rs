use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::data::{SpanId, SpanNode};
use super::layout::{FlameRow, RowKind, flatten_visible_rows};
use super::render::BarStyle;
use crate::input::KeyResult;

const DECAY_TIME_CONSTANTS: f64 = 5.0;
const SNAP_EPSILON: f64 = 0.001;
const TRANSITION_MS: f64 = 600.0;

/// Navigation reference width. Row ordering is width-independent, so any
/// positive value works for cursor math.
const NAV_WIDTH: u16 = 1000;

/// Animation state for an expand/collapse transition.
#[derive(Clone, Debug)]
pub struct ExpandAnimation {
    pub value: f64,
    pub target: f64,
}

/// View state for a horizontal flame graph widget.
///
/// State is represented as a single path from root to the cursor node.
/// A node is "expanded" (its children visible) iff it is on the path and
/// is not the last element (the leaf/cursor node).
const UNDO_LIMIT: usize = 32;

#[derive(Clone)]
pub struct FlameGraph {
    pub(crate) root: SpanNode,
    pub(crate) cost_types: Vec<super::data::CostType>,
    /// Path from root to the cursor node. Always non-empty.
    pub(crate) path: Vec<SpanId>,
    /// Index into the visible-row list for the cursor.
    pub(crate) cursor: usize,
    pub(crate) selected_for_legend: Option<SpanId>,
    pub(crate) animations: HashMap<SpanId, ExpandAnimation>,
    /// When set, this span is the visual root (focus mode).
    pub(crate) focus: Option<SpanId>,
    /// Visual style for bar segments.
    pub(crate) bar_style: BarStyle,
    /// Undo stack of previous (path, focus) pairs (bounded to UNDO_LIMIT).
    undo_stack: Vec<(Vec<SpanId>, Option<SpanId>)>,
    /// Redo stack of (path, focus) pairs (cleared on new navigation).
    redo_stack: Vec<(Vec<SpanId>, Option<SpanId>)>,
    /// Vertical scroll offset (number of rows scrolled past the top).
    pub(crate) scroll_offset: usize,
    /// Last known viewport height (rows), cached during render for scroll math.
    viewport_height: u16,
}

impl FlameGraph {
    pub fn new(root: SpanNode, cost_types: Vec<super::data::CostType>) -> Self {
        let root_id = root.id;
        let path = vec![root_id];
        Self {
            root,
            cost_types,
            path,
            cursor: 0,
            selected_for_legend: Some(root_id),
            animations: HashMap::new(),
            focus: None,
            bar_style: BarStyle::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            scroll_offset: 0,
            viewport_height: 0,
        }
    }

    /// Dispatch a key press. Returns whether the key was consumed.
    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
        let has_modifier = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if has_modifier {
            return KeyResult::Ignored;
        }

        let rows = self.visible_rows();

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor_up(&rows),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor_down(&rows),
            KeyCode::Right | KeyCode::Char('l') => self.expand_or_descend(),
            KeyCode::Left | KeyCode::Char('h') => self.collapse_or_ascend(),
            KeyCode::Char('f') => self.focus_on_selected(),
            KeyCode::Char('F') => self.unfocus(),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('r') => self.redo(),
            KeyCode::Enter => self.toggle_legend(&rows),
            _ => KeyResult::Ignored,
        }
    }

    /// Advance expand/collapse animations by `dt`.
    pub fn tick(&mut self, dt: Duration) {
        let dt_secs = dt.as_secs_f64();
        let k = DECAY_TIME_CONSTANTS / (TRANSITION_MS / 1000.0);
        let factor = 1.0 - (-k * dt_secs).exp();

        let mut finished = Vec::new();
        let mut any_collapse_finished = false;
        for (span_id, anim) in &mut self.animations {
            anim.value += (anim.target - anim.value) * factor;
            if (anim.value - anim.target).abs() < SNAP_EPSILON {
                anim.value = anim.target;
                finished.push(*span_id);
            }
        }
        for span_id in finished {
            let anim = self.animations.remove(&span_id);
            if anim.is_some_and(|a| a.target == 0.0) {
                any_collapse_finished = true;
            }
        }

        // When a collapse finishes, the visible row list shrinks. Reposition
        // the cursor to the path leaf in the new list so it stays valid.
        if any_collapse_finished {
            self.reposition_cursor_to_path_leaf();
        }
    }

    /// True when the widget needs idle redraws to advance animations.
    pub fn needs_idle_render(&self) -> bool {
        !self.animations.is_empty()
    }

    /// Current cursor index in the visible-row list.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the given span is currently expanded (children visible).
    ///
    /// A node is expanded iff it is on the path and is not the leaf.
    pub fn is_expanded(&self, span_id: SpanId) -> bool {
        self.path.contains(&span_id) && self.path.last() != Some(&span_id)
    }

    /// The root span node of this flame graph.
    pub fn root(&self) -> &SpanNode {
        &self.root
    }

    /// The currently focused span, if any (focus mode).
    pub fn focus(&self) -> Option<SpanId> {
        self.focus
    }

    /// The current path from root to cursor.
    pub fn path(&self) -> &[SpanId] {
        &self.path
    }

    /// Replace the cost types (palette) without disturbing navigation state.
    pub fn set_cost_types(&mut self, cost_types: Vec<super::data::CostType>) {
        self.cost_types = cost_types;
    }

    /// The current bar style.
    pub fn bar_style(&self) -> BarStyle {
        self.bar_style
    }

    /// Set the visual style for bar segments.
    pub fn set_bar_style(&mut self, style: BarStyle) {
        self.bar_style = style;
    }

    /// Current vertical scroll offset.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Update viewport height (called by render to cache for scroll math).
    pub(crate) fn set_viewport_height(&mut self, height: u16) {
        self.viewport_height = height;
    }

    /// Adjust scroll offset so the cursor row is visible within the viewport.
    fn ensure_cursor_visible(&mut self) {
        let vh = self.viewport_height as usize;
        if vh == 0 {
            return;
        }
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + vh {
            self.scroll_offset = self.cursor + 1 - vh;
        }
    }

    /// Flatten the tree into visible rows using a navigation-only width.
    pub fn visible_rows(&self) -> Vec<FlameRow> {
        self.visible_rows_for_width(NAV_WIDTH)
    }

    /// Flatten the tree into visible rows using the given render width.
    pub(crate) fn visible_rows_for_width(&self, width: u16) -> Vec<FlameRow> {
        flatten_visible_rows(
            &self.root,
            &self.path,
            &self.animations,
            self.selected_for_legend,
            self.focus,
            width,
        )
    }
}

// --- Animation helpers ---

impl FlameGraph {
    /// Start an expand animation for a node (children becoming visible).
    fn start_expand(&mut self, span_id: SpanId) {
        self.animations.insert(
            span_id,
            ExpandAnimation {
                value: SNAP_EPSILON * 2.0,
                target: 1.0,
            },
        );
    }

    /// Start a collapse animation for a node (children becoming hidden).
    fn start_collapse(&mut self, span_id: SpanId) {
        if let Some(anim) = self.animations.get_mut(&span_id) {
            anim.target = 0.0;
        } else {
            self.animations.insert(
                span_id,
                ExpandAnimation {
                    value: 1.0,
                    target: 0.0,
                },
            );
        }
    }

    /// Transition animations when switching from `self.path` to `new_path`.
    ///
    /// Starts collapse animations for nodes that were expanded but won't be,
    /// and expand animations for nodes that weren't expanded but will be.
    fn transition_animations(&mut self, new_path: &[SpanId]) {
        debug_assert!(!new_path.is_empty(), "new_path must not be empty");
        // Clone the old path to avoid borrowing `self` immutably during
        // mutable `start_collapse` / `start_expand` calls.
        let old_path = self.path.clone();
        let old_leaf = old_path.last().copied();
        for &old_id in &old_path {
            let was_expanded = Some(old_id) != old_leaf;
            let still_expanded = new_path.contains(&old_id) && new_path.last() != Some(&old_id);
            if was_expanded && !still_expanded {
                self.start_collapse(old_id);
            }
        }
        for (i, &new_id) in new_path.iter().enumerate() {
            let is_leaf = i == new_path.len() - 1;
            if !is_leaf && !old_path.contains(&new_id) {
                self.start_expand(new_id);
            }
        }
    }
}

// --- Undo/redo ---

impl FlameGraph {
    /// Save current path and focus to undo stack before a navigation action.
    fn push_undo(&mut self) {
        self.undo_stack.push((self.path.clone(), self.focus));
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn undo(&mut self) -> KeyResult {
        let Some((prev_path, prev_focus)) = self.undo_stack.pop() else {
            return KeyResult::Consumed;
        };
        self.redo_stack.push((self.path.clone(), self.focus));
        self.apply_path(prev_path);
        self.focus = prev_focus;
        KeyResult::Consumed
    }

    fn redo(&mut self) -> KeyResult {
        let Some((next_path, next_focus)) = self.redo_stack.pop() else {
            return KeyResult::Consumed;
        };
        self.undo_stack.push((self.path.clone(), self.focus));
        self.apply_path(next_path);
        self.focus = next_focus;
        KeyResult::Consumed
    }

    /// Apply a path (from undo/redo), updating animations and cursor.
    fn apply_path(&mut self, new_path: Vec<SpanId>) {
        self.transition_animations(&new_path);
        self.path = new_path;
        debug_assert!(
            !self.path.is_empty(),
            "path must never be empty after apply"
        );
        self.reposition_cursor_to_path_leaf();
    }
}

// --- Focus mode ---

impl FlameGraph {
    fn focus_on_selected(&mut self) -> KeyResult {
        let rows = self.visible_rows();
        let span_id = match span_id_at(&rows, self.cursor) {
            Some(id) => id,
            None => return KeyResult::Consumed,
        };
        // Focus on root is a no-op (root is already the real root).
        if span_id == self.root.id {
            return KeyResult::Consumed;
        }
        self.push_undo();
        self.focus = Some(span_id);

        // Ensure the focused node is selected (cursor + legend).
        if self.selected_for_legend.is_some() {
            self.selected_for_legend = Some(span_id);
        }

        // Auto-expand if the focused node has children.
        let first_child_id = find_node(&self.root, span_id)
            .filter(|n| !n.children.is_empty())
            .and_then(|n| n.children.first().map(|c| c.id));
        if let Some(child_id) = first_child_id {
            if !self.is_expanded(span_id) {
                self.animations.insert(
                    span_id,
                    ExpandAnimation {
                        value: SNAP_EPSILON * 2.0,
                        target: 1.0,
                    },
                );
            }
            self.path = ancestor_path(&self.root, child_id);
            if self.selected_for_legend.is_some() {
                self.selected_for_legend = Some(child_id);
            }
            self.move_cursor_to_span(child_id);
        } else {
            self.move_cursor_to_span(span_id);
        }
        KeyResult::Consumed
    }

    /// Unfocus via `F`: keep cursor on the currently selected span.
    fn unfocus(&mut self) -> KeyResult {
        if self.focus.is_none() {
            return KeyResult::Consumed;
        }
        let rows = self.visible_rows();
        let current_span = span_id_at(&rows, self.cursor);
        self.push_undo();
        self.focus = None;
        // Reposition cursor to the same span in the full (unfocused) tree.
        if let Some(span_id) = current_span {
            self.sync_path_to_cursor(span_id);
            self.move_cursor_to_span(span_id);
        }
        KeyResult::Consumed
    }

    /// Unfocus via `h` on the focus node: move cursor to focus node's parent.
    fn unfocus_to_parent(&mut self) -> KeyResult {
        let Some(focus_id) = self.focus else {
            return KeyResult::Consumed;
        };
        // Find the parent of the focus node via ancestor path.
        let focus_path = ancestor_path(&self.root, focus_id);
        let parent_id = if focus_path.len() >= 2 {
            focus_path[focus_path.len() - 2]
        } else {
            focus_id // focus node is root (shouldn't happen, but safe fallback)
        };
        self.push_undo();
        self.focus = None;
        self.sync_path_to_cursor(parent_id);
        if self.selected_for_legend.is_some() {
            self.selected_for_legend = Some(parent_id);
        }
        self.move_cursor_to_span(parent_id);
        KeyResult::Consumed
    }
}

// --- Navigation helpers (private) ---

impl FlameGraph {
    fn move_cursor_up(&mut self, rows: &[FlameRow]) -> KeyResult {
        self.push_undo();
        // Find the target span id from the pre-move row list.
        let mut target_id = None;
        let mut candidate = self.cursor;
        while candidate > 0 {
            candidate -= 1;
            if !is_legend_row(rows, candidate) {
                target_id = span_id_at(rows, candidate);
                break;
            }
        }
        if let Some(new_id) = target_id {
            // Sync legend and path first, then reposition cursor.
            if self.selected_for_legend.is_some() {
                self.selected_for_legend = Some(new_id);
            }
            self.sync_path_to_cursor(new_id);
            self.move_cursor_to_span(new_id);
        }
        KeyResult::Consumed
    }

    fn move_cursor_down(&mut self, rows: &[FlameRow]) -> KeyResult {
        self.push_undo();
        let max = rows.len().saturating_sub(1);
        // Find the target span id from the pre-move row list.
        let mut target_id = None;
        let mut candidate = self.cursor;
        while candidate < max {
            candidate += 1;
            if !is_legend_row(rows, candidate) {
                target_id = span_id_at(rows, candidate);
                break;
            }
        }
        if let Some(new_id) = target_id {
            // Sync legend and path first, then reposition cursor.
            if self.selected_for_legend.is_some() {
                self.selected_for_legend = Some(new_id);
            }
            self.sync_path_to_cursor(new_id);
            self.move_cursor_to_span(new_id);
        }
        KeyResult::Consumed
    }

    fn expand_or_descend(&mut self) -> KeyResult {
        self.push_undo();
        let rows = self.visible_rows();
        let span_id = match span_id_at(&rows, self.cursor) {
            Some(id) => id,
            None => return KeyResult::Consumed,
        };

        // Find first child of current node.
        let first_child_id = find_node(&self.root, span_id)
            .filter(|n| !n.children.is_empty())
            .and_then(|n| n.children.first().map(|c| c.id));

        let first_child_id = match first_child_id {
            Some(id) => id,
            None => return KeyResult::Consumed,
        };

        // Start expand animation for the current leaf (about to become ancestor).
        if !self.is_expanded(span_id) {
            self.start_expand(span_id);
        }

        // Push the child onto the path, making the current node expanded.
        self.path = ancestor_path(&self.root, first_child_id);

        // Sync legend first (changes visible rows), then position cursor.
        if self.selected_for_legend.is_some() {
            self.selected_for_legend = Some(first_child_id);
        }
        self.move_cursor_to_span(first_child_id);
        KeyResult::Consumed
    }

    fn collapse_or_ascend(&mut self) -> KeyResult {
        // If cursor is on the focus node, `h` unfocuses to the focus node's parent.
        if let Some(focus_id) = self.focus {
            let rows = self.visible_rows();
            if let Some(cursor_id) = span_id_at(&rows, self.cursor)
                && cursor_id == focus_id
            {
                return self.unfocus_to_parent();
            }
        }

        // If path has only one element (root), no-op.
        if self.path.len() <= 1 {
            return KeyResult::Consumed;
        }
        self.push_undo();

        // The parent is the second-to-last element in the path.
        let parent_id = self.path[self.path.len() - 2];

        // Start collapse animation for the parent (it's about to stop being expanded).
        self.start_collapse(parent_id);

        // Pop the leaf, making parent the new leaf (no longer expanded).
        self.path.truncate(self.path.len() - 1);

        // Sync legend first (changes visible rows), then position cursor.
        if self.selected_for_legend.is_some() {
            self.selected_for_legend = Some(parent_id);
        }
        self.move_cursor_to_span(parent_id);
        KeyResult::Consumed
    }

    fn toggle_legend(&mut self, rows: &[FlameRow]) -> KeyResult {
        if let Some(span_id) = span_id_at(rows, self.cursor) {
            if self.selected_for_legend == Some(span_id) {
                self.selected_for_legend = None;
            } else {
                self.selected_for_legend = Some(span_id);
            }
        }
        KeyResult::Consumed
    }

    /// Sync the path to match the new cursor position after j/k movement.
    ///
    /// Computes the ancestor path from root to the new span, starts collapse
    /// animations for nodes leaving the path and expand animations for nodes
    /// entering the path.
    fn sync_path_to_cursor(&mut self, new_span_id: SpanId) {
        let new_path = ancestor_path(&self.root, new_span_id);
        if new_path.is_empty() {
            return;
        }
        self.transition_animations(&new_path);
        self.path = new_path;
    }

    fn move_cursor_to_span(&mut self, target_id: SpanId) {
        let new_rows = self.visible_rows();
        if let Some(idx) = new_rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::Span { span_id, .. } if span_id == target_id))
        {
            self.cursor = idx;
        }
        self.ensure_cursor_visible();
    }

    /// Reposition cursor to the path's leaf after collapse animations finish.
    fn reposition_cursor_to_path_leaf(&mut self) {
        if let Some(&leaf_id) = self.path.last() {
            let rows = self.visible_rows();
            if let Some(idx) = rows
                .iter()
                .position(|r| matches!(r.kind, RowKind::Span { span_id, .. } if span_id == leaf_id))
            {
                self.cursor = idx;
                self.ensure_cursor_visible();
                return;
            }
            // Fallback: clamp cursor into bounds.
            if rows.is_empty() {
                self.cursor = 0;
            } else if self.cursor >= rows.len() {
                self.cursor = rows.len().saturating_sub(1);
            }
            // If we landed on a legend row, move up to the nearest span row.
            let rows = self.visible_rows();
            while self.cursor > 0 && is_legend_row(&rows, self.cursor) {
                self.cursor -= 1;
            }
            self.ensure_cursor_visible();
        }
    }
}

// --- Tree search helpers ---

fn is_legend_row(rows: &[FlameRow], index: usize) -> bool {
    matches!(
        rows.get(index),
        Some(FlameRow {
            kind: RowKind::Legend { .. }
        })
    )
}

fn span_id_at(rows: &[FlameRow], index: usize) -> Option<SpanId> {
    rows.get(index).map(|r| match r.kind {
        RowKind::Span { span_id, .. } => span_id,
        RowKind::Legend { span_id } => span_id,
    })
}

/// Find a node by `SpanId` in the tree.
pub(crate) fn find_node(root: &SpanNode, target: SpanId) -> Option<&SpanNode> {
    if root.id == target {
        return Some(root);
    }
    for child in &root.children {
        if let Some(found) = find_node(child, target) {
            return Some(found);
        }
    }
    None
}

/// Collect the path of ancestor `SpanId`s from root to the target (inclusive).
///
/// Returns an empty vec if the target is not found.
fn ancestor_path(root: &SpanNode, target: SpanId) -> Vec<SpanId> {
    let mut path = Vec::new();
    ancestor_path_walk(root, target, &mut path);
    path
}

fn ancestor_path_walk(node: &SpanNode, target: SpanId, path: &mut Vec<SpanId>) -> bool {
    path.push(node.id);
    if node.id == target {
        return true;
    }
    for child in &node.children {
        if ancestor_path_walk(child, target, path) {
            return true;
        }
    }
    path.pop();
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::flame_graph::data::{CostType, SpanNodeBuilder};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::style::Color;

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn simple_tree() -> (SpanNode, Vec<CostType>) {
        let mut b = SpanNodeBuilder::new();
        let c1 = b.leaf("big", vec![7.0]);
        let c2 = b.leaf("small", vec![3.0]);
        let root = b.span("root", vec![10.0], vec![c1, c2]);
        let cost_types = vec![CostType {
            name: "cpu",
            color: Color::Red,
        }];
        (root, cost_types)
    }

    #[test]
    fn initial_state_has_cursor_at_zero() {
        let (root, ct) = simple_tree();
        let fg = FlameGraph::new(root.clone(), ct);
        assert_eq!(fg.cursor, 0);
        assert!(!fg.is_expanded(root.id));
        assert!(fg.selected_for_legend.is_some());
    }

    #[test]
    fn right_expands_and_descends() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);
        let result = fg.handle_key(&make_key(KeyCode::Right));
        assert_eq!(result, KeyResult::Consumed);
        assert!(fg.is_expanded(fg.root.id));
        // Cursor descends to first child.
        assert_eq!(fg.cursor, 1);
    }

    #[test]
    fn left_collapses_and_ascends() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);
        fg.handle_key(&make_key(KeyCode::Right)); // expand + descend
        assert!(fg.is_expanded(fg.root.id));
        assert_eq!(fg.cursor, 1); // at first child

        fg.handle_key(&make_key(KeyCode::Left)); // collapse + ascend
        // Animation starts -- collapse animation on root.
        assert!(fg.animations.contains_key(&fg.root.id));
        // Cursor ascends back to root.
        assert_eq!(fg.cursor, 0);
    }

    #[test]
    fn down_moves_to_next_sibling() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);
        fg.handle_key(&make_key(KeyCode::Right)); // expand root + descend
        // Tick to complete animation.
        for _ in 0..32 {
            fg.tick(Duration::from_millis(16));
        }
        // Cursor is at first child (1). Down moves to sibling.
        assert_eq!(fg.cursor, 1);
        let path_before = fg.path.clone();
        fg.handle_key(&make_key(KeyCode::Down));
        // Path-first ordering: the new path child moves to position 1,
        // so cursor stays at 1 pointing to the new sibling.
        assert_ne!(fg.path, path_before, "path should change to new sibling");
    }

    #[test]
    fn ctrl_key_is_ignored() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);
        let key = KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert_eq!(fg.handle_key(&key), KeyResult::Ignored);
    }

    #[test]
    fn enter_toggles_legend() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);
        assert!(fg.selected_for_legend.is_some()); // starts on
        fg.handle_key(&make_key(KeyCode::Enter));
        assert!(fg.selected_for_legend.is_none()); // toggled off
        fg.handle_key(&make_key(KeyCode::Enter));
        assert!(fg.selected_for_legend.is_some()); // toggled on again
    }

    #[test]
    fn tick_converges_animations() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);
        fg.handle_key(&make_key(KeyCode::Right)); // expand
        assert!(!fg.animations.is_empty());

        for _ in 0..96 {
            fg.tick(Duration::from_millis(16));
        }
        // Animation should have completed and been removed.
        assert!(fg.animations.is_empty());
    }

    #[test]
    fn needs_idle_render_tracks_animation_presence() {
        let (root, ct) = simple_tree();
        let mut fg = FlameGraph::new(root, ct);

        assert!(!fg.needs_idle_render());

        fg.handle_key(&make_key(KeyCode::Right));
        assert!(fg.needs_idle_render());

        for _ in 0..96 {
            fg.tick(Duration::from_millis(16));
        }

        assert!(!fg.needs_idle_render());
    }

    #[test]
    fn tick_is_deterministic() {
        let run = || {
            let (root, ct) = simple_tree();
            let mut fg = FlameGraph::new(root, ct);
            fg.handle_key(&make_key(KeyCode::Right));
            for _ in 0..10 {
                fg.tick(Duration::from_millis(16));
            }
            fg.animations
                .values()
                .map(|a| format!("{:.6}", a.value))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn ancestor_path_returns_root_to_target() {
        let mut b = SpanNodeBuilder::new();
        let grandchild = b.leaf("gc", vec![1.0]);
        let child = b.span("child", vec![2.0], vec![grandchild.clone()]);
        let root = b.span("root", vec![3.0], vec![child.clone()]);

        let path = ancestor_path(&root, grandchild.id);
        assert_eq!(path, vec![root.id, child.id, grandchild.id]);
    }

    #[test]
    fn ancestor_path_root_only() {
        let mut b = SpanNodeBuilder::new();
        let root = b.leaf("root", vec![1.0]);
        let path = ancestor_path(&root, root.id);
        assert_eq!(path, vec![root.id]);
    }

    #[test]
    fn ancestor_path_not_found_is_empty() {
        let mut b = SpanNodeBuilder::new();
        let root = b.leaf("root", vec![1.0]);
        let path = ancestor_path(&root, SpanId(999));
        assert!(path.is_empty());
    }
}

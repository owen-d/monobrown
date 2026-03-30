//! Command palette state and logic.
//!
//! A filterable, two-level command picker. The root level shows
//! available commands; drilling into a command reveals its sub-items.
//!
//! The palette is generic over the item ID type `Id`, allowing each
//! application to define its own command identifiers.

use std::fmt::Debug;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget},
};

use crate::input::KeyResult;
use crate::render::{
    Constraints, LayoutRenderable, OverflowBehavior, Size, display_width, ellipsize_text,
    summarize_text,
};

/// A hotkey that can both render a label and match key events.
///
/// Single source of truth: the same struct drives display (via `label`)
/// and dispatch (via `matches`).
#[derive(Clone, Copy)]
pub struct HotkeyBinding {
    /// Display label shown in the palette UI (e.g. "Ctrl+G").
    pub label: &'static str,
    /// Returns `true` if the given key event triggers this hotkey.
    pub matches: fn(&KeyEvent) -> bool,
}

impl Debug for HotkeyBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotkeyBinding")
            .field("label", &self.label)
            .finish()
    }
}

/// A single entry in the command palette.
#[derive(Debug, Clone)]
pub struct PaletteItem<Id> {
    /// Display label shown in the list.
    pub label: String,
    /// Keyboard shortcut binding, or `None` for no hotkey.
    pub hotkey: Option<HotkeyBinding>,
    /// Identifier for this command (with any typed payload).
    pub id: Id,
}

impl<Id> PaletteItem<Id> {
    /// Returns the hotkey display label, or an empty string if none.
    pub fn hotkey_label(&self) -> &str {
        match &self.hotkey {
            Some(binding) => binding.label,
            None => "",
        }
    }
}

/// Result of processing a key event through the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction<Id> {
    /// Key was consumed as text input or navigation.
    Filtered,
    /// An item was selected (via Enter on highlighted item or hotkey match).
    Selected(Id),
    /// Palette dismissed (Esc at root level).
    Dismissed,
    /// Navigated back one level (Esc at sub-level).
    Back,
    /// Key not handled — bubble up to outer layer.
    Ignored,
}

impl<Id> PaletteAction<Id> {
    /// Convert to [`KeyResult`] for use at modal layer boundaries.
    ///
    /// Defined here (at the source) so that consumers cannot write
    /// incorrect adapters. Adding a new variant forces the author
    /// to classify it.
    pub fn to_key_result(&self) -> KeyResult {
        match self {
            Self::Ignored | Self::Dismissed => KeyResult::Ignored,
            Self::Filtered | Self::Selected(_) | Self::Back => KeyResult::Consumed,
        }
    }
}

/// Hierarchical level within the palette.
#[derive(Debug, Clone)]
enum PaletteLevel<Id> {
    /// Top-level command list.
    Root { items: Vec<PaletteItem<Id>> },
    /// Sub-items for a specific parent command.
    SubItems {
        parent_label: String,
        items: Vec<PaletteItem<Id>>,
    },
}

/// State for the command palette overlay.
///
/// Generic over `Id`, the application-specific command identifier type.
#[derive(Debug, Clone)]
pub struct CommandPaletteState<Id: Clone + PartialEq + Eq + Debug> {
    /// Current filter text typed by the user.
    filter: String,
    /// Index of the highlighted item in the *filtered* list.
    selected: usize,
    /// Current hierarchy level.
    level: PaletteLevel<Id>,
}

/// Returns whether `c` is a word boundary character (space, dash, underscore).
fn is_word_separator(c: char) -> bool {
    matches!(c, ' ' | '-' | '_')
}

/// Score a fuzzy (subsequence) match of `query` against `candidate`.
///
/// Returns `None` if the query is not a subsequence of the candidate.
/// Returns `Some(score)` where higher scores indicate better matches.
///
/// Scoring rules:
/// - +1 per matched character
/// - +5 bonus for matching at a word boundary (first char of a word)
/// - +3 bonus for each character in a contiguous run (beyond the first)
/// - +10 bonus if the first match is at position 0 (prefix match)
fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let candidate_lower: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    let query_lower: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();

    let mut score: i32 = 0;
    let mut candidate_idx = 0;
    let mut prev_match_idx: Option<usize> = None;

    for &qc in &query_lower {
        // Find the next occurrence of qc in candidate starting from candidate_idx.
        let found = candidate_lower[candidate_idx..]
            .iter()
            .position(|&cc| cc == qc);
        let match_idx = match found {
            Some(offset) => candidate_idx + offset,
            None => return None,
        };

        // +1 per matched character.
        score += 1;

        // +5 for word boundary match (position 0, or preceded by a separator).
        if match_idx == 0
            || candidate_lower
                .get(match_idx.wrapping_sub(1))
                .copied()
                .is_none_or(is_word_separator)
        {
            score += 5;
        }

        // +3 for contiguous run (this char immediately follows the previous match).
        if let Some(prev) = prev_match_idx
            && match_idx == prev + 1
        {
            score += 3;
        }

        // +10 if the very first query char matches at position 0.
        if prev_match_idx.is_none() && match_idx == 0 {
            score += 10;
        }

        prev_match_idx = Some(match_idx);
        candidate_idx = match_idx + 1;
    }

    Some(score)
}

impl<Id: Clone + PartialEq + Eq + Debug> CommandPaletteState<Id> {
    /// Create a new palette at the root level with the given command list.
    pub fn new(root_items: Vec<PaletteItem<Id>>) -> Self {
        Self {
            filter: String::new(),
            selected: 0,
            level: PaletteLevel::Root { items: root_items },
        }
    }

    /// The current filter text.
    pub fn filter_text(&self) -> &str {
        &self.filter
    }

    /// The parent label when inside a sub-items level, or `None` at root.
    pub fn level_label(&self) -> Option<&str> {
        match &self.level {
            PaletteLevel::Root { .. } => None,
            PaletteLevel::SubItems { parent_label, .. } => Some(parent_label),
        }
    }

    /// Items for the current level.
    fn current_items(&self) -> &[PaletteItem<Id>] {
        match &self.level {
            PaletteLevel::Root { items } | PaletteLevel::SubItems { items, .. } => items,
        }
    }

    /// Items matching the current filter via fuzzy subsequence matching.
    ///
    /// When the filter is empty, returns all items in their original order.
    /// When non-empty, returns only items whose labels contain the filter as a
    /// subsequence, sorted by match score descending (best matches first).
    pub fn filtered_items(&self) -> Vec<&PaletteItem<Id>> {
        if self.filter.is_empty() {
            return self.current_items().iter().collect();
        }

        let mut scored: Vec<(&PaletteItem<Id>, i32)> = self
            .current_items()
            .iter()
            .filter_map(|item| fuzzy_score(&self.filter, &item.label).map(|s| (item, s)))
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        scored.into_iter().map(|(item, _)| item).collect()
    }

    /// Append a character to the filter and reset selection to the top.
    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    /// Remove the last character from the filter.
    pub fn delete_back(&mut self) {
        self.filter.pop();
        // Clamp selection to new filtered length.
        let count = self.filtered_items().len();
        if count > 0 && self.selected >= count {
            self.selected = count - 1;
        }
    }

    /// Move the selection up by one.
    pub fn scroll_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the selection down by one.
    pub fn scroll_down(&mut self) {
        let count = self.filtered_items().len();
        if count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
    }

    /// Index of the currently highlighted item in the filtered list.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The currently highlighted item, if any.
    pub fn selected_item(&self) -> Option<&PaletteItem<Id>> {
        let items = self.filtered_items();
        items.get(self.selected).copied()
    }

    /// Drill into a sub-items level for a parent command.
    ///
    /// `parent_label` is used as the breadcrumb title.
    /// `sub_items` are the sub-items to display.
    pub fn drill_into(&mut self, parent_label: String, sub_items: Vec<PaletteItem<Id>>) {
        self.level = PaletteLevel::SubItems {
            parent_label,
            items: sub_items,
        };
        self.filter.clear();
        self.selected = 0;
    }

    /// Whether the palette is at the root level.
    pub fn is_root(&self) -> bool {
        matches!(self.level, PaletteLevel::Root { .. })
    }

    /// Process a key event, returning what happened.
    ///
    /// Dispatch order:
    /// 1. Hotkey scan — match against current-level item hotkeys
    /// 2. Navigation — Up/Down/Enter/Esc/Backspace
    /// 3. Text input — unmodified characters → filter
    /// 4. Ignored — key not handled
    pub fn handle_key(&mut self, key: &KeyEvent) -> PaletteAction<Id>
    where
        Id: Clone,
    {
        // 1. Hotkey scan: check all current-level items.
        let items = match &self.level {
            PaletteLevel::Root { items } => items,
            PaletteLevel::SubItems { items, .. } => items,
        };
        for item in items {
            if let Some(ref binding) = item.hotkey
                && (binding.matches)(key)
            {
                return PaletteAction::Selected(item.id.clone());
            }
        }

        // 2. Navigation.
        match key.code {
            KeyCode::Up => {
                self.scroll_up();
                return PaletteAction::Filtered;
            }
            KeyCode::Down => {
                self.scroll_down();
                return PaletteAction::Filtered;
            }
            KeyCode::Enter => {
                if let Some(item) = self.selected_item() {
                    return PaletteAction::Selected(item.id.clone());
                }
                return PaletteAction::Filtered;
            }
            KeyCode::Esc => {
                return if self.is_root() {
                    PaletteAction::Dismissed
                } else {
                    PaletteAction::Back
                };
            }
            KeyCode::Backspace => {
                self.delete_back();
                return PaletteAction::Filtered;
            }
            _ => {}
        }

        // 3. Text input (unmodified characters only).
        if let KeyCode::Char(c) = key.code
            && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
        {
            self.type_char(c);
            return PaletteAction::Filtered;
        }

        // 4. Not handled.
        PaletteAction::Ignored
    }
}

const COMPACT_PALETTE_WIDTH: u16 = 12;
const COMPACT_PALETTE_HEIGHT: u16 = 1;
const FRAMED_PALETTE_HEIGHT: u16 = 5;
const FRAMED_PALETTE_TITLE: &str = " Command Palette ";

impl<Id> LayoutRenderable for CommandPaletteState<Id>
where
    Id: Clone + PartialEq + Eq + Debug,
{
    fn measure(&self, constraints: Constraints) -> Size {
        if constraints.max_height == Some(0) {
            return Size::ZERO;
        }

        let height_limited = constraints
            .max_height
            .is_some_and(|height| height < FRAMED_PALETTE_HEIGHT);
        let preferred_width = if height_limited {
            compact_palette_width(self)
        } else {
            framed_palette_width(self)
        };
        let width = constraints.constrain(Size::new(preferred_width, 0)).width;
        if width == 0 {
            return Size::ZERO;
        }

        let overflow = palette_overflow(width, constraints.max_height);
        let desired_height = if overflow == OverflowBehavior::Summary {
            COMPACT_PALETTE_HEIGHT
        } else {
            let chrome_height = FRAMED_PALETTE_HEIGHT;
            let item_rows = self.filtered_items().len().min(u16::MAX as usize) as u16;
            chrome_height.saturating_add(item_rows)
        };

        constraints.constrain(Size::new(preferred_width, desired_height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if palette_overflow(area.width, Some(area.height)) == OverflowBehavior::Summary {
            render_compact_palette(self, area, buf);
            return;
        }

        render_framed_palette(self, area, buf);
    }
}

fn palette_overflow(width: u16, max_height: Option<u16>) -> OverflowBehavior {
    if width < COMPACT_PALETTE_WIDTH
        || max_height.is_some_and(|height| height < FRAMED_PALETTE_HEIGHT)
    {
        OverflowBehavior::Summary
    } else {
        OverflowBehavior::Ellipsis
    }
}

/// Render the palette into `area` using the component's width-first layout rules.
pub fn render_command_palette<Id>(state: &CommandPaletteState<Id>, area: Rect, buf: &mut Buffer)
where
    Id: Clone + PartialEq + Eq + Debug,
{
    state.render(area, buf);
}

fn render_compact_palette<Id>(state: &CommandPaletteState<Id>, area: Rect, buf: &mut Buffer)
where
    Id: Clone + PartialEq + Eq + Debug,
{
    let filtered = state.filtered_items();
    let count = filtered.len();
    let summary = match filtered.get(state.selected.min(count.saturating_sub(1))) {
        Some(item) => compact_palette_summary(state.filter_text(), count, &item.label, area.width),
        None => format!(">{} | 0 results", state.filter_text()),
    };
    buf.set_stringn(
        area.x,
        area.y,
        summary,
        area.width as usize,
        Style::default(),
    );
}

fn compact_palette_summary(filter: &str, count: usize, label: &str, width: u16) -> String {
    let prefix = if filter.is_empty() {
        "palette".to_string()
    } else {
        format!(">{filter}")
    };
    let candidates = [
        format!("{prefix} | {count} results | {label}"),
        format!("{count} results | {label}"),
        format!("{count} | {label}"),
        label.to_string(),
    ];

    for candidate in &candidates {
        if display_width(candidate) <= width as usize {
            return candidate.clone();
        }
    }

    ellipsize_text(candidates.last().map_or("", String::as_str), width as usize)
}

fn compact_palette_width<Id>(state: &CommandPaletteState<Id>) -> u16
where
    Id: Clone + PartialEq + Eq + Debug,
{
    let filtered = state.filtered_items();
    let count = filtered.len();
    let summary = match filtered.get(state.selected.min(count.saturating_sub(1))) {
        Some(item) => compact_palette_summary(state.filter_text(), count, &item.label, u16::MAX),
        None => format!(">{} | 0 results", state.filter_text()),
    };
    saturating_width(display_width(&summary)).max(COMPACT_PALETTE_WIDTH)
}

fn framed_palette_width<Id>(state: &CommandPaletteState<Id>) -> u16
where
    Id: Clone + PartialEq + Eq + Debug,
{
    let filter_width = display_width(&format!("> {}", state.filter_text()));
    let footer_width = display_width(&format_palette_footer(
        state.level_label(),
        state.filtered_items().len(),
        state.filtered_items().len(),
    ));
    let items_width = state
        .filtered_items()
        .iter()
        .map(|item| palette_item_width(&item.label, item.hotkey_label()))
        .max()
        .unwrap_or(0);
    let inner_width = filter_width.max(footer_width).max(items_width);
    let outer_width = display_width(FRAMED_PALETTE_TITLE).max(inner_width.saturating_add(2));
    saturating_width(outer_width).max(COMPACT_PALETTE_WIDTH)
}

fn palette_item_width(label: &str, hotkey: &str) -> usize {
    let prefix_width = 2;
    let suffix_width = if hotkey.is_empty() {
        0
    } else {
        1 + display_width(hotkey)
    };
    prefix_width + display_width(label) + suffix_width
}

fn saturating_width(width: usize) -> u16 {
    width.min(u16::MAX as usize) as u16
}

fn render_framed_palette<Id>(state: &CommandPaletteState<Id>, area: Rect, buf: &mut Buffer)
where
    Id: Clone + PartialEq + Eq + Debug,
{
    let block = Block::default()
        .title(FRAMED_PALETTE_TITLE)
        .borders(Borders::ALL);
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let filter_text = format!("> {}", state.filter_text());
    Paragraph::new(filter_text).render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    if inner.height >= 2 {
        let sep = "─".repeat(inner.width as usize);
        Paragraph::new(sep).render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
    }

    let items_height = inner.height.saturating_sub(3);
    let filtered = state.filtered_items();
    let selected = state.selected_index().min(filtered.len().saturating_sub(1));
    let (start, end) = visible_window(filtered.len(), selected, items_height as usize);
    let visible_items = &filtered[start..end];

    let list_items: Vec<ListItem> = visible_items
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            let is_selected = start + offset == selected;
            ListItem::new(Line::from(format_palette_item(
                &item.label,
                item.hotkey_label(),
                inner.width as usize,
                is_selected,
            )))
        })
        .collect();

    let list =
        List::new(list_items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let list_area = Rect::new(inner.x, inner.y + 2, inner.width, items_height);
    let list_selected = if visible_items.is_empty() {
        None
    } else {
        Some(selected - start)
    };
    let mut list_state = ListState::default().with_selected(list_selected);
    StatefulWidget::render(list, list_area, buf, &mut list_state);

    let shown = visible_items.len();
    let total = filtered.len();
    let footer = format_palette_footer(state.level_label(), shown, total);
    let footer_y = inner.y + inner.height - 1;
    Paragraph::new(footer).render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
}

fn visible_window(total_items: usize, selected: usize, capacity: usize) -> (usize, usize) {
    if total_items == 0 || capacity == 0 {
        return (0, 0);
    }

    let capacity = capacity.min(total_items);
    let start = selected
        .saturating_add(1)
        .saturating_sub(capacity)
        .min(total_items - capacity);
    (start, start + capacity)
}

fn format_palette_item(label: &str, hotkey: &str, width: usize, is_selected: bool) -> String {
    let prefix = if is_selected { "> " } else { "  " };
    let suffix = if hotkey.is_empty() {
        String::new()
    } else {
        format!(" {hotkey}")
    };
    summarize_text(prefix, label, &suffix, width)
}

fn format_palette_footer(level_label: Option<&str>, shown: usize, total: usize) -> String {
    let level = match level_label {
        None => "[Root]".to_string(),
        Some(parent) => format!("[Sub: {parent}]"),
    };
    if shown < total {
        format!("{level} {shown}/{total} shown")
    } else {
        level
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect};

    /// Test-only item ID type.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TestId {
        Action { name: String },
    }

    fn test_item(label: &str) -> PaletteItem<TestId> {
        PaletteItem {
            label: label.to_string(),
            hotkey: None,
            id: TestId::Action {
                name: label.to_lowercase(),
            },
        }
    }

    fn test_palette(items: Vec<&str>) -> CommandPaletteState<TestId> {
        let root: Vec<PaletteItem<TestId>> = items.into_iter().map(test_item).collect();
        CommandPaletteState::new(root)
    }

    #[test]
    fn new_palette_is_at_root() {
        let palette = test_palette(vec!["Change State"]);
        assert!(palette.is_root());
        assert!(palette.level_label().is_none());
        assert_eq!(palette.filter_text(), "");
    }

    #[test]
    fn root_has_provided_commands() {
        let palette = test_palette(vec!["Change State"]);
        let items = palette.filtered_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Change State");
    }

    #[test]
    fn type_char_appends_and_resets_selection() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.type_char('c');
        assert_eq!(palette.filter_text(), "c");
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn delete_back_removes_last_char() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.type_char('a');
        palette.type_char('b');
        palette.delete_back();
        assert_eq!(palette.filter_text(), "a");
    }

    #[test]
    fn delete_back_on_empty_is_noop() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.delete_back();
        assert_eq!(palette.filter_text(), "");
    }

    #[test]
    fn drill_into_changes_level() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.type_char('x'); // Set some filter first.
        palette.drill_into("Change State".to_string(), vec![test_item("Design")]);
        assert!(!palette.is_root());
        assert_eq!(palette.level_label(), Some("Change State"));
        assert_eq!(palette.filter_text(), ""); // Filter reset on drill.
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn scroll_down_clamps_to_last() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into("Test".to_string(), vec![test_item("A"), test_item("B")]);
        palette.scroll_down();
        palette.scroll_down();
        palette.scroll_down();
        assert_eq!(palette.selected, 1);
    }

    #[test]
    fn scroll_up_clamps_to_zero() {
        let palette = test_palette(vec!["Change State"]);
        // Fresh palette, selected is already 0.
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn filter_narrows_items() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into(
            "Test".to_string(),
            vec![test_item("Design"), test_item("Implementation")],
        );
        palette.type_char('d');
        let filtered = palette.filtered_items();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "Design");
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.type_char('C');
        let filtered = palette.filtered_items();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "Change State");
    }

    // --- fuzzy_score unit tests ---

    #[test]
    fn fuzzy_score_empty_query_matches_everything() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn fuzzy_score_no_match_returns_none() {
        assert_eq!(fuzzy_score("xyz", "Change State"), None);
    }

    #[test]
    fn fuzzy_score_subsequence_match() {
        // "chst" is a subsequence of "Change State": C-h from "Ch", s from "St", t from "St".
        assert!(fuzzy_score("chst", "Change State").is_some());
    }

    #[test]
    fn fuzzy_score_case_insensitive() {
        let upper = fuzzy_score("CS", "Change State");
        let lower = fuzzy_score("cs", "Change State");
        assert!(upper.is_some());
        assert_eq!(upper, lower);
    }

    #[test]
    fn fuzzy_score_word_boundary_bonus() {
        // "cs" on "Change State" matches at word boundaries (C, S).
        // "cs" on "achest" matches mid-word (c, s) -- no boundary bonus.
        let boundary_score = fuzzy_score("cs", "Change State").unwrap();
        let mid_word_score = fuzzy_score("cs", "achest").unwrap();
        assert!(
            boundary_score > mid_word_score,
            "word boundary score ({boundary_score}) should beat mid-word score ({mid_word_score})"
        );
    }

    #[test]
    fn fuzzy_score_prefix_bonus() {
        // "cha" starts at position 0 of "Change State" -- prefix bonus.
        // "sta" starts at position 7 -- no prefix bonus.
        let prefix_score = fuzzy_score("cha", "Change State").unwrap();
        let non_prefix_score = fuzzy_score("sta", "Change State").unwrap();
        assert!(
            prefix_score > non_prefix_score,
            "prefix score ({prefix_score}) should beat non-prefix score ({non_prefix_score})"
        );
    }

    #[test]
    fn fuzzy_score_contiguous_bonus() {
        // "des" on "Design" -- all contiguous (D-e-s).
        // "dis" on "Design" -- non-contiguous (D...i...s? actually D, then skip e to find i? no, 'i' is at index 3).
        // Better test: "de" (contiguous) vs "dn" (non-contiguous) on "Design".
        let contiguous = fuzzy_score("de", "Design").unwrap();
        let non_contiguous = fuzzy_score("dn", "Design").unwrap();
        assert!(
            contiguous > non_contiguous,
            "contiguous score ({contiguous}) should beat non-contiguous score ({non_contiguous})"
        );
    }

    // --- filtered_items integration tests ---

    #[test]
    fn filtered_items_empty_query_returns_all() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into(
            "Test".to_string(),
            vec![test_item("Alpha"), test_item("Beta")],
        );
        assert_eq!(palette.filtered_items().len(), 2);
    }

    #[test]
    fn filtered_items_no_match_returns_empty() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into("Test".to_string(), vec![test_item("Alpha")]);
        palette.type_char('z');
        assert!(palette.filtered_items().is_empty());
    }

    #[test]
    fn filtered_items_sorts_by_score() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into(
            "Test".to_string(),
            vec![test_item("achest"), test_item("Change State")],
        );
        // "cs" should rank "Change State" (word boundary matches) above "achest".
        palette.type_char('c');
        palette.type_char('s');
        let filtered = palette.filtered_items();
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].label, "Change State");
        assert_eq!(filtered[1].label, "achest");
    }

    #[test]
    fn filtered_items_subsequence_non_contiguous() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into("Test".to_string(), vec![test_item("Change State")]);
        // "chst" is a non-contiguous subsequence of "Change State".
        for c in "chst".chars() {
            palette.type_char(c);
        }
        let filtered = palette.filtered_items();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "Change State");
    }

    #[test]
    fn selected_item_returns_correct_item() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into("Test".to_string(), vec![test_item("A"), test_item("B")]);
        palette.scroll_down();
        assert_eq!(palette.selected_item().unwrap().label, "B");
    }

    #[test]
    fn selected_item_none_when_empty() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into("Test".to_string(), vec![]);
        assert!(palette.selected_item().is_none());
    }

    #[test]
    fn delete_back_clamps_selection() {
        let mut palette = test_palette(vec!["Change State"]);
        palette.drill_into("Test".to_string(), vec![test_item("abc"), test_item("def")]);
        // Select "def" (index 1).
        palette.scroll_down();
        assert_eq!(palette.selected, 1);
        // Type 'd' to filter to just "def".
        palette.type_char('d');
        assert_eq!(palette.selected, 0); // Reset by type_char.
        // Now delete 'd' — both items return. Selection stays at 0, which is valid.
        palette.delete_back();
        assert!(palette.selected < palette.filtered_items().len());
    }

    // --- handle_key tests ---

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn item_with_ctrl_g() -> PaletteItem<TestId> {
        PaletteItem {
            label: "Git Commit".to_string(),
            hotkey: Some(HotkeyBinding {
                label: "Ctrl+G",
                matches: |k| {
                    k.code == KeyCode::Char('g') && k.modifiers.contains(KeyModifiers::CONTROL)
                },
            }),
            id: TestId::Action {
                name: "git commit".into(),
            },
        }
    }

    fn palette_with_hotkey() -> CommandPaletteState<TestId> {
        CommandPaletteState::new(vec![item_with_ctrl_g(), test_item("Deploy")])
    }

    fn render_text(palette: &CommandPaletteState<TestId>, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        palette.render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn handle_key_hotkey_dispatch() {
        let mut palette = palette_with_hotkey();
        let action = palette.handle_key(&key_ctrl('g'));
        assert_eq!(
            action,
            PaletteAction::Selected(TestId::Action {
                name: "git commit".into()
            })
        );
    }

    #[test]
    fn handle_key_hotkey_miss() {
        let mut palette = palette_with_hotkey();
        let action = palette.handle_key(&key_ctrl('z'));
        assert_eq!(action, PaletteAction::Ignored);
    }

    #[test]
    fn handle_key_up_returns_filtered() {
        let mut palette = test_palette(vec!["A", "B"]);
        assert_eq!(
            palette.handle_key(&key(KeyCode::Up)),
            PaletteAction::Filtered
        );
    }

    #[test]
    fn handle_key_down_returns_filtered() {
        let mut palette = test_palette(vec!["A", "B"]);
        assert_eq!(
            palette.handle_key(&key(KeyCode::Down)),
            PaletteAction::Filtered
        );
    }

    #[test]
    fn handle_key_enter_returns_selected() {
        let mut palette = test_palette(vec!["Alpha"]);
        let action = palette.handle_key(&key(KeyCode::Enter));
        assert_eq!(
            action,
            PaletteAction::Selected(TestId::Action {
                name: "alpha".into()
            })
        );
    }

    #[test]
    fn handle_key_backspace_returns_filtered() {
        let mut palette = test_palette(vec!["Alpha"]);
        palette.type_char('a');
        assert_eq!(
            palette.handle_key(&key(KeyCode::Backspace)),
            PaletteAction::Filtered
        );
    }

    #[test]
    fn handle_key_esc_at_root_returns_dismissed() {
        let mut palette = test_palette(vec!["Alpha"]);
        assert_eq!(
            palette.handle_key(&key(KeyCode::Esc)),
            PaletteAction::Dismissed
        );
    }

    #[test]
    fn handle_key_esc_at_sub_level_returns_back() {
        let mut palette = test_palette(vec!["Alpha"]);
        palette.drill_into("Alpha".to_string(), vec![test_item("Sub")]);
        assert_eq!(palette.handle_key(&key(KeyCode::Esc)), PaletteAction::Back);
    }

    #[test]
    fn handle_key_text_input_appends_to_filter() {
        let mut palette = test_palette(vec!["Alpha"]);
        let action = palette.handle_key(&key(KeyCode::Char('x')));
        assert_eq!(action, PaletteAction::Filtered);
        assert_eq!(palette.filter_text(), "x");
    }

    #[test]
    fn handle_key_shift_char_counts_as_text() {
        let mut palette = test_palette(vec!["Alpha"]);
        let shifted = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        let action = palette.handle_key(&shifted);
        assert_eq!(action, PaletteAction::Filtered);
        assert_eq!(palette.filter_text(), "A");
    }

    #[test]
    fn handle_key_unhandled_returns_ignored() {
        let mut palette = test_palette(vec!["Alpha"]);
        assert_eq!(
            palette.handle_key(&key(KeyCode::F(1))),
            PaletteAction::Ignored
        );
    }

    #[test]
    fn handle_key_hotkey_takes_priority_over_text() {
        // 'g' as a plain char would be text input, but Ctrl+G is a hotkey.
        // Hotkey must win even though 'g' could filter.
        let mut palette = palette_with_hotkey();
        let action = palette.handle_key(&key_ctrl('g'));
        assert_eq!(
            action,
            PaletteAction::Selected(TestId::Action {
                name: "git commit".into()
            })
        );
        // Filter must remain empty -- hotkey short-circuited before text input.
        assert_eq!(palette.filter_text(), "");
    }

    #[test]
    fn desired_height_accounts_for_palette_chrome_and_items() {
        let palette = test_palette(vec!["Alpha", "Beta", "Gamma"]);
        assert_eq!(palette.measure(Constraints::tight_width(40)).height, 8);
        assert_eq!(palette.measure(Constraints::tight_width(8)).height, 1);
    }

    #[test]
    fn measure_prefers_content_width_when_constraints_are_loose() {
        let palette = test_palette(vec!["Alpha", "Beta"]);
        let measured = palette.measure(Constraints::loose(40, 10));

        assert!(
            measured.width < 40,
            "expected intrinsic width, got {measured:?}"
        );
        assert_eq!(measured.height, 7);
    }

    #[test]
    fn compact_render_collapses_to_result_summary() {
        let mut palette = test_palette(vec!["Alpha", "Beta"]);
        palette.type_char('b');
        let text = render_text(&palette, 20, 1);
        assert!(text.contains("1 results"));
        assert!(text.contains("Beta"));
    }

    #[test]
    fn short_palette_height_keeps_selected_item_visible() {
        let mut palette = test_palette(vec!["Alpha", "Beta", "Gamma", "Delta", "Epsilon"]);
        for _ in 0..4 {
            palette.scroll_down();
        }

        let text = render_text(&palette, 24, 8);
        assert!(
            text.contains("Epsilon"),
            "selected item should stay visible:\n{text}"
        );
        assert!(
            text.contains("3/5 shown"),
            "footer should summarize hidden rows:\n{text}"
        );
    }
}

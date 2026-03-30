//! Standalone vim-style line editor with modal editing.
//!
//! Pure state machine: accepts `KeyEvent`, returns `EditorEffect`.
//! Editing remains unit-testable in isolation; the render adapter lives here so
//! the widget can participate in width-first layout like the rest of `mb-tui`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Modifier as TuiModifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::{OverflowBehavior, display_width, summarize_text};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Editing mode for the vim editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
}

/// Effect produced by `VimEditor::step`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEffect {
    /// Key was handled; caller should re-render.
    Consumed,
    /// User submitted the line (Enter in insert mode).
    Submit(String),
    /// User wants to leave the editor (Esc in normal mode).
    Exit,
    /// Key was not handled; caller may use it.
    Ignored,
}

/// Operator for operator-pending mode (e.g. `d`, `c`, `y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Delete,
    Change,
    Yank,
}

/// Which flavour of character-find the operator is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindKind {
    FindForward,
    FindBackward,
    TilForward,
    TilBackward,
}

/// Transient state for multi-key sequences in normal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// Waiting for a character argument to `f`.
    FindForward,
    /// Waiting for a character argument to `F`.
    FindBackward,
    /// Waiting for a character argument to `t`.
    TilForward,
    /// Waiting for a character argument to `T`.
    TilBackward,
    /// Waiting for a character to replace under cursor (`r`).
    Replace,
    /// Waiting for a motion after an operator (`d`, `c`, or `y`).
    Operator(Operator),
    /// Operator waiting for a find-char argument (e.g. `df`, `ct`).
    OperatorFind(Operator, FindKind),
    /// Operator waiting for a text object key (e.g. `i` or `a` pressed, now
    /// waiting for `w`).
    TextObject(Operator, TextObjectScope),
    /// Visual mode waiting for text object key after `i` or `a`.
    VisualTextObject(TextObjectScope),
}

/// Whether a text object is "inner" or "a" (includes surrounding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextObjectScope {
    Inner,
    A,
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// Bounded undo/redo stack of `(text, cursor)` state snapshots.
///
/// The stack stores a linear sequence of states. `position` indexes the
/// state that is currently "active". Undo goes left, redo goes right.
/// When a new mutation occurs, the current live state is appended and
/// everything to the right is discarded.
#[derive(Clone)]
struct UndoStack {
    /// History of `(text, cursor)` states.
    entries: Vec<(String, usize)>,
    /// Index of the most recent state restored by undo, or `entries.len()`
    /// when at the top (meaning the live buffer is ahead of all entries).
    position: usize,
    /// Whether the live buffer state has been saved to the stack during
    /// the current undo sequence (so redo can return to it).
    live_saved: bool,
}

/// Maximum number of undo entries retained.
const UNDO_STACK_CAPACITY: usize = 100;

impl UndoStack {
    fn new() -> Self {
        let stack = Self {
            entries: Vec::new(),
            position: 0,
            live_saved: false,
        };
        assert!(stack.position <= stack.entries.len());
        assert!(!stack.live_saved);
        stack
    }

    /// Save a pre-mutation snapshot. Truncates redo history and enforces bounds.
    fn push(&mut self, text: String, cursor: usize) {
        self.entries.truncate(self.position);
        self.entries.push((text, cursor));
        self.position = self.entries.len();
        self.live_saved = false;

        // Drop oldest entries when capacity exceeded.
        if self.entries.len() > UNDO_STACK_CAPACITY {
            let excess = self.entries.len() - UNDO_STACK_CAPACITY;
            self.entries.drain(..excess);
            self.position = self.entries.len();
        }
        assert!(self.position <= self.entries.len());
        assert!(self.entries.len() <= UNDO_STACK_CAPACITY);
    }

    /// Save the live buffer state so redo can return to it. Called once
    /// at the start of an undo sequence.
    fn save_live(&mut self, text: String, cursor: usize) {
        if !self.live_saved && self.position == self.entries.len() {
            self.entries.push((text, cursor));
            self.live_saved = true;
        }
        assert!(!self.entries.is_empty());
    }

    /// Undo: return the previous snapshot, or `None` if at bottom.
    fn undo(&mut self) -> Option<&(String, usize)> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        assert!(self.position < self.entries.len());
        Some(&self.entries[self.position])
    }

    /// Redo: return the next snapshot, or `None` if at top.
    fn redo(&mut self) -> Option<&(String, usize)> {
        if self.position + 1 >= self.entries.len() {
            return None;
        }
        self.position += 1;
        assert!(self.position < self.entries.len());
        Some(&self.entries[self.position])
    }
}

// ---------------------------------------------------------------------------
// LineBuffer
// ---------------------------------------------------------------------------

/// Single-line text buffer with a byte-offset cursor.
///
/// All public methods maintain the invariant that `cursor` sits on a char
/// boundary and `cursor <= text.len()`.
#[derive(Clone)]
struct LineBuffer {
    text: String,
    /// Byte offset into `text`. Always on a char boundary, `<= text.len()`.
    cursor: usize,
    /// Anchor byte offset for visual selection. Set when entering visual mode.
    anchor: Option<usize>,
}

impl LineBuffer {
    fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: None,
        }
    }

    // -- Queries ------------------------------------------------------------

    fn text(&self) -> &str {
        &self.text
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn len(&self) -> usize {
        self.text.len()
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Byte offset of the last char's start, or 0 if empty.
    fn last_char_offset(&self) -> usize {
        self.text
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Character at byte offset `pos`, with assertions.
    fn expect_char_at(&self, pos: usize) -> char {
        assert!(
            pos < self.len(),
            "char_at out of bounds: {pos} >= {}",
            self.len()
        );
        assert!(self.text.is_char_boundary(pos));
        let Some(ch) = self.text[pos..].chars().next() else {
            unreachable!("pos {pos} is within bounds and on a char boundary");
        };
        ch
    }

    /// Character at byte offset `pos`, if any.
    fn char_at(&self, pos: usize) -> Option<char> {
        assert!(
            self.text.is_char_boundary(pos),
            "pos {pos} is not a char boundary"
        );
        self.text[pos..].chars().next()
    }

    /// Advance `pos` to the next char boundary, or `text.len()`.
    fn next_boundary(&self, pos: usize) -> usize {
        assert!(
            pos <= self.text.len(),
            "pos {pos} out of bounds (len {})",
            self.text.len()
        );
        let mut i = pos;
        if i < self.text.len() {
            i += self.text[i..].chars().next().map_or(0, char::len_utf8);
        }
        assert!(self.text.is_char_boundary(i));
        i
    }

    /// Retreat `pos` to the previous char boundary, or 0.
    fn prev_boundary(&self, pos: usize) -> usize {
        assert!(
            pos <= self.text.len(),
            "pos {pos} out of bounds (len {})",
            self.text.len()
        );
        let result = self.text[..pos]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        assert!(self.text.is_char_boundary(result));
        result
    }

    // -- Character classes --------------------------------------------------

    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    fn is_whitespace(c: char) -> bool {
        c.is_whitespace()
    }

    fn char_class(c: char) -> CharClass {
        if Self::is_word_char(c) {
            CharClass::Word
        } else if Self::is_whitespace(c) {
            CharClass::Whitespace
        } else {
            CharClass::Punctuation
        }
    }

    // -- Motions ------------------------------------------------------------

    /// Byte offset of the next word start (vim `w`).
    fn next_word_start(&self) -> usize {
        assert!(self.text.is_char_boundary(self.cursor));
        if self.cursor >= self.len() {
            return self.cursor;
        }
        let mut pos = self.cursor;
        let start_class = Self::char_class(self.expect_char_at(pos));

        // Skip current class.
        while pos < self.len() {
            match self.char_at(pos) {
                Some(c) if Self::char_class(c) == start_class => pos = self.next_boundary(pos),
                _ => break,
            }
        }
        // Skip whitespace.
        while pos < self.len() {
            match self.char_at(pos) {
                Some(c) if Self::is_whitespace(c) => pos = self.next_boundary(pos),
                _ => break,
            }
        }
        assert!(pos <= self.len());
        assert!(self.text.is_char_boundary(pos));
        pos
    }

    /// Byte offset of the previous word start (vim `b`).
    fn prev_word_start(&self) -> usize {
        assert!(self.text.is_char_boundary(self.cursor));
        if self.cursor == 0 {
            return 0;
        }
        let mut pos = self.prev_boundary(self.cursor);

        // Skip whitespace backwards.
        while pos > 0 {
            match self.char_at(pos) {
                Some(c) if Self::is_whitespace(c) => pos = self.prev_boundary(pos),
                _ => break,
            }
        }
        if pos == 0 && self.char_at(0).is_some_and(Self::is_whitespace) {
            return 0;
        }
        let target_class = Self::char_class(self.expect_char_at(pos));

        // Skip same class backwards.
        while pos > 0 {
            let prev = self.prev_boundary(pos);
            match self.char_at(prev) {
                Some(c) if Self::char_class(c) == target_class => pos = prev,
                _ => break,
            }
        }
        assert!(pos <= self.len());
        assert!(self.text.is_char_boundary(pos));
        pos
    }

    /// Byte offset of the end of the current/next word (vim `e`).
    fn word_end(&self) -> usize {
        assert!(self.text.is_char_boundary(self.cursor));
        if self.cursor >= self.len() {
            return self.cursor;
        }
        let mut pos = self.cursor;

        // If at end of text (last char), stay.
        if self.next_boundary(pos) >= self.len() {
            return pos;
        }

        // Move at least one char forward.
        pos = self.next_boundary(pos);

        // Skip whitespace.
        while pos < self.len() {
            match self.char_at(pos) {
                Some(c) if Self::is_whitespace(c) => pos = self.next_boundary(pos),
                _ => break,
            }
        }
        if pos >= self.len() {
            return self.last_char_offset();
        }

        let target_class = Self::char_class(self.expect_char_at(pos));

        // Advance through same class.
        while pos < self.len() {
            let next = self.next_boundary(pos);
            if next >= self.len() {
                break;
            }
            match self.char_at(next) {
                Some(c) if Self::char_class(c) == target_class => pos = next,
                _ => break,
            }
        }
        assert!(pos <= self.len());
        assert!(self.text.is_char_boundary(pos));
        pos
    }

    /// First non-whitespace byte offset (vim `^`).
    fn first_non_whitespace(&self) -> usize {
        for (i, c) in self.text.char_indices() {
            if !Self::is_whitespace(c) {
                assert!(self.text.is_char_boundary(i));
                return i;
            }
        }
        self.len()
    }

    /// Find next occurrence of `target` after cursor (vim `f`).
    fn find_forward(&self, target: char) -> Option<usize> {
        assert!(self.text.is_char_boundary(self.cursor));
        let start = self.next_boundary(self.cursor);
        for (i, c) in self.text[start..].char_indices() {
            if c == target {
                let result = start + i;
                assert!(self.text.is_char_boundary(result));
                return Some(result);
            }
        }
        None
    }

    /// Find next occurrence of `target` after cursor, stop one before (vim `t`).
    fn til_forward(&self, target: char) -> Option<usize> {
        self.find_forward(target)
            .map(|pos| self.prev_boundary(pos).max(self.cursor))
    }

    /// Find previous occurrence of `target` before cursor (vim `F`).
    fn find_backward(&self, target: char) -> Option<usize> {
        assert!(self.text.is_char_boundary(self.cursor));
        for (i, c) in self.text[..self.cursor].char_indices().rev() {
            if c == target {
                assert!(self.text.is_char_boundary(i));
                return Some(i);
            }
        }
        None
    }

    /// Find previous occurrence of `target` before cursor, stop one after (vim `T`).
    fn til_backward(&self, target: char) -> Option<usize> {
        self.find_backward(target).map(|pos| {
            let after = self.next_boundary(pos);
            after.min(self.cursor)
        })
    }

    // -- Mutations ----------------------------------------------------------

    fn insert_char(&mut self, c: char) {
        assert!(self.cursor <= self.len());
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        assert!(self.cursor <= self.len());
        assert!(self.text.is_char_boundary(self.cursor));
    }

    fn backspace(&mut self) {
        assert!(self.cursor <= self.len());
        if self.cursor > 0 {
            let prev = self.prev_boundary(self.cursor);
            self.text.drain(prev..self.cursor);
            self.cursor = prev;
        }
        assert!(self.cursor <= self.len());
    }

    /// Delete the char at `cursor` (like `x` in normal mode).
    fn delete_at_cursor(&mut self) {
        assert!(self.cursor <= self.len());
        if self.cursor < self.len() {
            let next = self.next_boundary(self.cursor);
            self.text.drain(self.cursor..next);
        }
        // Clamp cursor if we deleted the last char.
        if !self.is_empty() && self.cursor >= self.len() {
            self.cursor = self.last_char_offset();
        }
        assert!(self.cursor <= self.len());
    }

    /// Move cursor left one char, clamped at 0.
    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_boundary(self.cursor);
        }
        assert!(self.text.is_char_boundary(self.cursor));
    }

    /// Move cursor right one char, clamped at len.
    fn move_right(&mut self) {
        if self.cursor < self.len() {
            self.cursor = self.next_boundary(self.cursor);
        }
        assert!(self.text.is_char_boundary(self.cursor));
    }

    /// Move cursor right, but clamp to last char offset (normal mode style).
    fn move_right_normal(&mut self) {
        if self.cursor < self.len() {
            let next = self.next_boundary(self.cursor);
            // In normal mode, cursor should not go past last char.
            if next < self.len() {
                self.cursor = next;
            }
        }
        assert!(self.text.is_char_boundary(self.cursor));
    }

    /// Clamp cursor for normal mode: must be on a char, not past last char.
    fn clamp_normal(&mut self) {
        if !self.is_empty() && self.cursor >= self.len() {
            self.cursor = self.last_char_offset();
        }
        assert!(self.text.is_char_boundary(self.cursor));
    }

    /// Delete text in `[start, end)` and position cursor at `start`.
    fn delete_range(&mut self, start: usize, end: usize) {
        assert!(start <= end, "delete_range: start {start} > end {end}");
        assert!(
            end <= self.len(),
            "delete_range: end {end} > len {}",
            self.len()
        );
        assert!(self.text.is_char_boundary(start));
        assert!(self.text.is_char_boundary(end));
        self.text.drain(start..end);
        self.cursor = start.min(self.len());
        assert!(self.text.is_char_boundary(self.cursor));
    }

    /// Replace the character under the cursor with `c`.
    fn replace_at_cursor(&mut self, c: char) {
        assert!(self.cursor < self.len(), "replace_at_cursor: cursor at end");
        assert!(self.text.is_char_boundary(self.cursor));
        let next = self.next_boundary(self.cursor);
        self.text.drain(self.cursor..next);
        self.text.insert(self.cursor, c);
        assert!(self.text.is_char_boundary(self.cursor));
    }

    /// Inner word text object: range of the word (same char class) under cursor.
    fn inner_word(&self) -> (usize, usize) {
        assert!(self.cursor <= self.len());
        if self.is_empty() {
            return (0, 0);
        }
        let pos = self.cursor.min(self.last_char_offset());
        let class = Self::char_class(self.expect_char_at(pos));

        // Scan backward to find start of same-class run.
        let mut start = pos;
        while start > 0 {
            let prev = self.prev_boundary(start);
            if Self::char_class(self.expect_char_at(prev)) == class {
                start = prev;
            } else {
                break;
            }
        }
        // Scan forward to find end of same-class run.
        let mut end = self.next_boundary(pos);
        while end < self.len() {
            if Self::char_class(self.expect_char_at(end)) == class {
                end = self.next_boundary(end);
            } else {
                break;
            }
        }
        assert!(start <= end);
        assert!(end <= self.len());
        (start, end)
    }

    /// A word text object: word under cursor plus trailing (or leading) space.
    fn a_word(&self) -> (usize, usize) {
        let (start, end) = self.inner_word();
        if start == end {
            return (start, end);
        }
        // Try trailing whitespace first.
        let mut new_end = end;
        while new_end < self.len() {
            if Self::is_whitespace(self.expect_char_at(new_end)) {
                new_end = self.next_boundary(new_end);
            } else {
                break;
            }
        }
        if new_end > end {
            return (start, new_end);
        }
        // No trailing whitespace; try leading whitespace.
        let mut new_start = start;
        while new_start > 0 {
            let prev = self.prev_boundary(new_start);
            if Self::is_whitespace(self.expect_char_at(prev)) {
                new_start = prev;
            } else {
                break;
            }
        }
        assert!(new_start <= start);
        (new_start, end)
    }

    fn take_text(&mut self) -> String {
        let taken = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.anchor = None;
        taken
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Whitespace,
    Punctuation,
}

// ---------------------------------------------------------------------------
// VimEditor
// ---------------------------------------------------------------------------

/// Modal line editor with vim keybindings.
///
/// Pure state machine: call `step()` with key events, inspect effects.
#[derive(Clone)]
pub struct VimEditor {
    buf: LineBuffer,
    mode: VimMode,
    /// Multi-key sequence state (e.g. `f` waiting for a char).
    pending: Option<Pending>,
    /// Undo/redo history of `(text, cursor)` snapshots.
    undo_stack: UndoStack,
    /// Whether we have already saved a snapshot for the current insert session.
    insert_snapshot_saved: bool,
    /// Accumulated numeric count prefix (e.g. `3` in `3w`).
    count: Option<u32>,
    /// Count saved before entering operator-pending mode, for multiplication.
    /// In vim, `2d3w` = delete 6 words (2 * 3).
    operator_count: Option<u32>,
    /// Single yank register for copy/paste.
    register: Option<String>,
}

impl Default for VimEditor {
    fn default() -> Self {
        Self {
            buf: LineBuffer::new(),
            mode: VimMode::Insert,
            pending: None,
            undo_stack: UndoStack::new(),
            insert_snapshot_saved: false,
            count: None,
            operator_count: None,
            register: None,
        }
    }
}

impl VimEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        self.buf.text()
    }

    pub fn cursor(&self) -> usize {
        self.buf.cursor()
    }

    pub fn mode(&self) -> VimMode {
        self.mode
    }

    /// Returns the visual selection range `(start, end)` if in visual mode.
    pub fn selection(&self) -> Option<(usize, usize)> {
        if self.mode == VimMode::Visual {
            let anchor = self.buf.anchor?;
            let cursor = self.buf.cursor();
            assert!(anchor <= self.buf.len());
            assert!(cursor <= self.buf.len());
            Some((anchor.min(cursor), anchor.max(cursor)))
        } else {
            None
        }
    }

    /// Reset the editor to insert mode for a new editing session.
    ///
    /// Called when the TUI enters editing mode so the user can type
    /// immediately, even if the previous session ended in normal mode.
    pub fn enter(&mut self) {
        self.mode = VimMode::Insert;
        self.pending = None;
        self.count = None;
        self.operator_count = None;
        self.buf.anchor = None;
    }

    pub fn take_text(&mut self) -> String {
        self.pending = None;
        self.mode = VimMode::Insert;
        self.count = None;
        self.operator_count = None;
        self.insert_snapshot_saved = false;
        self.buf.take_text()
    }

    /// Human-readable label for the current pending state, if any.
    ///
    /// Returns a string like "3", "d", "3d", "3df", etc. reflecting the
    /// accumulated count and/or pending operator state.
    pub fn pending_display(&self) -> Option<String> {
        let suffix = self.pending.as_ref().map(|p| match p {
            Pending::FindForward => "f",
            Pending::FindBackward => "F",
            Pending::TilForward => "t",
            Pending::TilBackward => "T",
            Pending::Replace => "r",
            Pending::Operator(Operator::Delete) => "d",
            Pending::Operator(Operator::Change) => "c",
            Pending::Operator(Operator::Yank) => "y",
            Pending::OperatorFind(Operator::Delete, FindKind::FindForward) => "df",
            Pending::OperatorFind(Operator::Delete, FindKind::FindBackward) => "dF",
            Pending::OperatorFind(Operator::Delete, FindKind::TilForward) => "dt",
            Pending::OperatorFind(Operator::Delete, FindKind::TilBackward) => "dT",
            Pending::OperatorFind(Operator::Change, FindKind::FindForward) => "cf",
            Pending::OperatorFind(Operator::Change, FindKind::FindBackward) => "cF",
            Pending::OperatorFind(Operator::Change, FindKind::TilForward) => "ct",
            Pending::OperatorFind(Operator::Change, FindKind::TilBackward) => "cT",
            Pending::OperatorFind(Operator::Yank, FindKind::FindForward) => "yf",
            Pending::OperatorFind(Operator::Yank, FindKind::FindBackward) => "yF",
            Pending::OperatorFind(Operator::Yank, FindKind::TilForward) => "yt",
            Pending::OperatorFind(Operator::Yank, FindKind::TilBackward) => "yT",
            Pending::TextObject(Operator::Delete, TextObjectScope::Inner) => "di",
            Pending::TextObject(Operator::Delete, TextObjectScope::A) => "da",
            Pending::TextObject(Operator::Change, TextObjectScope::Inner) => "ci",
            Pending::TextObject(Operator::Change, TextObjectScope::A) => "ca",
            Pending::TextObject(Operator::Yank, TextObjectScope::Inner) => "yi",
            Pending::TextObject(Operator::Yank, TextObjectScope::A) => "ya",
            Pending::VisualTextObject(TextObjectScope::Inner) => "i",
            Pending::VisualTextObject(TextObjectScope::A) => "a",
        });
        // Build display string from parts: [operator_count][suffix][count].
        let mut parts = String::new();
        if let Some(oc) = self.operator_count {
            parts.push_str(&oc.to_string());
        }
        if let Some(s) = suffix {
            parts.push_str(s);
        }
        if let Some(mc) = self.count {
            parts.push_str(&mc.to_string());
        }
        if parts.is_empty() { None } else { Some(parts) }
    }

    /// Save an undo snapshot of the current buffer state.
    fn save_undo_snapshot(&mut self) {
        self.undo_stack
            .push(self.buf.text().to_owned(), self.buf.cursor());
    }

    /// Consume the accumulated count, returning it (default 1).
    fn take_count(&mut self) -> u32 {
        self.count.take().unwrap_or(1)
    }

    /// Process one key event, returning the effect.
    pub fn step(&mut self, key: KeyEvent) -> EditorEffect {
        match self.mode {
            VimMode::Insert => self.step_insert(key),
            VimMode::Normal => self.step_normal(key),
            VimMode::Visual => self.step_visual(key),
        }
    }

    // -- Insert mode --------------------------------------------------------

    /// Save an undo snapshot on the first mutation within an insert session,
    /// so the entire session is one undo unit.
    fn ensure_insert_snapshot(&mut self) {
        if !self.insert_snapshot_saved {
            self.save_undo_snapshot();
            self.insert_snapshot_saved = true;
        }
    }

    fn step_insert(&mut self, key: KeyEvent) -> EditorEffect {
        match key.code {
            KeyCode::Esc => {
                self.mode = VimMode::Normal;
                self.insert_snapshot_saved = false;
                // Vim moves cursor back one when exiting insert mode.
                if self.buf.cursor() > 0 && !self.buf.is_empty() {
                    self.buf.move_left();
                }
                EditorEffect::Consumed
            }
            KeyCode::Enter => {
                let text = self.buf.take_text();
                self.insert_snapshot_saved = false;
                EditorEffect::Submit(text)
            }
            KeyCode::Backspace => {
                self.ensure_insert_snapshot();
                self.buf.backspace();
                EditorEffect::Consumed
            }
            KeyCode::Delete => {
                self.ensure_insert_snapshot();
                self.buf.delete_at_cursor();
                EditorEffect::Consumed
            }
            KeyCode::Left => {
                self.buf.move_left();
                EditorEffect::Consumed
            }
            KeyCode::Right => {
                self.buf.move_right();
                EditorEffect::Consumed
            }
            KeyCode::Char(c) => {
                self.ensure_insert_snapshot();
                self.buf.insert_char(c);
                EditorEffect::Consumed
            }
            _ => EditorEffect::Ignored,
        }
    }

    // -- Normal mode --------------------------------------------------------

    fn step_normal(&mut self, key: KeyEvent) -> EditorEffect {
        // Handle pending multi-key sequences first.
        if let Some(pending) = self.pending.take() {
            return self.step_pending(pending, key);
        }

        // Accumulate numeric count prefix.
        if let KeyCode::Char(d @ '1'..='9') = key.code {
            let digit = d as u32 - '0' as u32;
            self.count = Some(
                self.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit),
            );
            return EditorEffect::Consumed;
        }
        if let KeyCode::Char('0') = key.code
            && let Some(n) = self.count
        {
            self.count = Some(n.saturating_mul(10));
            return EditorEffect::Consumed;
            // Fall through: '0' without count = go to start of line.
        }

        self.step_normal_dispatch(key)
    }

    /// Dispatch a normal-mode key after count accumulation.
    fn step_normal_dispatch(&mut self, key: KeyEvent) -> EditorEffect {
        match key.code {
            KeyCode::Esc => {
                self.count = None;
                EditorEffect::Exit
            }

            // Undo / redo.
            KeyCode::Char('u') if key.modifiers == KeyModifiers::NONE => self.step_undo(),
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => self.step_redo(),

            // Motions (repeatable by count).
            KeyCode::Char('h')
            | KeyCode::Left
            | KeyCode::Char('l')
            | KeyCode::Right
            | KeyCode::Char('w')
            | KeyCode::Char('b')
            | KeyCode::Char('e')
            | KeyCode::Char('0')
            | KeyCode::Char('$')
            | KeyCode::Char('^') => {
                let n = self.take_count();
                for _ in 0..n {
                    self.step_normal_motion(key);
                }
                EditorEffect::Consumed
            }

            // Find/til, replace, operators (enter pending state).
            KeyCode::Char('f')
            | KeyCode::Char('t')
            | KeyCode::Char('F')
            | KeyCode::Char('T')
            | KeyCode::Char('r')
            | KeyCode::Char('d')
            | KeyCode::Char('c')
            | KeyCode::Char('y')
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.enter_pending(key)
            }

            // Paste.
            KeyCode::Char('p') | KeyCode::Char('P')
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.count = None;
                self.step_paste(key.code == KeyCode::Char('P'))
            }

            // Visual mode.
            KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => {
                self.count = None;
                self.enter_visual_mode()
            }

            // Mode transitions and deletions.
            KeyCode::Char('i')
            | KeyCode::Char('a')
            | KeyCode::Char('I')
            | KeyCode::Char('A')
            | KeyCode::Char('x')
            | KeyCode::Char('X') => self.step_normal_command(key),

            _ => {
                self.count = None;
                EditorEffect::Ignored
            }
        }
    }

    /// Set the pending state for multi-key normal-mode commands.
    fn enter_pending(&mut self, key: KeyEvent) -> EditorEffect {
        match key.code {
            KeyCode::Char('f') => self.pending = Some(Pending::FindForward),
            KeyCode::Char('F') => self.pending = Some(Pending::FindBackward),
            KeyCode::Char('t') => self.pending = Some(Pending::TilForward),
            KeyCode::Char('T') => self.pending = Some(Pending::TilBackward),
            KeyCode::Char('r') => {
                if !self.buf.is_empty() {
                    self.pending = Some(Pending::Replace);
                }
            }
            KeyCode::Char('d') => {
                self.operator_count = self.count.take();
                self.pending = Some(Pending::Operator(Operator::Delete));
            }
            KeyCode::Char('c') => {
                self.operator_count = self.count.take();
                self.pending = Some(Pending::Operator(Operator::Change));
            }
            KeyCode::Char('y') => {
                self.operator_count = self.count.take();
                self.pending = Some(Pending::Operator(Operator::Yank));
            }
            _ => unreachable!("enter_pending called with non-pending key"),
        }
        EditorEffect::Consumed
    }

    /// Undo: save live state for redo, then restore previous snapshot.
    /// Respects count: `3u` undoes 3 times.
    fn step_undo(&mut self) -> EditorEffect {
        let n = self.take_count();
        self.undo_stack
            .save_live(self.buf.text().to_owned(), self.buf.cursor());
        for _ in 0..n {
            if let Some((text, cursor)) = self.undo_stack.undo() {
                self.buf.text = text.clone();
                self.buf.cursor = *cursor;
            } else {
                break;
            }
        }
        self.buf.clamp_normal();
        assert!(self.buf.text.is_char_boundary(self.buf.cursor));
        assert!(self.buf.cursor <= self.buf.len());
        EditorEffect::Consumed
    }

    /// Redo: restore the next snapshot.
    /// Respects count: `3<C-r>` redoes 3 times.
    fn step_redo(&mut self) -> EditorEffect {
        let n = self.take_count();
        for _ in 0..n {
            if let Some((text, cursor)) = self.undo_stack.redo() {
                self.buf.text = text.clone();
                self.buf.cursor = *cursor;
            } else {
                break;
            }
        }
        self.buf.clamp_normal();
        assert!(self.buf.text.is_char_boundary(self.buf.cursor));
        assert!(self.buf.cursor <= self.buf.len());
        EditorEffect::Consumed
    }

    /// Handle cursor motion keys in normal mode.
    fn step_normal_motion(&mut self, key: KeyEvent) -> EditorEffect {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                self.buf.move_left();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.buf.move_right_normal();
            }
            KeyCode::Char('w') => {
                self.buf.cursor = self.buf.next_word_start();
                self.buf.clamp_normal();
            }
            KeyCode::Char('b') => {
                self.buf.cursor = self.buf.prev_word_start();
            }
            KeyCode::Char('e') => {
                self.buf.cursor = self.buf.word_end();
            }
            KeyCode::Char('0') => {
                self.buf.cursor = 0;
            }
            KeyCode::Char('$') => {
                if !self.buf.is_empty() {
                    self.buf.cursor = self.buf.last_char_offset();
                }
            }
            KeyCode::Char('^') => {
                self.buf.cursor = self.buf.first_non_whitespace();
                self.buf.clamp_normal();
            }
            _ => unreachable!("step_normal_motion called with non-motion key"),
        }
        EditorEffect::Consumed
    }

    /// Enter insert mode, marking that a snapshot is already saved.
    ///
    /// The snapshot is expected to have been saved before the operation
    /// that transitions to insert mode (e.g. by `apply_operator` for `c`,
    /// or explicitly for `i`/`a`/`I`/`A`).
    fn enter_insert_mode(&mut self) {
        self.insert_snapshot_saved = true;
        self.mode = VimMode::Insert;
    }

    /// Handle mode transitions (i/a/I/A) and deletions (x/X) in normal mode.
    fn step_normal_command(&mut self, key: KeyEvent) -> EditorEffect {
        match key.code {
            KeyCode::Char('i') => {
                self.count = None;
                self.save_undo_snapshot();
                self.enter_insert_mode();
            }
            KeyCode::Char('a') => {
                self.count = None;
                self.save_undo_snapshot();
                self.enter_insert_mode();
                self.buf.move_right();
            }
            KeyCode::Char('I') => {
                self.count = None;
                self.save_undo_snapshot();
                self.enter_insert_mode();
                self.buf.cursor = self.buf.first_non_whitespace();
            }
            KeyCode::Char('A') => {
                self.count = None;
                self.save_undo_snapshot();
                self.enter_insert_mode();
                self.buf.cursor = self.buf.len();
            }
            KeyCode::Char('x') => {
                if !self.buf.is_empty() {
                    let n = self.take_count();
                    // Compute range of chars to delete for register.
                    let start = self.buf.cursor();
                    let mut end = start;
                    for _ in 0..n {
                        if end < self.buf.len() {
                            end = self.buf.next_boundary(end);
                        }
                    }
                    self.register = Some(self.buf.text[start..end].to_string());
                    self.save_undo_snapshot();
                    for _ in 0..n {
                        self.buf.delete_at_cursor();
                    }
                }
            }
            KeyCode::Char('X') => {
                if !self.buf.is_empty() {
                    self.save_undo_snapshot();
                    let n = self.take_count();
                    for _ in 0..n {
                        self.buf.backspace();
                    }
                }
            }
            _ => unreachable!("step_normal_command called with non-command key"),
        }
        EditorEffect::Consumed
    }

    /// Handle the second key of a multi-key sequence.
    fn step_pending(&mut self, pending: Pending, key: KeyEvent) -> EditorEffect {
        // Esc always cancels any pending state and count.
        if key.code == KeyCode::Esc {
            self.count = None;
            self.operator_count = None;
            return EditorEffect::Consumed;
        }

        match pending {
            Pending::FindForward
            | Pending::FindBackward
            | Pending::TilForward
            | Pending::TilBackward => self.step_pending_find(pending, key),
            Pending::Replace => self.step_pending_replace(key),
            Pending::Operator(op) => self.step_operator(op, key),
            Pending::OperatorFind(op, kind) => self.step_operator_find(op, kind, key),
            Pending::TextObject(op, scope) => self.step_text_object(op, scope, key),
            Pending::VisualTextObject(scope) => self.step_visual_text_object(scope, key),
        }
    }

    /// Resolve a bare find/til pending (cursor motion only), with count.
    fn step_pending_find(&mut self, pending: Pending, key: KeyEvent) -> EditorEffect {
        let KeyCode::Char(target) = key.code else {
            self.count = None;
            return EditorEffect::Consumed;
        };
        let n = self.take_count();
        for _ in 0..n {
            let new_pos = match pending {
                Pending::FindForward => self.buf.find_forward(target),
                Pending::FindBackward => self.buf.find_backward(target),
                Pending::TilForward => self.buf.til_forward(target),
                Pending::TilBackward => self.buf.til_backward(target),
                _ => unreachable!("step_pending_find called with non-find pending"),
            };
            if let Some(pos) = new_pos {
                self.buf.cursor = pos;
            } else {
                break;
            }
        }
        EditorEffect::Consumed
    }

    /// Handle `r{char}`: replace character(s) under cursor.
    /// With count, `3ra` replaces 3 chars starting at cursor with 'a'.
    /// If fewer than `count` chars remain, it is a no-op (vim behavior).
    fn step_pending_replace(&mut self, key: KeyEvent) -> EditorEffect {
        let KeyCode::Char(c) = key.code else {
            self.count = None;
            return EditorEffect::Consumed;
        };
        assert!(!self.buf.is_empty(), "replace on empty buffer");
        let n = self.take_count();

        // Count the chars remaining from cursor. No-op if fewer than n.
        let chars_remaining = self.buf.text[self.buf.cursor..].chars().count();
        if (n as usize) > chars_remaining {
            return EditorEffect::Consumed;
        }

        self.save_undo_snapshot();
        for i in 0..n {
            let pos = self.buf.cursor;
            self.buf.replace_at_cursor(c);
            // Advance cursor for subsequent replacements, but stay on last char replaced.
            if i + 1 < n {
                self.buf.cursor = self.buf.next_boundary(pos);
            }
        }
        assert!(self.buf.text.is_char_boundary(self.buf.cursor));
        EditorEffect::Consumed
    }

    /// Handle the motion key after an operator (`d` or `c`).
    fn step_operator(&mut self, op: Operator, key: KeyEvent) -> EditorEffect {
        // Accumulate digits after the operator (e.g. `d3w`).
        if let KeyCode::Char(d @ '1'..='9') = key.code {
            let digit = d as u32 - '0' as u32;
            self.count = Some(
                self.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit),
            );
            self.pending = Some(Pending::Operator(op));
            return EditorEffect::Consumed;
        }
        if let KeyCode::Char('0') = key.code
            && let Some(n) = self.count
        {
            self.count = Some(n.saturating_mul(10));
            self.pending = Some(Pending::Operator(op));
            return EditorEffect::Consumed;
            // Fall through: '0' as motion = beginning of line.
        }
        self.step_operator_motion(op, key)
    }

    /// Dispatch a resolved motion after an operator, applying count.
    fn step_operator_motion(&mut self, op: Operator, key: KeyEvent) -> EditorEffect {
        let KeyCode::Char(ch) = key.code else {
            self.count = None;
            self.operator_count = None;
            return EditorEffect::Consumed;
        };
        match ch {
            'w' | 'b' | 'e' | '$' | '0' | '^' | 'h' | 'l' => {
                let range = self.resolve_counted_motion(op, ch);
                self.count = None;
                self.apply_operator(op, range);
            }
            'd' if op == Operator::Delete => {
                self.count = None;
                self.operator_count = None;
                self.apply_operator(op, Some((0, self.buf.len())));
            }
            'c' if op == Operator::Change => {
                self.count = None;
                self.operator_count = None;
                self.apply_operator(op, Some((0, self.buf.len())));
            }
            'y' if op == Operator::Yank => {
                self.count = None;
                self.operator_count = None;
                self.apply_operator(op, Some((0, self.buf.len())));
            }
            'i' => {
                self.pending = Some(Pending::TextObject(op, TextObjectScope::Inner));
                return EditorEffect::Consumed;
            }
            'a' => {
                self.pending = Some(Pending::TextObject(op, TextObjectScope::A));
                return EditorEffect::Consumed;
            }
            'f' => {
                self.pending = Some(Pending::OperatorFind(op, FindKind::FindForward));
                return EditorEffect::Consumed;
            }
            'F' => {
                self.pending = Some(Pending::OperatorFind(op, FindKind::FindBackward));
                return EditorEffect::Consumed;
            }
            't' => {
                self.pending = Some(Pending::OperatorFind(op, FindKind::TilForward));
                return EditorEffect::Consumed;
            }
            'T' => {
                self.pending = Some(Pending::OperatorFind(op, FindKind::TilBackward));
                return EditorEffect::Consumed;
            }
            _ => {
                self.count = None;
                self.operator_count = None;
            }
        }
        EditorEffect::Consumed
    }

    /// Resolve a motion repeated `count` times to a `(start, end)` range.
    ///
    /// Simulates the motion N times, advancing cursor after each step,
    /// then returns a range from the original cursor to the final endpoint.
    fn resolve_counted_motion(&mut self, op: Operator, ch: char) -> Option<(usize, usize)> {
        let motion_count = self.count.unwrap_or(1);
        let op_count = self.operator_count.take().unwrap_or(1);
        let n = op_count.saturating_mul(motion_count);
        let saved = self.buf.cursor();
        let is_backward = matches!(ch, 'b' | 'h' | '0' | '^');
        let mut found_any = false;

        for _ in 0..n {
            if let Some((s, e)) = self.resolve_simple_motion(op, ch) {
                found_any = true;
                self.buf.cursor = if is_backward { s } else { e };
            }
        }

        // The last resolve_simple_motion gave us the final boundary.
        // For forward motions, `e` from the last call is the correct end.
        // For backward motions, `s` from the last call is the correct start.
        let final_cursor = self.buf.cursor;
        self.buf.cursor = saved;

        if !found_any {
            return None;
        }
        assert!(final_cursor <= self.buf.len());
        let result = if is_backward {
            (final_cursor, saved)
        } else {
            (saved, final_cursor)
        };
        assert!(result.0 <= result.1, "motion range start > end");
        Some(result)
    }

    /// Resolve a simple (non-find) motion character to a `(start, end)` range.
    fn resolve_simple_motion(&self, _op: Operator, ch: char) -> Option<(usize, usize)> {
        let cur = self.buf.cursor();
        match ch {
            'w' => Some((cur, self.buf.next_word_start())),
            'b' => Some((self.buf.prev_word_start(), cur)),
            'e' => {
                let end = self.buf.word_end();
                // `e` is inclusive: include the character at `end`.
                Some((cur, self.buf.next_boundary(end)))
            }
            '$' => Some((cur, self.buf.len())),
            '0' => Some((0, cur)),
            '^' => {
                let fnw = self.buf.first_non_whitespace();
                if fnw < cur {
                    Some((fnw, cur))
                } else {
                    Some((cur, fnw))
                }
            }
            'h' => {
                if cur > 0 {
                    Some((self.buf.prev_boundary(cur), cur))
                } else {
                    None
                }
            }
            'l' => {
                if cur < self.buf.len() {
                    Some((cur, self.buf.next_boundary(cur)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Handle the char argument after an operator-find (e.g. `df{char}`).
    fn step_operator_find(&mut self, op: Operator, kind: FindKind, key: KeyEvent) -> EditorEffect {
        let KeyCode::Char(target) = key.code else {
            self.count = None;
            self.operator_count = None;
            return EditorEffect::Consumed;
        };
        // Multiply operator count and motion count for find-based operators.
        let motion_count = self.take_count();
        let op_count = self.operator_count.take().unwrap_or(1);
        let n = op_count.saturating_mul(motion_count);
        let cur = self.buf.cursor();
        let saved = cur;
        for _ in 0..n {
            let found = match kind {
                FindKind::FindForward => self.buf.find_forward(target),
                FindKind::TilForward => self.buf.til_forward(target),
                FindKind::FindBackward => self.buf.find_backward(target),
                FindKind::TilBackward => self.buf.til_backward(target),
            };
            if let Some(pos) = found {
                self.buf.cursor = pos;
            } else {
                // Not enough matches; restore and bail.
                self.buf.cursor = saved;
                self.apply_operator(op, None);
                return EditorEffect::Consumed;
            }
        }
        let endpoint = self.buf.cursor();
        self.buf.cursor = saved;
        let range = match kind {
            FindKind::FindForward => Some((cur, self.buf.next_boundary(endpoint))),
            FindKind::TilForward => Some((cur, endpoint)),
            FindKind::FindBackward => Some((endpoint, cur)),
            FindKind::TilBackward => Some((endpoint, cur)),
        };
        self.apply_operator(op, range);
        EditorEffect::Consumed
    }

    /// Apply an operator to a resolved range, saving an undo snapshot first.
    /// Empty ranges are no-ops: no deletion, no mode change.
    /// Deleted/yanked text is saved to the register.
    fn apply_operator(&mut self, op: Operator, range: Option<(usize, usize)>) {
        let Some((start, end)) = range else { return };
        if start >= end {
            return;
        }
        // Save the text before mutating.
        let yanked = self.buf.text[start..end].to_string();
        assert!(!yanked.is_empty());

        match op {
            Operator::Yank => {
                self.register = Some(yanked);
                // No deletion, no mode change. Yank is read-only.
            }
            Operator::Delete => {
                self.register = Some(yanked);
                self.save_undo_snapshot();
                self.buf.delete_range(start, end);
                self.buf.clamp_normal();
            }
            Operator::Change => {
                self.register = Some(yanked);
                self.save_undo_snapshot();
                self.buf.delete_range(start, end);
                self.mode = VimMode::Insert;
                self.insert_snapshot_saved = true;
            }
        }
        assert!(self.buf.text.is_char_boundary(self.buf.cursor));
    }

    // -- Text objects -------------------------------------------------------

    /// Resolve a text object pending state (e.g. `diw`, `caw`).
    fn step_text_object(
        &mut self,
        op: Operator,
        scope: TextObjectScope,
        key: KeyEvent,
    ) -> EditorEffect {
        let KeyCode::Char('w') = key.code else {
            self.count = None;
            self.operator_count = None;
            return EditorEffect::Consumed;
        };
        self.count = None;
        self.operator_count = None;
        let range = match scope {
            TextObjectScope::Inner => self.buf.inner_word(),
            TextObjectScope::A => self.buf.a_word(),
        };
        assert!(range.0 <= range.1);
        if range.0 < range.1 {
            self.apply_operator(op, Some(range));
        }
        EditorEffect::Consumed
    }

    /// Resolve a visual-mode text object (e.g. `viw`).
    fn step_visual_text_object(&mut self, scope: TextObjectScope, key: KeyEvent) -> EditorEffect {
        let KeyCode::Char('w') = key.code else {
            return EditorEffect::Consumed;
        };
        let (start, end) = match scope {
            TextObjectScope::Inner => self.buf.inner_word(),
            TextObjectScope::A => self.buf.a_word(),
        };
        assert!(start <= end);
        if start < end {
            self.buf.anchor = Some(start);
            // Cursor goes to last char of selection (end - 1 char).
            self.buf.cursor = self.buf.prev_boundary(end).max(start);
        }
        EditorEffect::Consumed
    }

    // -- Visual mode --------------------------------------------------------

    /// Enter visual mode, setting anchor at current cursor position.
    fn enter_visual_mode(&mut self) -> EditorEffect {
        self.mode = VimMode::Visual;
        let cursor = self.buf.cursor();
        self.buf.anchor = Some(cursor);
        assert!(cursor <= self.buf.len());
        EditorEffect::Consumed
    }

    /// Exit visual mode, returning to normal mode.
    fn exit_visual_mode(&mut self) {
        self.mode = VimMode::Normal;
        self.buf.anchor = None;
        self.pending = None;
    }

    /// Process a key in visual mode.
    fn step_visual(&mut self, key: KeyEvent) -> EditorEffect {
        // Handle pending states (text objects in visual mode).
        if let Some(pending) = self.pending.take() {
            return self.step_pending(pending, key);
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('v') => {
                self.exit_visual_mode();
                EditorEffect::Consumed
            }
            // Motions extend selection.
            KeyCode::Char('h') | KeyCode::Left => {
                self.buf.move_left();
                EditorEffect::Consumed
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.buf.move_right_normal();
                EditorEffect::Consumed
            }
            KeyCode::Char('w') => {
                self.buf.cursor = self.buf.next_word_start();
                self.buf.clamp_normal();
                EditorEffect::Consumed
            }
            KeyCode::Char('b') => {
                self.buf.cursor = self.buf.prev_word_start();
                EditorEffect::Consumed
            }
            KeyCode::Char('e') => {
                self.buf.cursor = self.buf.word_end();
                EditorEffect::Consumed
            }
            KeyCode::Char('0') => {
                self.buf.cursor = 0;
                EditorEffect::Consumed
            }
            KeyCode::Char('$') => {
                if !self.buf.is_empty() {
                    self.buf.cursor = self.buf.last_char_offset();
                }
                EditorEffect::Consumed
            }
            // Operators on selection.
            KeyCode::Char('d') | KeyCode::Char('x') => self.step_visual_operator(Operator::Delete),
            KeyCode::Char('c') => self.step_visual_operator(Operator::Change),
            KeyCode::Char('y') => self.step_visual_operator(Operator::Yank),
            // Text objects in visual mode.
            KeyCode::Char('i') => {
                self.pending = Some(Pending::VisualTextObject(TextObjectScope::Inner));
                EditorEffect::Consumed
            }
            KeyCode::Char('a') => {
                self.pending = Some(Pending::VisualTextObject(TextObjectScope::A));
                EditorEffect::Consumed
            }
            _ => EditorEffect::Consumed,
        }
    }

    /// Apply an operator to the visual selection, then exit visual mode.
    fn step_visual_operator(&mut self, op: Operator) -> EditorEffect {
        let anchor = self.buf.anchor.unwrap_or(self.buf.cursor());
        let cursor = self.buf.cursor();
        let start = anchor.min(cursor);
        // Visual selection is inclusive of the cursor char.
        let end = if anchor.max(cursor) < self.buf.len() {
            self.buf.next_boundary(anchor.max(cursor))
        } else {
            self.buf.len()
        };
        assert!(start <= end);
        self.exit_visual_mode();
        if start < end {
            self.apply_operator(op, Some((start, end)));
        }
        EditorEffect::Consumed
    }

    // -- Paste --------------------------------------------------------------

    /// Paste register contents before or after cursor.
    fn step_paste(&mut self, before: bool) -> EditorEffect {
        let Some(reg) = self.register.clone() else {
            return EditorEffect::Consumed;
        };
        assert!(!reg.is_empty());
        self.save_undo_snapshot();
        if before {
            // Insert before cursor.
            let pos = self.buf.cursor();
            self.buf.text.insert_str(pos, &reg);
            // Cursor on last inserted char.
            let end = pos + reg.len();
            self.buf.cursor = self.buf.prev_boundary(end).max(pos);
        } else {
            // Insert after cursor.
            let pos = if self.buf.is_empty() {
                0
            } else {
                self.buf.next_boundary(self.buf.cursor())
            };
            self.buf.text.insert_str(pos, &reg);
            let end = pos + reg.len();
            self.buf.cursor = self.buf.prev_boundary(end).max(pos);
        }
        assert!(self.buf.text.is_char_boundary(self.buf.cursor));
        assert!(self.buf.cursor <= self.buf.len());
        EditorEffect::Consumed
    }
}

const COMPACT_EDITOR_WIDTH: u16 = 10;
const COMPACT_EDITOR_HEIGHT: u16 = 1;
const FRAMED_EDITOR_HEIGHT: u16 = 4;
const FRAMED_EDITOR_TITLE: &str = " Vim Editor ";

impl LayoutRenderable for VimEditor {
    fn measure(&self, constraints: Constraints) -> Size {
        if constraints.max_height == Some(0) {
            return Size::ZERO;
        }

        let compact_due_to_height = constraints
            .max_height
            .is_some_and(|height| height < FRAMED_EDITOR_HEIGHT);
        let preferred_width = if compact_due_to_height {
            compact_editor_width(self)
        } else {
            framed_editor_width(self)
        };
        let width = constraints.constrain(Size::new(preferred_width, 0)).width;
        if width == 0 {
            return Size::ZERO;
        }

        let desired_height = match editor_overflow(width, constraints.max_height) {
            OverflowBehavior::Summary => COMPACT_EDITOR_HEIGHT,
            OverflowBehavior::Clip | OverflowBehavior::Ellipsis => FRAMED_EDITOR_HEIGHT,
        };

        constraints.constrain(Size::new(preferred_width, desired_height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if editor_overflow(area.width, Some(area.height)) == OverflowBehavior::Summary {
            render_compact_editor(self, area, buf);
            return;
        }

        render_framed_editor(self, area, buf);
    }
}

fn editor_overflow(width: u16, max_height: Option<u16>) -> OverflowBehavior {
    if width < COMPACT_EDITOR_WIDTH
        || max_height.is_some_and(|height| height < FRAMED_EDITOR_HEIGHT)
    {
        OverflowBehavior::Summary
    } else {
        OverflowBehavior::Ellipsis
    }
}

fn render_framed_editor(editor: &VimEditor, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(FRAMED_EDITOR_TITLE)
        .borders(Borders::ALL);
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let vertical = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    render_status_line(editor, vertical[0], buf);
    render_editor_line(editor, vertical[1], buf);
}

fn render_status_line(editor: &VimEditor, area: Rect, buf: &mut Buffer) {
    let text = status_line_text(editor);
    Paragraph::new(text).render(area, buf);
}

fn render_editor_line(editor: &VimEditor, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut spans = Vec::new();
    let selection = editor.selection();
    let text = editor.text();
    let cursor = editor.cursor();

    if text.is_empty() {
        let style = Style::default().add_modifier(TuiModifier::REVERSED);
        spans.push(Span::styled(" ", style));
    } else {
        for (offset, ch) in text.char_indices() {
            let mut style = Style::default();
            if selection.is_some_and(|(start, end)| offset >= start && offset <= end) {
                style = style.add_modifier(TuiModifier::REVERSED);
            }
            if offset == cursor {
                style = style.add_modifier(TuiModifier::UNDERLINED);
            }
            spans.push(Span::styled(ch.to_string(), style));
        }
    }

    if !text.is_empty() && cursor == text.len() {
        spans.push(Span::styled(
            " ",
            Style::default().add_modifier(TuiModifier::UNDERLINED),
        ));
    }

    Paragraph::new(Line::from(spans)).render(area, buf);
}

fn render_compact_editor(editor: &VimEditor, area: Rect, buf: &mut Buffer) {
    let summary = compact_editor_summary(editor, area.width as usize);
    buf.set_stringn(
        area.x,
        area.y,
        summary,
        area.width as usize,
        Style::default(),
    );
}

fn compact_editor_summary(editor: &VimEditor, width: usize) -> String {
    let mode = match editor.mode() {
        VimMode::Insert => "I",
        VimMode::Normal => "N",
        VimMode::Visual => "V",
    };
    let pending = editor.pending_display().unwrap_or_default();
    let prefix = if pending.is_empty() {
        format!("{mode} ")
    } else {
        format!("{mode} {pending} ")
    };
    summarize_text(&prefix, &editor_excerpt(editor), "", width)
}

fn compact_editor_width(editor: &VimEditor) -> u16 {
    saturating_width(
        display_width(&compact_editor_summary(editor, usize::MAX))
            .max(COMPACT_EDITOR_WIDTH as usize),
    )
}

fn framed_editor_width(editor: &VimEditor) -> u16 {
    let inner_width =
        display_width(&status_line_text(editor)).max(display_width(&editor_excerpt(editor)).max(1));
    let outer_width = display_width(FRAMED_EDITOR_TITLE).max(inner_width.saturating_add(2));
    saturating_width(outer_width.max(COMPACT_EDITOR_WIDTH as usize))
}

fn status_line_text(editor: &VimEditor) -> String {
    let mode = match editor.mode() {
        VimMode::Insert => "INSERT",
        VimMode::Normal => "NORMAL",
        VimMode::Visual => "VISUAL",
    };
    let pending = editor.pending_display().unwrap_or_else(|| "-".to_string());
    let cursor = format!("{}", editor.cursor());
    format!("mode:{mode} pending:{pending} cursor:{cursor}")
}

fn editor_excerpt(editor: &VimEditor) -> String {
    let text = editor.text();
    let cursor = editor.cursor();
    if text.is_empty() {
        return "|".to_string();
    }

    let mut excerpt = String::new();
    for (offset, ch) in text.char_indices() {
        if offset == cursor {
            excerpt.push('|');
        }
        excerpt.push(ch);
    }
    if cursor == text.len() {
        excerpt.push('|');
    }
    excerpt
}

fn saturating_width(width: usize) -> u16 {
    width.min(u16::MAX as usize) as u16
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{buffer::Buffer, layout::Rect};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn char_key(c: char) -> KeyEvent {
        if c.is_uppercase() {
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
        } else {
            key(KeyCode::Char(c))
        }
    }

    /// Feed a string of chars as insert-mode key events.
    fn type_str(editor: &mut VimEditor, s: &str) {
        for c in s.chars() {
            // In insert mode, chars are inserted directly regardless of modifiers.
            editor.step(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn render_text(editor: &VimEditor, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        editor.render(area, &mut buf);
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

    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    #[test]
    fn new_editor_starts_in_insert_mode() {
        let editor = VimEditor::new();
        assert_eq!(editor.mode(), VimMode::Insert);
        assert_eq!(editor.text(), "");
        assert_eq!(editor.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Insert mode basics
    // -----------------------------------------------------------------------

    #[test]
    fn insert_chars() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "hello");
        assert_eq!(ed.text(), "hello");
        assert_eq!(ed.cursor(), 5);
    }

    #[test]
    fn insert_backspace() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "abc");
        ed.step(key(KeyCode::Backspace));
        assert_eq!(ed.text(), "ab");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn insert_backspace_on_empty() {
        let mut ed = VimEditor::new();
        let effect = ed.step(key(KeyCode::Backspace));
        assert_eq!(effect, EditorEffect::Consumed);
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn insert_delete() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "abc");
        ed.step(key(KeyCode::Left));
        ed.step(key(KeyCode::Left));
        ed.step(key(KeyCode::Delete));
        assert_eq!(ed.text(), "ac");
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn insert_submit() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "hello world");
        let effect = ed.step(key(KeyCode::Enter));
        assert_eq!(effect, EditorEffect::Submit("hello world".to_string()));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn insert_esc_enters_normal() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "abc");
        let effect = ed.step(key(KeyCode::Esc));
        assert_eq!(effect, EditorEffect::Consumed);
        assert_eq!(ed.mode(), VimMode::Normal);
        // Cursor moves back one on Esc.
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn insert_left_right_arrows() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "abcd");
        ed.step(key(KeyCode::Left));
        ed.step(key(KeyCode::Left));
        assert_eq!(ed.cursor(), 2);
        ed.step(key(KeyCode::Right));
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn insert_unhandled_key_returns_ignored() {
        let mut ed = VimEditor::new();
        let effect = ed.step(key(KeyCode::F(5)));
        assert_eq!(effect, EditorEffect::Ignored);
    }

    // -----------------------------------------------------------------------
    // Normal mode: motions
    // -----------------------------------------------------------------------

    fn normal_editor(text: &str) -> VimEditor {
        let mut ed = VimEditor::new();
        type_str(&mut ed, text);
        ed.step(key(KeyCode::Esc));
        ed
    }

    #[test]
    fn desired_height_collapses_for_narrow_widths() {
        let editor = VimEditor::new();
        assert_eq!(
            editor.measure(Constraints::tight_width(20)).height,
            FRAMED_EDITOR_HEIGHT
        );
        assert_eq!(
            editor.measure(Constraints::tight_width(8)).height,
            COMPACT_EDITOR_HEIGHT
        );
    }

    #[test]
    fn measure_collapses_for_short_heights() {
        let editor = VimEditor::new();
        assert_eq!(editor.measure(Constraints::loose(20, 1)), Size::new(10, 1));
        assert_eq!(
            editor.measure(Constraints::loose(20, FRAMED_EDITOR_HEIGHT)),
            Size::new(20, FRAMED_EDITOR_HEIGHT)
        );
    }

    #[test]
    fn measure_prefers_content_width_when_constraints_are_loose() {
        let mut editor = VimEditor::new();
        type_str(&mut editor, "hi");

        let measured = editor.measure(Constraints::loose(40, 10));

        assert!(
            measured.width < 40,
            "expected intrinsic width, got {measured:?}"
        );
        assert_eq!(measured.height, FRAMED_EDITOR_HEIGHT);
    }

    #[test]
    fn compact_render_keeps_mode_and_cursor_excerpt() {
        let mut editor = VimEditor::new();
        type_str(&mut editor, "hello");
        let text = render_text(&editor, 8, 1);
        assert!(text.contains('I'));
        assert!(text.contains('|'));
    }

    #[test]
    fn normal_h_moves_left() {
        let mut ed = normal_editor("abc");
        assert_eq!(ed.cursor(), 2); // on 'c'
        ed.step(char_key('h'));
        assert_eq!(ed.cursor(), 1); // on 'b'
    }

    #[test]
    fn normal_h_clamps_at_zero() {
        let mut ed = normal_editor("a");
        ed.step(char_key('h')); // already at 0
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn normal_l_moves_right() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('h')); // move to 'b'
        ed.step(char_key('l')); // move to 'c'
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn normal_l_clamps_at_last_char() {
        let mut ed = normal_editor("abc");
        // Cursor is on 'c' (last char), l should not move.
        ed.step(char_key('l'));
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn normal_zero_moves_to_start() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn normal_dollar_moves_to_end() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('$'));
        assert_eq!(ed.cursor(), 4); // on 'o'
    }

    #[test]
    fn normal_caret_moves_to_first_non_whitespace() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "   hello");
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('^'));
        assert_eq!(ed.cursor(), 3); // on 'h'
    }

    #[test]
    fn normal_caret_on_no_whitespace() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('^'));
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Normal mode: word motions
    // -----------------------------------------------------------------------

    #[test]
    fn normal_w_simple() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 6); // on 'w'
    }

    #[test]
    fn normal_w_punctuation_boundary() {
        let mut ed = normal_editor("foo.bar");
        ed.step(char_key('0'));
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 3); // on '.'
    }

    #[test]
    fn normal_w_at_end() {
        let mut ed = normal_editor("hi");
        // Cursor on 'i'. w should clamp to last char.
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn normal_b_simple() {
        let mut ed = normal_editor("hello world");
        // Cursor on 'd' (last char).
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 6); // on 'w'
    }

    #[test]
    fn normal_b_at_start() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn normal_b_punctuation() {
        let mut ed = normal_editor("foo.bar");
        // Cursor on 'r'. b -> 'b' (pos 4), b -> '.', b -> 'f'.
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 4); // on 'b'
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 3); // on '.'
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 0); // on 'f'
    }

    #[test]
    fn normal_e_simple() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('e'));
        assert_eq!(ed.cursor(), 4); // on 'o' of "hello"
    }

    #[test]
    fn normal_e_moves_to_next_word_end() {
        let mut ed = normal_editor("hi there");
        ed.step(char_key('0'));
        ed.step(char_key('e'));
        assert_eq!(ed.cursor(), 1); // on 'i'
        ed.step(char_key('e'));
        assert_eq!(ed.cursor(), 7); // on 'e' of "there"
    }

    // -----------------------------------------------------------------------
    // Normal mode: f/t/F/T
    // -----------------------------------------------------------------------

    #[test]
    fn normal_f_finds_char() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('f'));
        ed.step(char_key('d'));
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn normal_f_not_found() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('f'));
        ed.step(char_key('z'));
        assert_eq!(ed.cursor(), 0); // unchanged
    }

    #[test]
    fn normal_t_stops_before() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('t'));
        ed.step(char_key('d'));
        assert_eq!(ed.cursor(), 2); // on 'c', one before 'd'
    }

    #[test]
    fn normal_big_f_finds_backward() {
        let mut ed = normal_editor("abcdef");
        // Cursor on 'f' (pos 5), but after Esc it's on 'e' (pos 4).
        ed.step(char_key('F'));
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn normal_big_t_stops_after() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('T'));
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 2); // on 'c', one after 'b'
    }

    #[test]
    fn pending_cancelled_by_esc() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('f'));
        ed.step(key(KeyCode::Esc)); // cancel pending
        assert_eq!(ed.cursor(), 0); // unchanged
    }

    // -----------------------------------------------------------------------
    // Normal mode: insert mode transitions
    // -----------------------------------------------------------------------

    #[test]
    fn normal_i_enters_insert_at_cursor() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('h')); // cursor on 'b'
        ed.step(char_key('i'));
        assert_eq!(ed.mode(), VimMode::Insert);
        assert_eq!(ed.cursor(), 1); // still on 'b'
    }

    #[test]
    fn normal_a_enters_insert_after_cursor() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('a'));
        assert_eq!(ed.mode(), VimMode::Insert);
        assert_eq!(ed.cursor(), 1); // after 'a'
    }

    #[test]
    fn normal_big_i_enters_insert_at_first_nonwhitespace() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "  hello");
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('I'));
        assert_eq!(ed.mode(), VimMode::Insert);
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn normal_big_a_enters_insert_at_end() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('A'));
        assert_eq!(ed.mode(), VimMode::Insert);
        assert_eq!(ed.cursor(), 3); // past end
    }

    // -----------------------------------------------------------------------
    // Normal mode: deletions
    // -----------------------------------------------------------------------

    #[test]
    fn normal_x_deletes_at_cursor() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "bc");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn normal_x_on_last_char() {
        let mut ed = normal_editor("abc");
        // Cursor on 'c'.
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "ab");
        // Cursor should move back to 'b'.
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn normal_x_on_single_char() {
        let mut ed = normal_editor("a");
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn normal_big_x_deletes_before_cursor() {
        let mut ed = normal_editor("abc");
        // Cursor on 'c'.
        ed.step(char_key('X'));
        assert_eq!(ed.text(), "ac");
        assert_eq!(ed.cursor(), 1); // on 'c'
    }

    #[test]
    fn normal_big_x_at_start() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('X'));
        assert_eq!(ed.text(), "abc"); // no change
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Normal mode: Esc exits editor
    // -----------------------------------------------------------------------

    #[test]
    fn normal_esc_returns_exit() {
        let mut ed = normal_editor("hello");
        let effect = ed.step(key(KeyCode::Esc));
        assert_eq!(effect, EditorEffect::Exit);
    }

    // -----------------------------------------------------------------------
    // take_text
    // -----------------------------------------------------------------------

    #[test]
    fn take_text_resets() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "hello");
        let text = ed.take_text();
        assert_eq!(text, "hello");
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.mode(), VimMode::Insert);
    }

    // -----------------------------------------------------------------------
    // Empty buffer edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn normal_motions_on_empty() {
        let mut ed = VimEditor::new();
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.mode(), VimMode::Normal);

        // All motions should be no-ops on empty buffer.
        for k in [
            char_key('h'),
            char_key('l'),
            char_key('w'),
            char_key('b'),
            char_key('e'),
            char_key('0'),
            char_key('$'),
            char_key('^'),
        ] {
            ed.step(k);
            assert_eq!(ed.cursor(), 0);
        }
    }

    #[test]
    fn normal_x_on_empty() {
        let mut ed = VimEditor::new();
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Multi-byte characters
    // -----------------------------------------------------------------------

    #[test]
    fn insert_multibyte() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "cafe\u{0301}"); // cafe with combining accent
        assert_eq!(ed.text(), "cafe\u{0301}");
    }

    #[test]
    fn normal_motions_with_multibyte() {
        let mut ed = VimEditor::new();
        // Insert some unicode.
        type_str(&mut ed, "\u{00e9}l\u{00e8}ve"); // "eleve" with accents
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('0'));
        assert_eq!(ed.cursor(), 0);
        ed.step(char_key('l'));
        assert_eq!(ed.cursor(), 2); // past the 2-byte e-acute
    }

    // -----------------------------------------------------------------------
    // Arrow keys in normal mode
    // -----------------------------------------------------------------------

    #[test]
    fn normal_arrow_keys() {
        let mut ed = normal_editor("abc");
        ed.step(key(KeyCode::Left));
        assert_eq!(ed.cursor(), 1);
        ed.step(key(KeyCode::Right));
        assert_eq!(ed.cursor(), 2);
    }

    // -----------------------------------------------------------------------
    // Unhandled key in normal mode
    // -----------------------------------------------------------------------

    #[test]
    fn normal_unhandled_key() {
        let mut ed = normal_editor("abc");
        let effect = ed.step(key(KeyCode::F(5)));
        assert_eq!(effect, EditorEffect::Ignored);
    }

    // -----------------------------------------------------------------------
    // Compound sequences
    // -----------------------------------------------------------------------

    #[test]
    fn insert_esc_motion_insert_again() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "hello world");
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('0'));
        ed.step(char_key('w'));
        ed.step(char_key('i'));
        type_str(&mut ed, "big ");
        assert_eq!(ed.text(), "hello big world");
    }

    #[test]
    fn f_motion_then_delete() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('f'));
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 6);
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "hello orld");
    }

    // -----------------------------------------------------------------------
    // Word motion edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn w_across_multiple_spaces() {
        let mut ed = normal_editor("a   b");
        ed.step(char_key('0'));
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 4); // on 'b'
    }

    #[test]
    fn b_across_multiple_spaces() {
        let mut ed = normal_editor("a   b");
        // Cursor on 'b' (pos 4) after Esc.
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 0); // on 'a'
    }

    #[test]
    fn e_single_char_word() {
        let mut ed = normal_editor("a b c");
        ed.step(char_key('0'));
        ed.step(char_key('e'));
        // 'a' is a single char word, e from 'a' should go to end of next word.
        // Actually, 'a' at pos 0 is already at its own end. e should move to 'b'.
        assert_eq!(ed.cursor(), 2); // on 'b'
    }

    #[test]
    fn w_on_punctuation_then_word() {
        let mut ed = normal_editor("a..b");
        ed.step(char_key('0'));
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 1); // on '.'
        ed.step(char_key('w'));
        assert_eq!(ed.cursor(), 3); // on 'b'
    }

    // -----------------------------------------------------------------------
    // Additional edge case tests
    // -----------------------------------------------------------------------

    #[test]
    fn e_with_trailing_whitespace() {
        let mut ed = normal_editor("hi  ");
        ed.step(char_key('0'));
        ed.step(char_key('e'));
        assert_eq!(ed.cursor(), 1); // on 'i'
    }

    #[test]
    fn w_unicode_word_chars() {
        let mut ed = normal_editor("caf\u{00e9} latte");
        ed.step(char_key('0'));
        ed.step(char_key('w'));
        // With Unicode-aware classification, "café" is one word (5 bytes).
        // w skips the word then the space, landing on 'l' of "latte".
        assert_eq!(ed.cursor(), 6);
    }

    #[test]
    fn t_adjacent_char_no_move() {
        let mut ed = normal_editor("ab");
        ed.step(char_key('0'));
        ed.step(char_key('t'));
        ed.step(char_key('b'));
        assert_eq!(ed.cursor(), 0); // can't stop "before" adjacent char
    }

    // -----------------------------------------------------------------------
    // Replace: r{char}
    // -----------------------------------------------------------------------

    #[test]
    fn replace_basic() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('r'));
        ed.step(char_key('z'));
        assert_eq!(ed.text(), "zbc");
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.mode(), VimMode::Normal);
    }

    #[test]
    fn replace_at_end() {
        let mut ed = normal_editor("abc");
        // Cursor on 'c' after Esc.
        ed.step(char_key('r'));
        ed.step(char_key('z'));
        assert_eq!(ed.text(), "abz");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn replace_single_char() {
        let mut ed = normal_editor("x");
        ed.step(char_key('r'));
        ed.step(char_key('y'));
        assert_eq!(ed.text(), "y");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn replace_with_unicode() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('r'));
        ed.step(KeyEvent::new(KeyCode::Char('\u{00e9}'), KeyModifiers::NONE));
        assert_eq!(ed.text(), "\u{00e9}bc");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn replace_on_empty_is_noop() {
        let mut ed = VimEditor::new();
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('r'));
        // Should not enter pending state on empty buffer.
        assert_eq!(ed.pending_display(), None);
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn replace_cancelled_by_esc() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('r'));
        assert_eq!(ed.pending_display(), Some("r".into()));
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.text(), "abc"); // Unchanged.
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Operator: dw, db, de, d$, d0, dd
    // -----------------------------------------------------------------------

    #[test]
    fn dw_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('w'));
        // Deletes "hello " (cursor to next word start).
        assert_eq!(ed.text(), "world");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn dw_at_end_of_line() {
        let mut ed = normal_editor("hello");
        // Cursor on 'o'. dw from last char deletes nothing if next_word_start == cursor.
        ed.step(char_key('d'));
        ed.step(char_key('w'));
        // "hello" with cursor on 'o' (pos 4), next_word_start returns 5 (len).
        assert_eq!(ed.text(), "hell");
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn dw_across_punctuation() {
        let mut ed = normal_editor("foo.bar");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('w'));
        // "foo" is a word, next word start is '.', so deletes "foo".
        assert_eq!(ed.text(), ".bar");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn db_basic() {
        let mut ed = normal_editor("hello world");
        // Cursor on 'd' (pos 10). db deletes back to 'w' (pos 6).
        ed.step(char_key('d'));
        ed.step(char_key('b'));
        assert_eq!(ed.text(), "hello d");
        assert_eq!(ed.cursor(), 6);
    }

    #[test]
    fn db_at_start() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('b'));
        // At start, prev_word_start is 0, range is (0, 0), no deletion.
        assert_eq!(ed.text(), "hello");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn de_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('e'));
        // Deletes "hello" (inclusive of word end).
        assert_eq!(ed.text(), " world");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn d_dollar_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('w')); // Move to 'w' of "world".
        ed.step(char_key('d'));
        ed.step(char_key('$'));
        assert_eq!(ed.text(), "hello ");
        assert_eq!(ed.cursor(), 5);
    }

    #[test]
    fn d_zero_basic() {
        let mut ed = normal_editor("hello world");
        // Cursor on 'd' (pos 10).
        ed.step(char_key('d'));
        ed.step(char_key('0'));
        assert_eq!(ed.text(), "d");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn d_caret() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "   hello");
        ed.step(key(KeyCode::Esc));
        // Cursor on 'o' (pos 7).
        ed.step(char_key('d'));
        ed.step(char_key('^'));
        // Deletes from first non-ws (pos 3) to cursor (pos 7).
        assert_eq!(ed.text(), "   o");
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn dd_clears_entire_line() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('d'));
        ed.step(char_key('d'));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn dh_basic() {
        let mut ed = normal_editor("abc");
        // Cursor on 'c' (pos 2).
        ed.step(char_key('d'));
        ed.step(char_key('h'));
        assert_eq!(ed.text(), "ac");
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn dl_basic() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('l'));
        assert_eq!(ed.text(), "bc");
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Operator: df{char}, dt{char}
    // -----------------------------------------------------------------------

    #[test]
    fn df_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('f'));
        ed.step(char_key('o'));
        // find_forward from 'h' finds 'o' at pos 4, inclusive so deletes [0, 5).
        assert_eq!(ed.text(), " world");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn df_not_found_is_noop() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('f'));
        ed.step(char_key('z'));
        assert_eq!(ed.text(), "hello");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn dt_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('t'));
        ed.step(char_key(' '));
        // til_forward finds ' ' at pos 5, stops one before at pos 4.
        // Deletes [0, 4) — but wait, til returns the position before target.
        // From 'h', til ' ' -> prev_boundary(5) = 4, clamped to max(cursor=0).
        // So range is (0, 4).
        assert_eq!(ed.text(), "o world");
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Operator: c variants (change)
    // -----------------------------------------------------------------------

    #[test]
    fn cc_clears_and_enters_insert() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('c'));
        ed.step(char_key('c'));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.mode(), VimMode::Insert);
    }

    #[test]
    fn cw_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('c'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "world");
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.mode(), VimMode::Insert);
    }

    #[test]
    fn c_dollar_basic() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('w')); // Move to 'w'.
        ed.step(char_key('c'));
        ed.step(char_key('$'));
        assert_eq!(ed.text(), "hello ");
        assert_eq!(ed.mode(), VimMode::Insert);
        // Can now type replacement text.
        type_str(&mut ed, "there");
        assert_eq!(ed.text(), "hello there");
    }

    #[test]
    fn cf_basic() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('c'));
        ed.step(char_key('f'));
        ed.step(char_key('d'));
        // Deletes [0, 4) inclusive of 'd', so [0, next_boundary(3)) = [0, 4).
        assert_eq!(ed.text(), "ef");
        assert_eq!(ed.mode(), VimMode::Insert);
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Operator cancelled by Esc
    // -----------------------------------------------------------------------

    #[test]
    fn operator_cancelled_by_esc() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        assert_eq!(ed.pending_display(), Some("d".into()));
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.text(), "hello"); // Unchanged.
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.pending_display(), None);
    }

    #[test]
    fn operator_find_cancelled_by_esc() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('f'));
        assert_eq!(ed.pending_display(), Some("df".into()));
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.text(), "hello");
        assert_eq!(ed.cursor(), 0);
    }

    // -----------------------------------------------------------------------
    // Pending display
    // -----------------------------------------------------------------------

    #[test]
    fn pending_display_shows_correct_state() {
        let mut ed = normal_editor("hello");
        assert_eq!(ed.pending_display(), None);

        ed.step(char_key('d'));
        assert_eq!(ed.pending_display(), Some("d".into()));

        // Complete the operator to clear pending.
        ed.step(char_key('d'));
        assert_eq!(ed.pending_display(), None);
    }

    // -----------------------------------------------------------------------
    // Undo / redo
    // -----------------------------------------------------------------------

    #[test]
    fn undo_insert_session() {
        // Type text, esc, undo restores empty.
        let mut ed = VimEditor::new();
        type_str(&mut ed, "hello");
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.text(), "hello");

        ed.step(char_key('u'));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn undo_multiple_commands() {
        // Several commands, undo each one.
        let mut ed = VimEditor::new();
        type_str(&mut ed, "abc");
        ed.step(key(KeyCode::Esc));
        // Now in normal mode on 'c'. Delete 'c' with x.
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "ab");
        // Delete 'b' with x.
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "a");

        // Undo second x.
        ed.step(char_key('u'));
        assert_eq!(ed.text(), "ab");
        // Undo first x.
        ed.step(char_key('u'));
        assert_eq!(ed.text(), "abc");
    }

    #[test]
    fn redo_after_undo() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "hello");
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('x')); // Delete 'o'.
        assert_eq!(ed.text(), "hell");

        ed.step(char_key('u')); // Undo.
        assert_eq!(ed.text(), "hello");

        // Ctrl+R to redo.
        ed.step(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.text(), "hell");
    }

    #[test]
    fn redo_truncated_by_new_edit() {
        let mut ed = VimEditor::new();
        type_str(&mut ed, "abc");
        ed.step(key(KeyCode::Esc));
        ed.step(char_key('x')); // Delete 'c' -> "ab".
        ed.step(char_key('u')); // Undo -> "abc".

        // New edit truncates redo history.
        ed.step(char_key('x')); // Delete 'c' again.
        assert_eq!(ed.text(), "ab");

        // Redo should do nothing since history was truncated.
        ed.step(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.text(), "ab");
    }

    #[test]
    fn undo_after_dw() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "world");

        ed.step(char_key('u'));
        assert_eq!(ed.text(), "hello world");
    }

    #[test]
    fn undo_after_replace() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('r'));
        ed.step(char_key('z'));
        assert_eq!(ed.text(), "zbc");

        ed.step(char_key('u'));
        assert_eq!(ed.text(), "abc");
    }

    #[test]
    fn insert_session_is_one_undo_unit() {
        // Enter insert, type multiple chars, esc, undo reverts all of them.
        let mut ed = normal_editor("abc");
        ed.step(char_key('A')); // Append at end.
        type_str(&mut ed, "def");
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.text(), "abcdef");

        ed.step(char_key('u'));
        assert_eq!(ed.text(), "abc");
    }

    #[test]
    fn undo_on_empty_stack_is_noop() {
        let mut ed = VimEditor::new();
        ed.step(key(KeyCode::Esc));
        // No edits have been made; undo should be a no-op.
        ed.step(char_key('u'));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn redo_on_empty_stack_is_noop() {
        let mut ed = VimEditor::new();
        ed.step(key(KeyCode::Esc));
        // No undos have been performed; redo should be a no-op.
        ed.step(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn undo_restores_cursor_position() {
        let mut ed = normal_editor("hello world");
        let cursor_before = ed.cursor();
        ed.step(char_key('x'));
        assert_ne!(ed.cursor(), cursor_before);

        ed.step(char_key('u'));
        assert_eq!(ed.text(), "hello world");
        // Cursor should be restored to where it was before the command.
        assert_eq!(ed.cursor(), cursor_before);
    }

    #[test]
    fn undo_stack_bounds() {
        // Push more than UNDO_STACK_CAPACITY entries, oldest should be dropped.
        let mut ed = normal_editor("x");
        // Repeatedly replace the char to fill the undo stack.
        for i in 0..120 {
            let c = char::from(b'a' + (i % 26));
            ed.step(char_key('r'));
            ed.step(char_key(c));
        }
        // Should still be able to undo up to UNDO_STACK_CAPACITY times.
        for _ in 0..UNDO_STACK_CAPACITY {
            ed.step(char_key('u'));
        }
        // One more undo should be a no-op (oldest entries dropped).
        let text_before = ed.text().to_string();
        ed.step(char_key('u'));
        assert_eq!(ed.text(), text_before);
    }

    // -----------------------------------------------------------------------
    // Numeric counts
    // -----------------------------------------------------------------------

    #[test]
    fn count_3w_moves_three_words() {
        let mut ed = normal_editor("one two three four");
        ed.step(char_key('0'));
        ed.step(char_key('3'));
        ed.step(char_key('w'));
        // Should land on 'f' of "four".
        assert_eq!(ed.cursor(), 14);
    }

    #[test]
    fn count_3l_moves_three_chars() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('3'));
        ed.step(char_key('l'));
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn count_2x_deletes_two_chars() {
        let mut ed = normal_editor("abcdef");
        ed.step(char_key('0'));
        ed.step(char_key('2'));
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "cdef");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn count_3dw_deletes_three_words() {
        let mut ed = normal_editor("one two three four");
        ed.step(char_key('0'));
        ed.step(char_key('3'));
        ed.step(char_key('d'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "four");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn count_d3w_deletes_three_words() {
        // Count after operator also works.
        let mut ed = normal_editor("one two three four");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('3'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "four");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn zero_still_moves_to_start_without_count() {
        let mut ed = normal_editor("hello world");
        // Cursor on 'd' (last char). Press '0' with no preceding count.
        ed.step(char_key('0'));
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn two_digit_count_10l() {
        let mut ed = normal_editor("abcdefghijklmno");
        ed.step(char_key('0'));
        ed.step(char_key('1'));
        ed.step(char_key('0'));
        ed.step(char_key('l'));
        assert_eq!(ed.cursor(), 10);
    }

    #[test]
    fn count_2fa_finds_second_a() {
        let mut ed = normal_editor("banana");
        ed.step(char_key('0'));
        ed.step(char_key('2'));
        ed.step(char_key('f'));
        ed.step(char_key('a'));
        // First 'a' at pos 1, from there second 'a' at pos 3.
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn count_display_in_pending() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('3'));
        assert_eq!(ed.pending_display(), Some("3".into()));

        ed.step(char_key('d'));
        assert_eq!(ed.pending_display(), Some("3d".into()));

        // Cancel to avoid side effects.
        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.pending_display(), None);
    }

    #[test]
    fn count_3h_moves_left_three() {
        let mut ed = normal_editor("abcdef");
        // Cursor on 'f' (pos 5) after esc.
        assert_eq!(ed.cursor(), 5);
        ed.step(char_key('3'));
        ed.step(char_key('h'));
        // 5 -> 4 -> 3 -> 2
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn count_cleared_on_unhandled_key() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('3'));
        assert_eq!(ed.pending_display(), Some("3".into()));
        ed.step(key(KeyCode::F(5))); // Unhandled.
        assert_eq!(ed.pending_display(), None);
    }

    // -----------------------------------------------------------------------
    // Counted undo / redo
    // -----------------------------------------------------------------------

    #[test]
    fn counted_undo() {
        // 3u undoes 3 operations.
        let mut ed = normal_editor("abc");
        ed.step(char_key('x')); // delete c -> "ab"
        ed.step(char_key('x')); // delete b -> "a"
        ed.step(char_key('x')); // delete a -> ""
        // 3u should restore all three.
        ed.step(char_key('3'));
        ed.step(char_key('u'));
        assert_eq!(ed.text(), "abc");
    }

    #[test]
    fn counted_redo() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('x'));
        ed.step(char_key('x'));
        ed.step(char_key('x'));
        ed.step(char_key('3'));
        ed.step(char_key('u'));
        assert_eq!(ed.text(), "abc");
        // 3 Ctrl+R should redo all three.
        ed.step(char_key('3'));
        ed.step(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.text(), "");
    }

    // -----------------------------------------------------------------------
    // Counted replace
    // -----------------------------------------------------------------------

    #[test]
    fn counted_replace() {
        // 3ra on "hello" at pos 0 -> "aaalo".
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('3'));
        ed.step(char_key('r'));
        ed.step(char_key('a'));
        assert_eq!(ed.text(), "aaalo");
    }

    #[test]
    fn counted_replace_too_few_chars() {
        // 5ra on "ab" -- only 2 chars, should be no-op.
        let mut ed = normal_editor("ab");
        ed.step(char_key('0'));
        ed.step(char_key('5'));
        ed.step(char_key('r'));
        ed.step(char_key('a'));
        assert_eq!(ed.text(), "ab");
    }

    // -----------------------------------------------------------------------
    // Edge case: count exceeding buffer
    // -----------------------------------------------------------------------

    #[test]
    fn count_exceeding_buffer_length() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        // 100x on 3-char buffer.
        ed.step(char_key('1'));
        ed.step(char_key('0'));
        ed.step(char_key('0'));
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn count_l_clamps_at_end() {
        let mut ed = normal_editor("abc");
        ed.step(char_key('0'));
        ed.step(char_key('1'));
        ed.step(char_key('0'));
        ed.step(char_key('0'));
        ed.step(char_key('l'));
        assert_eq!(ed.cursor(), 2); // Clamped at last char.
    }

    // -----------------------------------------------------------------------
    // Operator count multiplication
    // -----------------------------------------------------------------------

    #[test]
    fn operator_count_multiplication() {
        // 2d3w should delete 6 words.
        let mut ed = normal_editor("a b c d e f g h");
        ed.step(char_key('0'));
        ed.step(char_key('2'));
        ed.step(char_key('d'));
        ed.step(char_key('3'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "g h");
    }

    // -----------------------------------------------------------------------
    // Visual mode
    // -----------------------------------------------------------------------

    #[test]
    fn visual_v_enters_and_esc_exits() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        assert_eq!(ed.mode(), VimMode::Visual);
        assert_eq!(ed.selection(), Some((0, 0)));

        ed.step(key(KeyCode::Esc));
        assert_eq!(ed.mode(), VimMode::Normal);
        assert_eq!(ed.selection(), None);
    }

    #[test]
    fn visual_v_toggles_back_to_normal() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('v'));
        assert_eq!(ed.mode(), VimMode::Visual);
        ed.step(char_key('v'));
        assert_eq!(ed.mode(), VimMode::Normal);
    }

    #[test]
    fn visual_motion_extends_selection() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('w'));
        // After w, cursor at 6 ('w'), selection is (0, 6).
        assert_eq!(ed.selection(), Some((0, 6)));
        assert_eq!(ed.cursor(), 6);
    }

    #[test]
    fn visual_dollar_extends_to_end() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('$'));
        assert_eq!(ed.selection(), Some((0, 4)));
    }

    #[test]
    fn visual_e_extends_to_word_end() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('e'));
        assert_eq!(ed.selection(), Some((0, 4)));
    }

    #[test]
    fn visual_d_deletes_selection() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('w'));
        ed.step(char_key('d'));
        // Deletes "hello " (0..6, inclusive of cursor char 'w' -> next boundary = 7).
        // Wait: selection is (0, 6), step_visual_operator makes it inclusive of cursor char.
        // anchor=0, cursor=6. start=0, end=next_boundary(6)=7. Deletes "hello w".
        // Actually let me re-check: selection is min(anchor, cursor)..max(anchor, cursor).
        // anchor=0, cursor=6. So selection() returns (0, 6). But step_visual_operator
        // computes end = next_boundary(max(0,6)) = 7. So deletes [0,7) = "hello w".
        assert_eq!(ed.text(), "orld");
        assert_eq!(ed.mode(), VimMode::Normal);
    }

    #[test]
    fn visual_c_changes_selection() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('e'));
        ed.step(char_key('c'));
        // Deletes "hello" (0..5, inclusive of 'o' at pos 4 -> next_boundary = 5).
        assert_eq!(ed.text(), " world");
        assert_eq!(ed.mode(), VimMode::Insert);
    }

    #[test]
    fn visual_y_yanks_selection() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('e'));
        ed.step(char_key('y'));
        // Text unchanged, back to normal mode.
        assert_eq!(ed.text(), "hello world");
        assert_eq!(ed.mode(), VimMode::Normal);
        // Cursor remains at end of selection (pos 4, on 'o').
        assert_eq!(ed.cursor(), 4);
        // Paste after cursor to verify yanked text.
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "hellohello world");
    }

    #[test]
    fn visual_x_same_as_d() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('l'));
        ed.step(char_key('x'));
        assert_eq!(ed.text(), "llo");
        assert_eq!(ed.mode(), VimMode::Normal);
    }

    // -----------------------------------------------------------------------
    // Yank / paste
    // -----------------------------------------------------------------------

    #[test]
    fn yank_inner_word_then_paste() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('y'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "hello world"); // Unchanged.
        // Paste after cursor.
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "hhelloello world");
    }

    #[test]
    fn dd_yanks_then_paste() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('d'));
        ed.step(char_key('d'));
        assert_eq!(ed.text(), "");
        // Paste the deleted line.
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "hello world");
    }

    #[test]
    fn dw_yanks_deleted_text() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "world");
        // Paste the deleted "hello ".
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "whello orld");
    }

    #[test]
    fn big_p_pastes_before_cursor() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('y'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        // Move to 'w' of "world".
        ed.step(char_key('w'));
        ed.step(char_key('P'));
        // P inserts before cursor.
        assert_eq!(ed.text(), "hello helloworld");
    }

    #[test]
    fn paste_on_empty_register_is_noop() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "hello");
    }

    #[test]
    fn paste_saves_undo_snapshot() {
        let mut ed = normal_editor("hello");
        ed.step(char_key('0'));
        ed.step(char_key('y'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        ed.step(char_key('$'));
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "hellohello");
        // Undo should restore.
        ed.step(char_key('u'));
        assert_eq!(ed.text(), "hello");
    }

    // -----------------------------------------------------------------------
    // Text objects
    // -----------------------------------------------------------------------

    #[test]
    fn diw_deletes_inner_word() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('l')); // on 'e'
        ed.step(char_key('d'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), " world");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn diw_on_whitespace_deletes_whitespace() {
        let mut ed = normal_editor("hello   world");
        // Move to whitespace.
        ed.step(char_key('0'));
        ed.step(char_key('w')); // Now on first space? No, w goes to next word start.
        // Actually w from 'h' skips "hello" and spaces, lands on 'w'.
        // Let me use f to find a space.
        ed.step(char_key('0'));
        ed.step(char_key('f'));
        ed.step(char_key(' ')); // on first space at pos 5
        ed.step(char_key('d'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "helloworld");
    }

    #[test]
    fn ciw_changes_word() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('c'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), " world");
        assert_eq!(ed.mode(), VimMode::Insert);
        type_str(&mut ed, "goodbye");
        assert_eq!(ed.text(), "goodbye world");
    }

    #[test]
    fn daw_deletes_word_and_trailing_space() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('d'));
        ed.step(char_key('a'));
        ed.step(char_key('w'));
        // "hello" + trailing space " " deleted.
        assert_eq!(ed.text(), "world");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn daw_at_end_includes_leading_space() {
        let mut ed = normal_editor("hello world");
        // Cursor on 'd' (last char). Move to 'w' of world.
        ed.step(char_key('b')); // on 'w'
        ed.step(char_key('d'));
        ed.step(char_key('a'));
        ed.step(char_key('w'));
        // "world" has no trailing space, so leading " " is included.
        assert_eq!(ed.text(), "hello");
    }

    #[test]
    fn viw_selects_word() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('v'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        assert_eq!(ed.mode(), VimMode::Visual);
        // anchor=0 (start of "hello"), cursor=4 (on 'o', last char of "hello").
        assert_eq!(ed.selection(), Some((0, 4)));
    }

    #[test]
    fn yiw_yanks_word() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('y'));
        ed.step(char_key('i'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "hello world");
        // Paste to verify.
        ed.step(char_key('p'));
        assert_eq!(ed.text(), "hhelloello world");
    }

    #[test]
    fn caw_changes_word_with_space() {
        let mut ed = normal_editor("hello world");
        ed.step(char_key('0'));
        ed.step(char_key('c'));
        ed.step(char_key('a'));
        ed.step(char_key('w'));
        assert_eq!(ed.text(), "world");
        assert_eq!(ed.mode(), VimMode::Insert);
        type_str(&mut ed, "goodbye ");
        assert_eq!(ed.text(), "goodbye world");
    }
}

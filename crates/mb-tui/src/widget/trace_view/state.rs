//! Interactive state over immutable admitted facts and time lanes.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::data::{GroupId, ItemId, TraceData, TraceError, TraceItem, TraceTrack};
use super::layout::{Lane, TimeWindow, track_lanes};
use crate::input::KeyResult;

/// A selectable row in the current flat group presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Group(usize),
    Lane { track: usize, lane: usize },
}

/// A bounded timeline widget with stable overlap lanes and independent viewport.
///
/// Data is immutable after construction. Pan/zoom never changes lanes or
/// selection. Collapsing the selected item's group moves selection to its header.
#[derive(Clone)]
pub struct TraceView {
    pub(crate) data: TraceData,
    pub(crate) lanes: Vec<Vec<Lane>>,
    group_tracks: BTreeMap<Option<GroupId>, Vec<usize>>,
    collapsed: BTreeSet<GroupId>,
    pub(crate) rows: Vec<Row>,
    pub(crate) cursor: usize,
    item_cursor: usize,
    pub(crate) window: TimeWindow,
    pub(crate) show_details: bool,
    scroll: usize,
    viewport_height: u16,
}

impl TraceView {
    /// Admit complete display data and compute deterministic time lanes once.
    ///
    /// # Errors
    /// Rejects duplicate identities, unknown group references, and the bounds
    /// documented on [`TraceData`]. Reversed interval endpoints remain valid input.
    pub fn new(data: TraceData) -> Result<Self, TraceError> {
        data.validate()?;
        let window = TimeWindow::fit(&data);
        let lanes = data.tracks.iter().map(track_lanes).collect();
        let mut group_tracks: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (index, track) in data.tracks.iter().enumerate() {
            group_tracks.entry(track.group).or_default().push(index);
        }
        let mut view = Self {
            data,
            lanes,
            group_tracks,
            collapsed: BTreeSet::new(),
            rows: Vec::new(),
            cursor: 0,
            item_cursor: 0,
            window,
            show_details: true,
            scroll: 0,
            viewport_height: 0,
        };
        view.rebuild_rows();
        Ok(view)
    }

    /// Borrow admitted facts without exposing mutable layout inputs.
    pub fn data(&self) -> &TraceData {
        &self.data
    }

    /// Return the selected item, or no item for a group header or empty track.
    pub fn selected_item(&self) -> Option<&TraceItem> {
        let Row::Lane { track, lane } = *self.rows.get(self.cursor)? else {
            return None;
        };
        let item = *self.lanes[track][lane].items.get(self.item_cursor)?;
        Some(&self.data.tracks[track].items[item])
    }

    /// Return the selected track, including tracks with no observations.
    pub fn selected_track(&self) -> Option<&TraceTrack> {
        match *self.rows.get(self.cursor)? {
            Row::Lane { track, .. } => Some(&self.data.tracks[track]),
            Row::Group(_) => None,
        }
    }

    /// Return the selected group header or the selected track's containing group.
    pub fn selected_group(&self) -> Option<GroupId> {
        match *self.rows.get(self.cursor)? {
            Row::Group(group) => Some(self.data.groups[group].id),
            Row::Lane { track, .. } => self.data.tracks[track].group,
        }
    }

    /// Select an item by identity, expanding its group and revealing its time.
    /// Returns false for an absent identity without changing state.
    pub fn select_item(&mut self, id: ItemId) -> bool {
        let Some((track, item)) = self.data.tracks.iter().enumerate().find_map(|(t, track)| {
            track
                .items
                .iter()
                .position(|item| item.id == id)
                .map(|i| (t, i))
        }) else {
            return false;
        };
        if let Some(group) = self.data.tracks[track].group
            && self.collapsed.remove(&group)
        {
            self.rebuild_rows();
        }
        let lane = self.lanes[track]
            .iter()
            .position(|lane| lane.items.contains(&item))
            .expect("admitted item has a lane");
        self.cursor = self
            .rows
            .iter()
            .position(|row| *row == Row::Lane { track, lane })
            .expect("expanded track has a row");
        self.item_cursor = self.lanes[track][lane]
            .items
            .iter()
            .position(|&i| i == item)
            .expect("item belongs to its lane");
        self.reveal_selection();
        true
    }

    /// Current inclusive viewport coordinates, in the dataset's time unit.
    pub fn time_window(&self) -> (i64, i64) {
        (self.window.start, self.window.end)
    }

    /// Set an explicit nonempty viewing range without disturbing selection.
    ///
    /// # Errors
    /// Returns [`TraceError::InvalidTimeWindow`] when `start >= end`.
    pub fn set_time_window(&mut self, start: i64, end: i64) -> Result<(), TraceError> {
        if start >= end {
            return Err(TraceError::InvalidTimeWindow);
        }
        self.window = TimeWindow { start, end };
        Ok(())
    }

    /// Fit all known coordinates; untimed observations do not invent an axis.
    pub fn fit_time(&mut self) {
        self.window = TimeWindow::fit(&self.data);
    }

    /// Whether the named group is collapsed.
    pub fn is_group_collapsed(&self, id: GroupId) -> bool {
        self.collapsed.contains(&id)
    }

    /// Change a group's visibility, reconciling hidden selection to its header.
    /// Returns false for an absent group without changing state.
    pub fn set_group_collapsed(&mut self, id: GroupId, collapsed: bool) -> bool {
        let Some(group) = self.data.groups.iter().position(|group| group.id == id) else {
            return false;
        };
        let previous = self.rows.get(self.cursor).copied();
        let hides_selection = collapsed && self.selected_group() == Some(id);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
        self.rebuild_rows();
        let target = if hides_selection {
            Some(Row::Group(group))
        } else {
            previous
        };
        self.cursor = self
            .rows
            .iter()
            .position(|row| Some(*row) == target)
            .unwrap_or(0);
        if hides_selection {
            self.item_cursor = 0;
        }
        self.ensure_visible();
        true
    }

    /// Current index among selectable visible rows.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Number of rows after group collapse, before viewport clipping.
    pub fn visible_row_count(&self) -> usize {
        self.rows.len()
    }

    /// Cached vertical offset, updated by mutable render and row navigation.
    pub fn scroll_offset(&self) -> usize {
        self.scroll
    }

    /// Dispatch documented controls, leaving application and modified keys alone.
    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
        if key.kind == KeyEventKind::Release
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return KeyResult::Ignored;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_row(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_row(1),
            KeyCode::PageUp => self.move_row(-(self.viewport_height.max(1) as isize)),
            KeyCode::PageDown => self.move_row(self.viewport_height.max(1) as isize),
            KeyCode::Tab => self.move_item(1),
            KeyCode::BackTab => self.move_item(-1),
            KeyCode::Left | KeyCode::Char('h') => self.pan(-1),
            KeyCode::Right | KeyCode::Char('l') => self.pan(1),
            KeyCode::Char('+') | KeyCode::Char('=') => self.zoom(true),
            KeyCode::Char('-') => self.zoom(false),
            KeyCode::Char('0') => self.fit_time(),
            KeyCode::Enter => self.show_details = !self.show_details,
            KeyCode::Char(' ') => {
                if let Some(group) = self.selected_group() {
                    self.set_group_collapsed(group, !self.is_group_collapsed(group));
                }
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    /// Clamp scrolling against the current height and keep selection visible.
    pub(crate) fn effective_scroll(&self, height: u16) -> usize {
        let height = usize::from(height.max(1));
        let scroll = self.scroll.min(self.rows.len().saturating_sub(height));
        scroll
            .min(self.cursor)
            .max((self.cursor + 1).saturating_sub(height))
    }

    /// Refresh the viewport cache after layout so subsequent page keys are exact.
    pub(crate) fn set_viewport_height(&mut self, height: u16) {
        self.viewport_height = height;
        self.ensure_visible();
    }

    /// Return track/item totals without counting concurrent intervals as time.
    pub(crate) fn group_counts(&self, id: GroupId) -> (usize, usize) {
        self.group_tracks.get(&Some(id)).map_or((0, 0), |tracks| {
            (
                tracks.len(),
                tracks
                    .iter()
                    .map(|&t| self.data.tracks[t].items.len())
                    .sum(),
            )
        })
    }

    /// Rebuild only visibility; immutable lane assignment remains authoritative.
    fn rebuild_rows(&mut self) {
        self.rows.clear();
        for (index, group) in self.data.groups.iter().enumerate() {
            self.rows.push(Row::Group(index));
            if !self.collapsed.contains(&group.id) {
                append_tracks(
                    &mut self.rows,
                    &self.lanes,
                    self.group_tracks.get(&Some(group.id)),
                );
            }
        }
        append_tracks(&mut self.rows, &self.lanes, self.group_tracks.get(&None));
    }

    /// Move within visible rows; selection changes reveal the next item.
    fn move_row(&mut self, delta: isize) {
        let cursor = self
            .cursor
            .saturating_add_signed(delta)
            .min(self.rows.len().saturating_sub(1));
        if cursor != self.cursor {
            self.cursor = cursor;
            self.item_cursor = 0;
            self.reveal_selection();
        }
    }

    /// Cycle items in a lane so projected collisions never hide an observation.
    fn move_item(&mut self, delta: isize) {
        let Some(&Row::Lane { track, lane }) = self.rows.get(self.cursor) else {
            return;
        };
        let count = self.lanes[track][lane].items.len();
        if count > 0 {
            self.item_cursor =
                (self.item_cursor as isize + delta).rem_euclid(count as isize) as usize;
            self.reveal_selection();
        }
    }

    /// Move by a quarter-window, preserving span at signed-coordinate limits.
    fn pan(&mut self, direction: i128) {
        let delta = (self.window.span() / 4).max(1) * direction;
        self.window =
            TimeWindow::shifted(i128::from(self.window.start) + delta, self.window.span());
    }

    /// Scale around the viewport center using exact integer coordinates.
    fn zoom(&mut self, inward: bool) {
        let old_span = self.window.span();
        let span = if inward {
            (old_span / 2).max(1)
        } else {
            (old_span * 2).min(i128::from(u64::MAX))
        };
        let start = i128::from(self.window.start) + (old_span - span) / 2;
        self.window = TimeWindow::shifted(start, span);
    }

    /// Reveal a recorded coordinate without changing the time scale.
    fn reveal_selection(&mut self) {
        if let Some(item) = self.selected_item() {
            let coordinates = item.timing.coordinates();
            let visible = coordinates
                .into_iter()
                .flatten()
                .any(|t| self.window.column(t, 1).is_some())
                || item
                    .timing
                    .interval()
                    .is_some_and(|(start, end)| start < self.window.end && end > self.window.start);
            if !visible && let Some(time) = coordinates[0] {
                self.window = TimeWindow::shifted(
                    i128::from(time) - self.window.span() / 2,
                    self.window.span(),
                );
            }
        }
        self.ensure_visible();
    }

    /// Reconcile scrolling after navigation without requiring a render first.
    fn ensure_visible(&mut self) {
        self.scroll = self.effective_scroll(self.viewport_height);
    }
}

/// Append precomputed lanes for one group, preserving supplied track order.
fn append_tracks(rows: &mut Vec<Row>, lanes: &[Vec<Lane>], tracks: Option<&Vec<usize>>) {
    if let Some(tracks) = tracks {
        for &track in tracks {
            for lane in 0..lanes[track].len() {
                rows.push(Row::Lane { track, lane });
            }
        }
    }
}

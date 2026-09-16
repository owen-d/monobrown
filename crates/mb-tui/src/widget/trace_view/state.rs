//! Trace projection over the shared hierarchy navigation engine.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use crossterm::event::KeyEvent;

use super::data::{
    GroupId, ItemId, TraceData, TraceError, TraceItem, TraceTiming, TraceTrack, TraceVisualRole,
    TrackId,
};
use super::layout::TimeWindow;
use crate::input::KeyResult;
use crate::theme;
use crate::widget::flame_graph::{
    CostBreakdown, CostType, CursorNavigation, FlameGraph, SpanId, SpanNode, VerticalNavigation,
};

const ROLE_COUNT: usize = 5;

#[derive(Clone, Copy)]
pub(crate) enum TraceSpan {
    Root,
    Group(GroupId),
    Track(TrackId),
    Item(ItemId),
}

/// A trace hierarchy with shared tree navigation and animated disclosure.
#[derive(Clone)]
pub struct TraceView {
    data: TraceData,
    graph: FlameGraph,
    spans: HashMap<SpanId, TraceSpan>,
    item_spans: BTreeMap<ItemId, SpanId>,
    group_spans: BTreeMap<GroupId, SpanId>,
    track_spans: BTreeMap<TrackId, SpanId>,
    window: TimeWindow,
    details_expanded: bool,
}

impl TraceView {
    /// Admit bounded display facts and project them into a stable hierarchy.
    ///
    /// # Errors
    /// Rejects the invalid identities and bounds documented on [`TraceData`].
    pub fn new(data: TraceData) -> Result<Self, TraceError> {
        data.validate()?;
        let window = TimeWindow::fit(&data);
        let mut projection = Projection::default();
        let root = projection.root(&data);
        let mut graph = FlameGraph::new(root, role_cost_types());
        graph.set_cursor_navigation(CursorNavigation::PreserveExpansion);
        graph.set_vertical_navigation(VerticalNavigation::Siblings);
        Ok(Self {
            data,
            graph,
            spans: projection.spans,
            item_spans: projection.item_spans,
            group_spans: projection.group_spans,
            track_spans: projection.track_spans,
            window,
            details_expanded: false,
        })
    }

    /// Borrow the admitted renderer-independent trace facts.
    pub fn data(&self) -> &TraceData {
        &self.data
    }

    /// Advance the same expand/collapse transitions used by Descendit.
    pub fn tick(&mut self, elapsed: Duration) {
        self.graph.tick(elapsed);
    }

    /// Whether a terminal event loop should continue animation redraws.
    pub fn needs_idle_render(&self) -> bool {
        self.graph.needs_idle_render()
    }

    /// Whether a two-key mark command is waiting for its name.
    pub fn mark_pending(&self) -> bool {
        self.graph.mark_pending()
    }

    /// Dispatch navigation through the shared flame-graph interaction model.
    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
        if key.code == crossterm::event::KeyCode::Enter
            && !key.modifiers.intersects(
                crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT,
            )
        {
            self.details_expanded = !self.details_expanded;
            return KeyResult::Consumed;
        }
        self.graph.handle_key(key)
    }

    /// Return the selected trace item, or none for structural rows.
    pub fn selected_item(&self) -> Option<&TraceItem> {
        let TraceSpan::Item(id) = self.selected_span()? else {
            return None;
        };
        self.data
            .tracks
            .iter()
            .flat_map(|track| &track.items)
            .find(|item| item.id == id)
    }

    /// Return the selected track for either its header or one of its items.
    pub fn selected_track(&self) -> Option<&TraceTrack> {
        match self.selected_span()? {
            TraceSpan::Track(id) => self.data.tracks.iter().find(|track| track.id == id),
            TraceSpan::Item(id) => self
                .data
                .tracks
                .iter()
                .find(|track| track.items.iter().any(|item| item.id == id)),
            TraceSpan::Root | TraceSpan::Group(_) => None,
        }
    }

    /// Return the selected group for a group, track, or event row.
    pub fn selected_group(&self) -> Option<GroupId> {
        match self.selected_span()? {
            TraceSpan::Group(id) => Some(id),
            TraceSpan::Track(id) => self
                .data
                .tracks
                .iter()
                .find(|track| track.id == id)
                .and_then(|track| track.group),
            TraceSpan::Item(id) => self
                .data
                .tracks
                .iter()
                .find(|track| track.items.iter().any(|item| item.id == id))
                .and_then(|track| track.group),
            TraceSpan::Root => None,
        }
    }

    /// Select and reveal one event by stable trace identity.
    pub fn select_item(&mut self, id: ItemId) -> bool {
        self.item_spans
            .get(&id)
            .copied()
            .is_some_and(|span| self.graph.select_span(span))
    }

    /// Current admitted time range.
    pub fn time_window(&self) -> (i64, i64) {
        (self.window.start, self.window.end)
    }

    /// Set an explicit nonempty trace range.
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

    /// Restore the range containing every supplied coordinate.
    pub fn fit_time(&mut self) {
        self.window = TimeWindow::fit(&self.data);
    }

    /// Whether the group is outside the currently expanded navigation path.
    pub fn is_group_collapsed(&self, id: GroupId) -> bool {
        self.group_spans
            .get(&id)
            .is_some_and(|span| !self.graph.is_expanded(*span))
    }

    /// Expand or collapse a group through the shared hierarchy state.
    pub fn set_group_collapsed(&mut self, id: GroupId, collapsed: bool) -> bool {
        let Some(&group_span) = self.group_spans.get(&id) else {
            return false;
        };
        let target = if collapsed {
            group_span
        } else {
            self.data
                .tracks
                .iter()
                .find(|track| track.group == Some(id))
                .and_then(|track| self.track_spans.get(&track.id))
                .copied()
                .unwrap_or(group_span)
        };
        self.graph.select_span(target)
    }

    /// Current visible-row cursor.
    pub fn cursor(&self) -> usize {
        self.graph.cursor()
    }

    /// Number of rows after hierarchy expansion and legend insertion.
    pub fn visible_row_count(&self) -> usize {
        self.graph.visible_rows().len()
    }

    /// Current vertical scroll offset.
    pub fn scroll_offset(&self) -> usize {
        self.graph.scroll_offset()
    }

    /// Whether the selected row's details are expanded.
    pub fn details_expanded(&self) -> bool {
        self.details_expanded
    }

    /// Toggle the selected row's details from an application shell.
    pub fn set_details_expanded(&mut self, expanded: bool) {
        self.details_expanded = expanded;
    }

    /// Visible hierarchy rows for the waterfall renderer.
    pub(crate) fn visible_rows(&self) -> Vec<crate::widget::flame_graph::FlameRow> {
        self.graph.visible_rows()
    }

    /// Resolve a renderer span identity back to trace meaning.
    pub(crate) fn trace_span(&self, span: SpanId) -> Option<TraceSpan> {
        self.spans.get(&span).copied()
    }

    /// Return the current navigation selection's internal span identity.
    pub(crate) fn selected_span_id(&self) -> Option<SpanId> {
        self.graph.selected_span()
    }

    /// Update the shared navigation viewport for keyboard scrolling.
    pub(crate) fn set_viewport_height(&mut self, height: u16) {
        self.graph.set_viewport_height(height);
    }

    /// Whether a structural span currently exposes its children.
    pub(crate) fn is_span_expanded(&self, span: SpanId) -> bool {
        self.graph.is_expanded(span)
    }

    /// Resolve a trace row to the shared navigation span.
    pub(crate) fn span_id_for(&self, span: TraceSpan) -> Option<SpanId> {
        match span {
            TraceSpan::Root => self
                .spans
                .iter()
                .find_map(|(id, value)| matches!(value, TraceSpan::Root).then_some(*id)),
            TraceSpan::Group(id) => self.group_spans.get(&id).copied(),
            TraceSpan::Track(id) => self.track_spans.get(&id).copied(),
            TraceSpan::Item(id) => self.item_spans.get(&id).copied(),
        }
    }

    fn selected_span(&self) -> Option<TraceSpan> {
        self.graph
            .selected_span()
            .and_then(|span| self.spans.get(&span).copied())
    }
}

#[derive(Default)]
struct Projection {
    next_span: u32,
    spans: HashMap<SpanId, TraceSpan>,
    item_spans: BTreeMap<ItemId, SpanId>,
    group_spans: BTreeMap<GroupId, SpanId>,
    track_spans: BTreeMap<TrackId, SpanId>,
}

impl Projection {
    fn root(&mut self, data: &TraceData) -> SpanNode {
        let mut children = Vec::new();
        for group in &data.groups {
            let tracks: Vec<_> = data
                .tracks
                .iter()
                .filter(|track| track.group == Some(group.id))
                .collect();

            // The storage adapter represents a root track as both the group
            // label and the group's first track. Keep the group as the
            // structural row, but render that source track only once by
            // placing its children directly under the group.
            let root_track = tracks.iter().position(|track| {
                track.details.iter().any(|(name, _)| name == "query id")
                    || track.label == group.label
            });
            let mut group_children = Vec::new();
            for (index, track) in tracks.iter().enumerate() {
                if Some(index) == root_track {
                    group_children.extend(self.track_children(track, data));
                } else {
                    group_children.push(self.track(track, data));
                }
            }
            let node = self.branch(
                group.label.clone(),
                TraceSpan::Group(group.id),
                group_children,
            );
            self.group_spans.insert(group.id, node.id);
            if let Some(index) = root_track {
                self.track_spans.insert(tracks[index].id, node.id);
            }
            children.push(node);
        }
        children.extend(
            data.tracks
                .iter()
                .filter(|track| track.group.is_none())
                .map(|track| self.track(track, data)),
        );
        self.branch("trace".into(), TraceSpan::Root, children)
    }

    fn track(&mut self, track: &TraceTrack, data: &TraceData) -> SpanNode {
        let children = self.track_children(track, data);
        let node = self.branch(track.label.clone(), TraceSpan::Track(track.id), children);
        self.track_spans.insert(track.id, node.id);
        node
    }

    fn track_children(&mut self, track: &TraceTrack, data: &TraceData) -> Vec<SpanNode> {
        let mut items: Vec<_> = track.items.iter().collect();
        items.sort_by_key(|item| {
            let first = item.timing.coordinates().into_iter().flatten().min();
            (first.is_none(), first, item.id)
        });
        items
            .into_iter()
            .map(|item| self.item(item, data))
            .collect()
    }

    fn item(&mut self, item: &TraceItem, data: &TraceData) -> SpanNode {
        let role = data
            .categories
            .iter()
            .find(|category| category.id == item.category)
            .map_or(TraceVisualRole::Neutral, |category| category.role);
        let mut amounts = vec![0.0; ROLE_COUNT];
        amounts[role_index(role)] = timing_weight(item.timing);
        let node = self.node(
            item.label.clone(),
            TraceSpan::Item(item.id),
            amounts,
            vec![],
        );
        self.item_spans.insert(item.id, node.id);
        node
    }

    fn branch(&mut self, label: String, span: TraceSpan, children: Vec<SpanNode>) -> SpanNode {
        let mut amounts = vec![0.0; ROLE_COUNT];
        for child in &children {
            for (total, child) in amounts.iter_mut().zip(&child.costs.amounts) {
                *total += child;
            }
        }
        if amounts.iter().all(|amount| *amount == 0.0) {
            amounts[0] = 1.0;
        }
        self.node(label, span, amounts, children)
    }

    fn node(
        &mut self,
        label: String,
        span: TraceSpan,
        amounts: Vec<f64>,
        children: Vec<SpanNode>,
    ) -> SpanNode {
        let id = SpanId(self.next_span);
        self.next_span += 1;
        self.spans.insert(id, span);
        SpanNode {
            id,
            label,
            costs: CostBreakdown { amounts },
            children,
        }
    }
}

fn timing_weight(timing: TraceTiming) -> f64 {
    match timing {
        TraceTiming::Interval { start, end } if start < end => {
            (i128::from(end) - i128::from(start)) as f64
        }
        TraceTiming::Instant(_)
        | TraceTiming::Interval { .. }
        | TraceTiming::MissingStart { .. }
        | TraceTiming::MissingEnd { .. }
        | TraceTiming::Untimed => 1.0,
    }
}

const fn role_index(role: TraceVisualRole) -> usize {
    match role {
        TraceVisualRole::Neutral => 0,
        TraceVisualRole::Scheduled => 1,
        TraceVisualRole::Success => 2,
        TraceVisualRole::Warning => 3,
        TraceVisualRole::Failure => 4,
    }
}

fn role_cost_types() -> Vec<CostType> {
    vec![
        CostType {
            name: "activity",
            color: theme::dim(),
        },
        CostType {
            name: "scheduled",
            color: theme::focus(),
        },
        CostType {
            name: "success",
            color: theme::success(),
        },
        CostType {
            name: "warning",
            color: theme::warning(),
        },
        CostType {
            name: "failure",
            color: theme::error(),
        },
    ]
}

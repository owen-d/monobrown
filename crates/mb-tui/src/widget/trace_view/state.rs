//! Trace projection over the established flame-graph presentation engine.

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
use crate::widget::flame_graph::{CostBreakdown, CostType, FlameGraph, SpanId, SpanNode};

const ROLE_COUNT: usize = 5;

#[derive(Clone, Copy)]
enum TraceSpan {
    Root,
    Group(GroupId),
    Track(TrackId),
    Item(ItemId),
}

/// A trace hierarchy presented by the same tree, navigation, and animation
/// engine as Descendit's flame graph.
#[derive(Clone)]
pub struct TraceView {
    data: TraceData,
    graph: FlameGraph,
    spans: HashMap<SpanId, TraceSpan>,
    item_spans: BTreeMap<ItemId, SpanId>,
    group_spans: BTreeMap<GroupId, SpanId>,
    track_spans: BTreeMap<TrackId, SpanId>,
    window: TimeWindow,
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
        let graph = FlameGraph::new(root, role_cost_types());
        Ok(Self {
            data,
            graph,
            spans: projection.spans,
            item_spans: projection.item_spans,
            group_spans: projection.group_spans,
            track_spans: projection.track_spans,
            window,
        })
    }

    /// Borrow the admitted renderer-independent trace facts.
    pub fn data(&self) -> &TraceData {
        &self.data
    }

    /// Borrow the shared flame-graph presentation state.
    pub(crate) fn graph(&self) -> &FlameGraph {
        &self.graph
    }

    /// Mutably borrow the shared presentation state for viewport-aware render.
    pub(crate) fn graph_mut(&mut self) -> &mut FlameGraph {
        &mut self.graph
    }

    /// Advance the same expand/collapse transitions used by Descendit.
    pub fn tick(&mut self, elapsed: Duration) {
        self.graph.tick(elapsed);
    }

    /// Whether a terminal event loop should continue animation redraws.
    pub fn needs_idle_render(&self) -> bool {
        self.graph.needs_idle_render()
    }

    /// Dispatch navigation through the shared flame-graph interaction model.
    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
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
            let tracks = data
                .tracks
                .iter()
                .filter(|track| track.group == Some(group.id))
                .map(|track| self.track(track, data))
                .collect();
            let node = self.branch(group.label.clone(), TraceSpan::Group(group.id), tracks);
            self.group_spans.insert(group.id, node.id);
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
        let mut items: Vec<_> = track.items.iter().collect();
        items.sort_by_key(|item| {
            let first = item.timing.coordinates().into_iter().flatten().min();
            (first.is_none(), first, item.id)
        });
        let children = items
            .into_iter()
            .map(|item| self.item(item, data))
            .collect();
        let node = self.branch(track.label.clone(), TraceSpan::Track(track.id), children);
        self.track_spans.insert(track.id, node.id);
        node
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

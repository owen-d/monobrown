//! Normalized diagram specs and graph adapter traits.
//!
//! This module owns the pure data boundary between caller-owned graph state and
//! renderer-owned layout policy. It validates just enough to make rendering
//! deterministic without pretending to be a lossless graph exchange format.
//! Graphs and row/span diagrams are separate blocks because they protect
//! different invariants.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{DiagramError, EdgeEndpoint};

/// Adapts caller-owned graph types into deterministic diagram input.
///
/// The trait exists so consumers do not have to copy their graph into this
/// crate's owned graph first. It should describe graph identity, labels, and
/// edges only; renderer choice and text layout must stay outside the adapter.
pub trait GraphDiagram {
    /// Stable node identity used only during normalization.
    ///
    /// The id is not rendered directly. Requiring `Ord` lets the crate produce
    /// deterministic output even when the caller's storage order is incidental.
    type NodeId: Clone + Ord;

    /// Returns every node that may be referenced by an edge.
    ///
    /// Implementations should not return duplicates; duplicates are treated as
    /// adapter bugs because they make generated ids ambiguous.
    fn nodes(&self) -> Vec<Self::NodeId>;

    /// Returns the human-facing label for a node.
    ///
    /// Labels are intentionally owned strings so the adapter boundary stays
    /// lifetime-simple for comments, tests, and docs.
    fn node_label(&self, node: &Self::NodeId) -> String;

    /// Returns directed edges between nodes returned by [`GraphDiagram::nodes`].
    ///
    /// The order is normalized by this crate, so implementations can expose
    /// their natural storage order without affecting snapshots.
    fn edges(&self) -> Vec<GraphEdge<Self::NodeId>>;
}

/// Edge data returned by a graph adapter before normalization.
///
/// It carries caller node ids so adapters can stay close to their native graph
/// model. It should not grow renderer-specific fields; visual policy belongs in
/// renderers or future text-diagram primitives.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GraphEdge<NodeId> {
    /// Caller-owned source node identity.
    pub source: NodeId,
    /// Caller-owned target node identity.
    pub target: NodeId,
    /// Optional semantic edge label, independent from arrow styling.
    pub label: Option<String>,
}

impl<NodeId> GraphEdge<NodeId> {
    /// Creates an unlabeled adapter edge.
    ///
    /// This is the common case for dependency graphs where the relation is
    /// implied by the graph itself.
    pub fn new(source: NodeId, target: NodeId) -> Self {
        Self {
            source,
            target,
            label: None,
        }
    }

    /// Creates a labeled adapter edge.
    ///
    /// Labels remain semantic data. Renderers decide whether that label becomes
    /// inline arrow text, a side annotation, or something else.
    pub fn labeled(source: NodeId, target: NodeId, label: impl Into<String>) -> Self {
        Self {
            source,
            target,
            label: Some(label.into()),
        }
    }
}

/// Stable node id inside a normalized diagram spec.
///
/// The newtype prevents renderer internals from confusing display labels with
/// lookup identity. It should not be used as caller graph identity; adapters
/// have their own `NodeId` for that.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DiagramNodeId(String);

impl DiagramNodeId {
    /// Creates a manual spec node id.
    ///
    /// Empty ids are rejected so missing endpoint errors never need to display
    /// an ambiguous placeholder.
    pub fn new(value: impl Into<String>) -> Result<Self, DiagramError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DiagramError::EmptyNodeId);
        }
        Ok(Self(value))
    }

    /// Returns the stable id text used for internal spec references.
    ///
    /// Renderers should prefer labels for display and use ids only as a
    /// fallback for structurally impossible missing labels.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    // Generated ids are deliberately boring; their only job is stable lookup.
    fn generated(index: u32) -> Self {
        Self(format!("n{index}"))
    }
}

/// A normalized node ready for renderer lookup.
///
/// `NodeSpec` separates stable identity from display text. Do not add position
/// or style here; those belong to layout primitives or renderers.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NodeSpec {
    id: DiagramNodeId,
    label: String,
}

impl NodeSpec {
    /// Creates a normalized node with explicit identity and display text.
    ///
    /// Manual specs use this when the caller already owns stable text ids.
    pub fn new(id: DiagramNodeId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
        }
    }

    /// Returns the node identity used by edges.
    ///
    /// This protects renderers from deriving identity from human-facing labels.
    pub fn id(&self) -> &DiagramNodeId {
        &self.id
    }

    /// Returns the human-facing node text.
    ///
    /// Labels may repeat; callers should not rely on them for graph identity.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A normalized edge ready for deterministic text rendering.
///
/// Edges reference [`DiagramNodeId`] rather than caller graph ids. This keeps
/// renderer lookup independent from custom graph storage and lifetimes.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EdgeSpec {
    source: DiagramNodeId,
    target: DiagramNodeId,
    label: Option<String>,
}

impl EdgeSpec {
    /// Creates an unlabeled normalized edge.
    ///
    /// `GraphSpec::new` validates that both endpoints exist in the node set.
    pub fn new(source: DiagramNodeId, target: DiagramNodeId) -> Self {
        Self {
            source,
            target,
            label: None,
        }
    }

    /// Creates a labeled normalized edge.
    ///
    /// The label is semantic edge text; renderers own arrow shape and spacing.
    pub fn labeled(source: DiagramNodeId, target: DiagramNodeId, label: impl Into<String>) -> Self {
        Self {
            source,
            target,
            label: Some(label.into()),
        }
    }

    /// Returns the source node id.
    ///
    /// Renderer code uses this for lookup, never for display unless the spec is
    /// somehow internally inconsistent.
    pub fn source(&self) -> &DiagramNodeId {
        &self.source
    }

    /// Returns the target node id.
    ///
    /// Keeping this as an id preserves the distinction between graph structure
    /// and rendered text.
    pub fn target(&self) -> &DiagramNodeId {
        &self.target
    }

    /// Returns optional semantic edge text.
    ///
    /// `None` means the edge relation is clear without extra annotation.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// A deterministic graph block inside a text diagram.
///
/// The spec is intentionally a lowest-common-denominator graph: nodes, directed
/// edges, labels, and validation. It should not absorb DOT/Mermaid-only
/// concepts such as subgraph styling, rank constraints, or hyperlink metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSpec {
    nodes: Vec<NodeSpec>,
    edges: Vec<EdgeSpec>,
}

impl GraphSpec {
    /// Creates a validated graph spec from normalized nodes and edges.
    ///
    /// Validation protects the renderer boundary: duplicate nodes and dangling
    /// edges are construction errors, not rendering decisions.
    pub fn new(mut nodes: Vec<NodeSpec>, mut edges: Vec<EdgeSpec>) -> Result<Self, DiagramError> {
        let mut ids = BTreeSet::new();
        for node in &nodes {
            if !ids.insert(node.id().clone()) {
                return Err(DiagramError::DuplicateNodeId {
                    id: node.id().as_str().to_owned(),
                });
            }
        }

        for edge in &edges {
            validate_endpoint(&ids, edge.source(), EdgeEndpoint::Source)?;
            validate_endpoint(&ids, edge.target(), EdgeEndpoint::Target)?;
        }

        nodes.sort();
        edges.sort();
        Ok(Self { nodes, edges })
    }

    /// Normalizes any graph adapter into a render-ready graph spec.
    ///
    /// Generated ids come from sorted adapter node ids. That makes output
    /// stable without requiring caller ids to be printable.
    pub fn from_graph<G>(graph: &G) -> Result<Self, DiagramError>
    where
        G: GraphDiagram,
    {
        let mut adapter_nodes = graph.nodes();
        adapter_nodes.sort();

        let mut ids_by_node = BTreeMap::new();
        let mut spec_nodes = Vec::new();
        let mut next_index = 0u32;
        let mut previous = None;

        for node in adapter_nodes {
            if previous.as_ref().is_some_and(|seen| seen == &node) {
                return Err(DiagramError::DuplicateGraphNode);
            }
            previous = Some(node.clone());

            let id = DiagramNodeId::generated(next_index);
            next_index = next_index
                .checked_add(1)
                .ok_or(DiagramError::TooManyNodes)?;
            ids_by_node.insert(node.clone(), id.clone());
            spec_nodes.push(NodeSpec::new(id, graph.node_label(&node)));
        }

        let mut adapter_edges = graph.edges();
        adapter_edges.sort();
        let mut spec_edges = Vec::new();

        for edge in adapter_edges {
            let GraphEdge {
                source,
                target,
                label,
            } = edge;
            let Some(source_id) = ids_by_node.get(&source) else {
                return Err(DiagramError::MissingGraphEdgeEndpoint {
                    endpoint: EdgeEndpoint::Source,
                });
            };
            let Some(target_id) = ids_by_node.get(&target) else {
                return Err(DiagramError::MissingGraphEdgeEndpoint {
                    endpoint: EdgeEndpoint::Target,
                });
            };

            match label {
                Some(label) => {
                    spec_edges.push(EdgeSpec::labeled(
                        source_id.clone(),
                        target_id.clone(),
                        label,
                    ));
                }
                None => {
                    spec_edges.push(EdgeSpec::new(source_id.clone(), target_id.clone()));
                }
            }
        }

        Self::new(spec_nodes, spec_edges)
    }

    /// Returns nodes in deterministic id order.
    ///
    /// Renderers rely on this for snapshot-stable output.
    pub fn nodes(&self) -> &[NodeSpec] {
        &self.nodes
    }

    /// Returns edges in deterministic endpoint and label order.
    ///
    /// Renderers should not reorder edges unless they own a stronger layout
    /// policy.
    pub fn edges(&self) -> &[EdgeSpec] {
        &self.edges
    }

    /// Looks up display text for a normalized node id.
    ///
    /// This centralizes the id-to-label boundary so renderers do not duplicate
    /// search policy.
    pub fn node_label(&self, id: &DiagramNodeId) -> Option<&str> {
        for node in &self.nodes {
            if node.id() == id {
                return Some(node.label());
            }
        }
        None
    }
}

/// Positive column span for row-based text diagrams.
///
/// The newtype prevents zero-width cells from reaching renderers. It is a
/// logical span, not a terminal-cell width; renderers decide how many
/// characters each span unit receives.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CellSpan(u16);

impl CellSpan {
    /// Creates a positive logical cell span.
    ///
    /// Zero is rejected because a zero-span cell cannot preserve labels,
    /// frames, or deterministic layout.
    pub fn new(units: u16) -> Result<Self, DiagramError> {
        if units == 0 {
            return Err(DiagramError::InvalidCellSpan { units });
        }
        Ok(Self(units))
    }

    /// Returns a one-unit span for ordinary cells.
    ///
    /// This keeps common rows terse while still making span semantics explicit.
    pub fn one() -> Self {
        Self(1)
    }

    /// Returns the logical span units.
    ///
    /// Renderers use this for proportional fixed-width allocation.
    pub fn units(self) -> u16 {
        self.0
    }
}

impl Default for CellSpan {
    fn default() -> Self {
        Self::one()
    }
}

/// Visual stack depth for a row cell.
///
/// Depth hints that a component has overlapping replicas without making the
/// diagram own deployment truth. Use the label for exact counts such as
/// `web pods x3`; use depth only for the rendered overlap cue.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CellDepth(u8);

impl CellDepth {
    /// Maximum visual layers before the overlap cue becomes noisy.
    pub const MAX_LAYERS: u8 = 4;

    /// Creates a visual cell depth.
    ///
    /// Values above [`CellDepth::MAX_LAYERS`] are rejected because text overlap
    /// is a hint, not a scalable replica renderer.
    pub fn new(layers: u8) -> Result<Self, DiagramError> {
        if (1..=Self::MAX_LAYERS).contains(&layers) {
            return Ok(Self(layers));
        }
        Err(DiagramError::InvalidCellDepth {
            layers,
            max: Self::MAX_LAYERS,
        })
    }

    /// Returns the default single visual layer.
    ///
    /// This keeps ordinary cells visually flat unless callers opt into overlap.
    pub fn one() -> Self {
        Self(1)
    }

    /// Returns the visual layer count.
    ///
    /// Renderers use this to draw small offset shadows behind a cell.
    pub fn layers(self) -> u8 {
        self.0
    }
}

impl Default for CellDepth {
    fn default() -> Self {
        Self::one()
    }
}

/// Horizontal alignment for cell labels.
///
/// Alignment is text-native presentation policy. It belongs on cells rather
/// than graph nodes because row/span diagrams often carry centered annotations
/// and right-aligned counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CellAlign {
    /// Left alignment is useful for names and prose labels.
    Left,
    /// Center alignment is the default for boxed spans and annotations.
    #[default]
    Center,
    /// Right alignment is useful for numeric labels.
    Right,
}

/// Repeat-fill style for cells that represent ranges, blocks, or annotations.
///
/// Fill is a small text-native primitive for diagrams such as compaction maps.
/// It should not be expanded into colors, CSS, or renderer-specific styling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellFill {
    /// Dense block fill for a primary range.
    Solid,
    /// Medium block fill for a secondary range.
    Medium,
    /// Light block fill for a tertiary range.
    Light,
    /// Rule fill for interval annotations.
    Dash,
}

/// One cell in a row/span diagram.
///
/// A cell owns semantic text, logical span, alignment, and optional fill. It
/// does not own absolute coordinates; renderers derive positions from row order
/// and span units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellSpec {
    label: String,
    span: CellSpan,
    depth: CellDepth,
    align: CellAlign,
    fill: Option<CellFill>,
}

impl CellSpec {
    /// Creates a labeled cell with a logical span.
    ///
    /// Use this for headers, annotations, and boxed labels. Empty labels are
    /// valid for spacer cells because blank rows are still meaningful diagrams.
    pub fn new(label: impl Into<String>, span: CellSpan) -> Self {
        Self {
            label: label.into(),
            span,
            depth: CellDepth::default(),
            align: CellAlign::default(),
            fill: None,
        }
    }

    /// Creates a fill-only cell.
    ///
    /// Fill-only cells model data ranges without inventing fake labels or graph
    /// nodes.
    pub fn filled(span: CellSpan, fill: CellFill) -> Self {
        Self {
            label: String::new(),
            span,
            depth: CellDepth::default(),
            align: CellAlign::default(),
            fill: Some(fill),
        }
    }

    /// Returns a copy with visual stack depth applied.
    ///
    /// Depth should not be used as the authoritative replica count. It only
    /// tells renderers to draw overlapping layers behind the cell.
    pub fn with_depth(mut self, depth: CellDepth) -> Self {
        self.depth = depth;
        self
    }

    /// Returns a copy with a different alignment.
    ///
    /// This keeps cell construction immutable and avoids a builder object for
    /// the first slice.
    pub fn with_align(mut self, align: CellAlign) -> Self {
        self.align = align;
        self
    }

    /// Returns a copy with fill applied behind or instead of the label.
    ///
    /// A filled labeled cell is useful for dashed interval annotations where
    /// the rule is part of the text-native meaning.
    pub fn with_fill(mut self, fill: CellFill) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Returns the cell label.
    ///
    /// Renderers may truncate this to satisfy fixed-width output.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the logical span.
    ///
    /// Span is the cell's contribution to row layout.
    pub fn span(&self) -> CellSpan {
        self.span
    }

    /// Returns the visual stack depth.
    ///
    /// A depth of one means the cell renders as a flat box.
    pub fn depth(&self) -> CellDepth {
        self.depth
    }

    /// Returns the label alignment.
    ///
    /// Alignment is local to the cell and independent of row framing.
    pub fn align(&self) -> CellAlign {
        self.align
    }

    /// Returns optional fill semantics.
    ///
    /// Fill is renderer-neutral; ASCII and Unicode renderers map it to their
    /// own glyph sets.
    pub fn fill(&self) -> Option<CellFill> {
        self.fill
    }
}

/// Frame policy for a row in a row/span diagram.
///
/// Frames are row-local because compaction-style diagrams mix boxed bands,
/// dashed annotations, and blank spacer rows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowFrame {
    /// Render the row as a boxed band.
    #[default]
    Boxed,
    /// Render only cell content, preserving spans without a surrounding box.
    Plain,
}

/// One horizontal band in a row/span diagram.
///
/// Rows provide ordering and frame policy. They do not store a computed width;
/// width is derived by renderers from the containing [`RowsSpec`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowSpec {
    cells: Vec<CellSpec>,
    frame: RowFrame,
}

impl RowSpec {
    /// Creates a boxed row from cells.
    ///
    /// Boxed is the default because row/span diagrams usually need visible
    /// boundaries to communicate grouping.
    pub fn new(cells: Vec<CellSpec>) -> Self {
        Self {
            cells,
            frame: RowFrame::default(),
        }
    }

    /// Creates an unboxed row from cells.
    ///
    /// Plain rows model annotations and spacer bands without pretending they
    /// are graph edges or framed records.
    pub fn plain(cells: Vec<CellSpec>) -> Self {
        Self {
            cells,
            frame: RowFrame::Plain,
        }
    }

    /// Returns a copy with explicit frame policy.
    ///
    /// This keeps framing orthogonal to cell content.
    pub fn with_frame(mut self, frame: RowFrame) -> Self {
        self.frame = frame;
        self
    }

    /// Returns row cells in caller-defined order.
    ///
    /// Cell order is semantic because it defines horizontal layout.
    pub fn cells(&self) -> &[CellSpec] {
        &self.cells
    }

    /// Returns row frame policy.
    ///
    /// Renderers use this to decide whether to emit borders around the row.
    pub fn frame(&self) -> RowFrame {
        self.frame
    }

    /// Returns the logical span total for this row.
    ///
    /// The value is derived, not stored, so it cannot drift from cell spans.
    pub fn span_total(&self) -> Result<u32, DiagramError> {
        let mut total = 0u32;
        for cell in &self.cells {
            total = total
                .checked_add(u32::from(cell.span().units()))
                .ok_or(DiagramError::RowSpanOverflow)?;
        }
        Ok(total)
    }
}

/// A text-native row/span diagram block.
///
/// `RowsSpec` models the compaction-style class of diagrams: rows, cells,
/// spans, labels, annotations, fills, and frames. It deliberately does not own
/// graph identity or edge semantics. The max span is stored after validation so
/// renderers do not need to handle impossible row arithmetic failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowsSpec {
    rows: Vec<RowSpec>,
    span_total_max: u32,
}

impl RowsSpec {
    /// Creates a row/span diagram from ordered rows.
    ///
    /// Construction validates span arithmetic once so renderers can stay
    /// focused on presentation.
    pub fn new(rows: Vec<RowSpec>) -> Result<Self, DiagramError> {
        let mut span_total_max = 0u32;
        for row in &rows {
            if row.cells().is_empty() {
                return Err(DiagramError::EmptyRow);
            }
            span_total_max = span_total_max.max(row.span_total()?);
        }
        Ok(Self {
            rows,
            span_total_max,
        })
    }

    /// Returns rows in caller-defined order.
    ///
    /// Row order is the vertical layout of the diagram.
    pub fn rows(&self) -> &[RowSpec] {
        &self.rows
    }

    /// Returns the widest logical row span.
    ///
    /// Renderers use the maximum span as the shared grid so rows with different
    /// cell counts still align proportionally.
    pub fn span_total_max(&self) -> u32 {
        self.span_total_max
    }
}

/// One renderable block inside a text diagram.
///
/// The enum exists so graph rendering can grow beside other text primitives
/// later. It should not become a universal diagram IR; add variants only when
/// they represent text-native primitives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagramBlock {
    /// A deterministic graph block normalized from an adapter or owned graph.
    Graph(GraphSpec),
    /// A text-native row/span block for boxed bands and annotations.
    Rows(RowsSpec),
}

/// A complete text-first diagram spec.
///
/// `DiagramSpec` owns renderable blocks and nothing about renderer policy. This
/// keeps ASCII fallback, Unicode box drawing, and future emitters independent
/// from graph adaptation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagramSpec {
    blocks: Vec<DiagramBlock>,
}

impl DiagramSpec {
    /// Creates a diagram from already-normalized blocks.
    ///
    /// This is the manual escape hatch for callers that want to assemble specs
    /// directly without implementing [`GraphDiagram`].
    pub fn new(blocks: Vec<DiagramBlock>) -> Self {
        Self { blocks }
    }

    /// Creates a one-block diagram from a graph adapter.
    ///
    /// This is the ergonomic path for existing graph types: implement the
    /// adapter, choose a renderer, and keep layout concerns out of the graph.
    pub fn from_graph<G>(graph: &G) -> Result<Self, DiagramError>
    where
        G: GraphDiagram,
    {
        Ok(Self::from_graph_spec(GraphSpec::from_graph(graph)?))
    }

    /// Creates a one-block diagram from a normalized graph spec.
    ///
    /// This supports direct spec construction without forcing callers through
    /// a custom adapter type.
    pub fn from_graph_spec(graph: GraphSpec) -> Self {
        Self {
            blocks: vec![DiagramBlock::Graph(graph)],
        }
    }

    /// Creates a one-block diagram from a row/span spec.
    ///
    /// This is the direct path for text-native diagrams that should not be
    /// represented as graphs.
    pub fn from_rows(rows: RowsSpec) -> Self {
        Self {
            blocks: vec![DiagramBlock::Rows(rows)],
        }
    }

    /// Returns renderable blocks in caller-defined order.
    ///
    /// Block order is part of the text artifact and must remain deterministic.
    pub fn blocks(&self) -> &[DiagramBlock] {
        &self.blocks
    }
}

// Endpoint validation belongs at spec construction, not in renderers.
fn validate_endpoint(
    ids: &BTreeSet<DiagramNodeId>,
    id: &DiagramNodeId,
    endpoint: EdgeEndpoint,
) -> Result<(), DiagramError> {
    if ids.contains(id) {
        return Ok(());
    }
    Err(DiagramError::MissingEdgeEndpoint {
        endpoint,
        id: id.as_str().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_adapter_normalizes_nodes_and_edges() -> Result<(), DiagramError> {
        /// Test adapter proving custom graphs need only expose trait data.
        struct ReversedGraph;

        impl GraphDiagram for ReversedGraph {
            type NodeId = u32;

            fn nodes(&self) -> Vec<Self::NodeId> {
                vec![2, 1]
            }

            fn node_label(&self, node: &Self::NodeId) -> String {
                format!("n{node}")
            }

            fn edges(&self) -> Vec<GraphEdge<Self::NodeId>> {
                vec![GraphEdge::labeled(2, 1, "uses")]
            }
        }

        let spec = GraphSpec::from_graph(&ReversedGraph)?;

        assert_eq!(spec.nodes()[0].label(), "n1");
        assert_eq!(spec.nodes()[1].label(), "n2");
        assert_eq!(spec.edges()[0].label(), Some("uses"));
        Ok(())
    }

    #[test]
    fn graph_spec_rejects_dangling_edges() -> Result<(), DiagramError> {
        let source = DiagramNodeId::new("source")?;
        let target = DiagramNodeId::new("target")?;
        let result = GraphSpec::new(
            vec![NodeSpec::new(source.clone(), "source")],
            vec![EdgeSpec::new(source, target)],
        );

        assert_eq!(
            result,
            Err(DiagramError::MissingEdgeEndpoint {
                endpoint: EdgeEndpoint::Target,
                id: "target".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn rows_spec_rejects_empty_rows() {
        let result = RowsSpec::new(vec![RowSpec::new(Vec::new())]);

        assert_eq!(result, Err(DiagramError::EmptyRow));
    }

    #[test]
    fn cell_depth_rejects_unreadable_layer_counts() {
        let result = CellDepth::new(0);

        assert_eq!(
            result,
            Err(DiagramError::InvalidCellDepth {
                layers: 0,
                max: CellDepth::MAX_LAYERS,
            })
        );
    }
}

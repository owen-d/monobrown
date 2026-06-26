//! Error types for protecting deterministic diagram invariants.
//!
//! Errors stay small and structural so callers can decide whether to fix their
//! graph adapter, concrete graph construction, or renderer configuration.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Identifies which side of an edge violated a graph invariant.
///
/// This keeps endpoint validation messages precise without coupling validation
/// to any concrete graph storage type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeEndpoint {
    /// The edge source did not resolve to a declared node.
    Source,
    /// The edge target did not resolve to a declared node.
    Target,
}

/// Reports invalid diagram construction before rendering begins.
///
/// Rendering is deliberately infallible, so all structural failures are caught
/// while building the normalized spec. Do not mix terminal-layout failures or
/// I/O failures into this type; renderers only transform in-memory data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagramError {
    /// Node ids must be non-empty so renderer lookups never need a sentinel.
    EmptyNodeId,
    /// A normalized spec may contain each node id once.
    DuplicateNodeId { id: String },
    /// A manually built spec edge referenced an id outside the node set.
    MissingEdgeEndpoint { endpoint: EdgeEndpoint, id: String },
    /// A graph adapter returned the same node id more than once.
    DuplicateGraphNode,
    /// A graph adapter returned an edge endpoint outside its node list.
    MissingGraphEdgeEndpoint { endpoint: EdgeEndpoint },
    /// More nodes were supplied than the generated id space supports.
    TooManyNodes,
    /// The owned graph rejected an edge endpoint it does not own.
    UnknownConcreteNode { id: u32 },
    /// Renderers need at least one column to produce deterministic text.
    InvalidRenderWidth { columns: u16 },
    /// Cell spans must be positive because zero-width cells cannot render.
    InvalidCellSpan { units: u16 },
    /// Cell depth is visual only and must stay small enough to remain readable.
    InvalidCellDepth { layers: u8, max: u8 },
    /// Rows must contain cells; use a blank cell for intentional whitespace.
    EmptyRow,
    /// Row span arithmetic exceeded the supported text-diagram grid.
    RowSpanOverflow,
}

impl Display for EdgeEndpoint {
    // Keep endpoint text stable because errors are useful in snapshot tests.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source => f.write_str("source"),
            Self::Target => f.write_str("target"),
        }
    }
}

impl Display for DiagramError {
    // Keep messages dependency-free; callers can match variants for structure.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNodeId => f.write_str("node id must not be empty"),
            Self::DuplicateNodeId { id } => {
                write!(f, "duplicate node id `{id}`")
            }
            Self::MissingEdgeEndpoint { endpoint, id } => {
                write!(f, "edge {endpoint} references unknown node `{id}`")
            }
            Self::DuplicateGraphNode => f.write_str("graph adapter returned a duplicate node"),
            Self::MissingGraphEdgeEndpoint { endpoint } => {
                write!(f, "graph adapter returned edge with unknown {endpoint}")
            }
            Self::TooManyNodes => f.write_str("graph contains too many nodes for generated ids"),
            Self::UnknownConcreteNode { id } => {
                write!(f, "concrete graph does not contain node {id}")
            }
            Self::InvalidRenderWidth { columns } => {
                write!(f, "render width must be positive, got {columns}")
            }
            Self::InvalidCellSpan { units } => {
                write!(f, "cell span must be positive, got {units}")
            }
            Self::InvalidCellDepth { layers, max } => {
                write!(f, "cell depth must be between 1 and {max}, got {layers}")
            }
            Self::EmptyRow => f.write_str("row must contain at least one cell"),
            Self::RowSpanOverflow => f.write_str("row span total overflowed"),
        }
    }
}

impl Error for DiagramError {}

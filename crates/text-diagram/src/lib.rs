//! Deterministic text-first diagrams for comments, docs, chats, and snapshots.
//!
//! This crate keeps graph ownership, text-diagram shape, and rendering separate:
//!
//! ```text
//! caller graph -> GraphDiagram -> GraphSpec -┐
//!                                            ├-> DiagramSpec -> DiagramRenderer
//! text rows  -> RowsSpec --------------------┘
//! ```
//!
//! The seam is intentionally smaller than DOT or Mermaid. `DiagramSpec` is a
//! text-diagram model, not a universal graph IR. Graph-shaped inputs are one
//! block in that model; row/span/box diagrams are another block beside it.
//!
//! # Example
//!
//! ```
//! use text_diagram::{
//!     ConcreteGraph, DiagramRenderer, DiagramSpec, UnicodeRenderer,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut graph = ConcreteGraph::new();
//! let input = graph.add_node("input")?;
//! let parse = graph.add_node("parse")?;
//! graph.add_labeled_edge(input, parse, "load")?;
//!
//! let spec = DiagramSpec::from_graph(&graph)?;
//! let rendered = UnicodeRenderer::default().render(&spec);
//!
//! assert!(rendered.contains("input"));
//! assert!(rendered.contains("parse"));
//! # Ok(())
//! # }
//! ```

mod error;
mod graph;
mod render;
mod spec;

pub use crate::error::{DiagramError, EdgeEndpoint};
pub use crate::graph::{ConcreteGraph, ConcreteNodeId};
pub use crate::render::{
    AsciiRenderer, CornerStyle, DepthStyle, DiagramRenderer, RenderConfig, RenderWidth,
    UnicodeRenderer,
};
pub use crate::spec::{
    CellAlign, CellDepth, CellFill, CellSpan, CellSpec, DiagramBlock, DiagramNodeId, DiagramSpec,
    EdgeSpec, GraphDiagram, GraphEdge, GraphSpec, NodeSpec, RowFrame, RowSpec, RowsSpec,
};

//! Adapter from descendit type/trait data into text-diagram graphs.
//!
//! This module keeps graph facts and display policy separate. The optimizer
//! owns `TypeTraitGraphData`; `text-diagram` owns rendering; this wrapper is the
//! narrow translation boundary between those two concepts.

use text_diagram::{
    DiagramError, DiagramRenderer, DiagramSpec, GraphDiagram, GraphEdge, UnicodeRenderer,
};

use super::{ImplEdge, ItemId, TypeTraitGraphData};

/// Read-only diagram view over a type/trait graph.
///
/// The wrapper exists so display decisions do not become part of the serialized
/// optimization graph. Descendit can add label modes or filtering here without
/// changing rewrite semantics or the JSON shape.
#[derive(Clone, Copy, Debug)]
pub struct TypeTraitGraphDiagram<'a> {
    data: &'a TypeTraitGraphData,
}

impl<'a> TypeTraitGraphDiagram<'a> {
    /// Create a diagram adapter for caller-owned graph data.
    ///
    /// The adapter borrows because rendering should not require copying the
    /// optimization graph into `text-diagram`'s concrete graph type.
    pub fn new(data: &'a TypeTraitGraphData) -> Self {
        Self { data }
    }

    /// Render this graph with the default Unicode text renderer.
    ///
    /// This is the human-facing path for reports. Callers that need another
    /// renderer can pass this adapter to `DiagramSpec::from_graph` directly.
    pub fn render_unicode(&self) -> Result<String, DiagramError> {
        let spec = DiagramSpec::from_graph(self)?;
        Ok(UnicodeRenderer::default().render(&spec))
    }
}

impl GraphDiagram for TypeTraitGraphDiagram<'_> {
    type NodeId = ItemId;

    fn nodes(&self) -> Vec<Self::NodeId> {
        self.data
            .types
            .iter()
            .cloned()
            .map(ItemId::Type)
            .chain(self.data.traits.iter().cloned().map(ItemId::Trait))
            .collect()
    }

    fn node_label(&self, node: &Self::NodeId) -> String {
        match node {
            ItemId::Type(ty) => format!("type {ty}"),
            ItemId::Trait(tr) => format!("trait {tr}"),
        }
    }

    fn edges(&self) -> Vec<GraphEdge<Self::NodeId>> {
        self.data
            .impls
            .iter()
            .cloned()
            .map(|edge| match edge {
                ImplEdge::TypeImplsTrait { ty, tr } => {
                    GraphEdge::labeled(ItemId::Type(ty), ItemId::Trait(tr), "impls")
                }
                ImplEdge::TraitImplsTrait { subject, target } => {
                    GraphEdge::labeled(ItemId::Trait(subject), ItemId::Trait(target), "implies")
                }
            })
            .collect()
    }
}

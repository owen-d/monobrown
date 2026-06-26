//! Owned graph type for callers without an existing graph model.
//!
//! The concrete graph is intentionally small. It protects node ownership and
//! endpoint validity, then implements `GraphDiagram` so the rest of the crate
//! treats it like any other supplied graph.

use std::collections::BTreeMap;

use crate::error::DiagramError;
use crate::spec::{GraphDiagram, GraphEdge};

/// Stable node handle allocated by [`ConcreteGraph`].
///
/// The id is opaque so callers cannot forge graph membership. Use it as a
/// handle for adding edges, not as text to render.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConcreteNodeId(u32);

impl ConcreteNodeId {
    /// Returns the numeric id for diagnostics and tests.
    ///
    /// This does not grant graph ownership; `ConcreteGraph` still validates
    /// endpoints before accepting edges.
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Minimal owned graph for direct diagram construction.
///
/// It exists for consumers who want `build graph -> choose renderer` without
/// implementing a trait over their own type. It should not grow layout state;
/// renderers own presentation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConcreteGraph {
    nodes: BTreeMap<ConcreteNodeId, String>,
    edges: Vec<GraphEdge<ConcreteNodeId>>,
    next_node: u32,
}

impl ConcreteGraph {
    /// Creates an empty owned graph.
    ///
    /// The graph stores nodes in a `BTreeMap` so adapter output is stable even
    /// before the normalization step sorts it again.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node and returns its opaque handle.
    ///
    /// Handles protect edge construction from label collisions: multiple nodes
    /// may share the same display label.
    pub fn add_node(&mut self, label: impl Into<String>) -> Result<ConcreteNodeId, DiagramError> {
        let next_node = self
            .next_node
            .checked_add(1)
            .ok_or(DiagramError::TooManyNodes)?;
        let id = ConcreteNodeId(self.next_node);
        self.next_node = next_node;
        self.nodes.insert(id, label.into());
        Ok(id)
    }

    /// Adds an unlabeled directed edge between existing nodes.
    ///
    /// Endpoint validation keeps invalid graph structure out of the adapter
    /// boundary and therefore out of renderer code.
    pub fn add_edge(
        &mut self,
        source: ConcreteNodeId,
        target: ConcreteNodeId,
    ) -> Result<(), DiagramError> {
        self.ensure_node(source)?;
        self.ensure_node(target)?;
        self.edges.push(GraphEdge::new(source, target));
        Ok(())
    }

    /// Adds a labeled directed edge between existing nodes.
    ///
    /// The label describes the relationship; it should not encode arrow style
    /// or renderer-specific spacing.
    pub fn add_labeled_edge(
        &mut self,
        source: ConcreteNodeId,
        target: ConcreteNodeId,
        label: impl Into<String>,
    ) -> Result<(), DiagramError> {
        self.ensure_node(source)?;
        self.ensure_node(target)?;
        self.edges.push(GraphEdge::labeled(source, target, label));
        Ok(())
    }

    // Membership checks belong here because this type owns node allocation.
    fn ensure_node(&self, id: ConcreteNodeId) -> Result<(), DiagramError> {
        if self.nodes.contains_key(&id) {
            return Ok(());
        }
        Err(DiagramError::UnknownConcreteNode { id: id.as_u32() })
    }
}

impl GraphDiagram for ConcreteGraph {
    type NodeId = ConcreteNodeId;

    fn nodes(&self) -> Vec<Self::NodeId> {
        self.nodes.keys().copied().collect()
    }

    fn node_label(&self, node: &Self::NodeId) -> String {
        match self.nodes.get(node) {
            Some(label) => label.clone(),
            None => {
                let id = node.as_u32();
                format!("node {id}")
            }
        }
    }

    fn edges(&self) -> Vec<GraphEdge<Self::NodeId>> {
        self.edges.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_graph_rejects_unknown_endpoint() -> Result<(), DiagramError> {
        let mut graph = ConcreteGraph::new();
        let source = graph.add_node("source")?;
        let result = graph.add_edge(source, ConcreteNodeId(99));

        assert_eq!(result, Err(DiagramError::UnknownConcreteNode { id: 99 }));
        Ok(())
    }
}

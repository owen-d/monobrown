use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

use super::{GraphCost, GraphOp, RewritePlan, TypeTraitGraph, TypeTraitGraphError};

/// Name of a type node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TypeName(String);

impl TypeName {
    /// Build a type name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Borrow the raw name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TypeName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for TypeName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Name of a trait node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TraitName(String);

impl TraitName {
    /// Build a trait name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Borrow the raw name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TraitName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for TraitName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Graph node id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ItemId {
    /// Type node.
    Type(TypeName),
    /// Trait node.
    Trait(TraitName),
}

impl ItemId {
    pub(crate) fn type_name(&self) -> Option<&TypeName> {
        match self {
            Self::Type(name) => Some(name),
            Self::Trait(_) => None,
        }
    }

    pub(crate) fn trait_name(&self) -> Option<&TraitName> {
        match self {
            Self::Type(_) => None,
            Self::Trait(name) => Some(name),
        }
    }
}

/// Implementation edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ImplEdge {
    /// A type implements a trait.
    TypeImplsTrait {
        /// Implementing type.
        ty: TypeName,
        /// Implemented trait.
        tr: TraitName,
    },
    /// A trait implies another trait.
    TraitImplsTrait {
        /// Implementing trait.
        subject: TraitName,
        /// Implemented trait.
        target: TraitName,
    },
}

impl ImplEdge {
    /// Source node id.
    pub fn source_id(&self) -> ItemId {
        match self {
            Self::TypeImplsTrait { ty, .. } => ItemId::Type(ty.clone()),
            Self::TraitImplsTrait { subject, .. } => ItemId::Trait(subject.clone()),
        }
    }

    /// Target trait.
    pub fn target_trait(&self) -> &TraitName {
        match self {
            Self::TypeImplsTrait { tr, .. } => tr,
            Self::TraitImplsTrait { target, .. } => target,
        }
    }

    /// Whether the edge references a trait as either endpoint.
    pub fn references_trait(&self, tr: &TraitName) -> bool {
        match self {
            Self::TypeImplsTrait { tr: target, .. } => target == tr,
            Self::TraitImplsTrait { subject, target } => subject == tr || target == tr,
        }
    }
}

impl fmt::Display for ImplEdge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeImplsTrait { ty, tr } => write!(formatter, "{ty} -> {tr}"),
            Self::TraitImplsTrait { subject, target } => write!(formatter, "{subject} -> {target}"),
        }
    }
}

/// Serializable graph data.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TypeTraitGraphData {
    /// Type nodes.
    pub types: BTreeSet<TypeName>,
    /// Trait nodes.
    pub traits: BTreeSet<TraitName>,
    /// Implementation edges.
    pub impls: BTreeSet<ImplEdge>,
}

impl TypeTraitGraphData {
    /// Return the current primitive-count cost.
    pub fn cost(&self) -> GraphCost {
        GraphCost {
            types: self.types.len(),
            traits: self.traits.len(),
            impls: self.impls.len(),
        }
    }

    /// Apply a rewrite plan to the data representation.
    pub fn apply_plan(&self, plan: &RewritePlan) -> Result<Self, TypeTraitGraphError> {
        let mut next = self.clone();
        for op in &plan.ops {
            next.apply_op(op)?;
        }
        TypeTraitGraph::from_data(next.clone())?;
        Ok(next)
    }

    /// Render a compact text graph.
    pub fn render_text_graph(&self) -> String {
        let mut output = String::new();
        let cost = self.cost();
        let _ = writeln!(
            output,
            "score: {} (types={} traits={} impls={})",
            cost.total(),
            cost.types,
            cost.traits,
            cost.impls
        );
        Self::render_named_set(&mut output, "types", &self.types);
        Self::render_named_set(&mut output, "traits", &self.traits);
        let _ = writeln!(output, "impls:");
        if self.impls.is_empty() {
            let _ = writeln!(output, "  <none>");
        } else {
            for edge in &self.impls {
                let _ = writeln!(output, "  {edge}");
            }
        }
        output
    }

    fn apply_op(&mut self, op: &GraphOp) -> Result<(), TypeTraitGraphError> {
        match op {
            GraphOp::AddImpl(edge) => self.add_impl(edge.clone()),
            GraphOp::RemoveImpl(edge) => self.remove_impl(edge),
            GraphOp::RemoveTrait(tr) => self.remove_trait(tr),
        }
    }

    fn add_impl(&mut self, edge: ImplEdge) -> Result<(), TypeTraitGraphError> {
        self.expect_edge_nodes(&edge)?;
        if self.impls.insert(edge.clone()) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::DuplicateImpl {
                source: edge.source_id(),
                target: edge.target_trait().clone(),
            })
        }
    }

    fn remove_impl(&mut self, edge: &ImplEdge) -> Result<(), TypeTraitGraphError> {
        if self.impls.remove(edge) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::UnknownImpl { edge: edge.clone() })
        }
    }

    fn remove_trait(&mut self, tr: &TraitName) -> Result<(), TypeTraitGraphError> {
        if self.traits.remove(tr) {
            self.impls.retain(|edge| !edge.references_trait(tr));
            Ok(())
        } else {
            Err(TypeTraitGraphError::UnknownTrait { tr: tr.clone() })
        }
    }

    fn expect_edge_nodes(&self, edge: &ImplEdge) -> Result<(), TypeTraitGraphError> {
        match edge {
            ImplEdge::TypeImplsTrait { ty, tr } => {
                if !self.types.contains(ty) {
                    return Err(TypeTraitGraphError::UnknownType { ty: ty.clone() });
                }
                if !self.traits.contains(tr) {
                    return Err(TypeTraitGraphError::UnknownTrait { tr: tr.clone() });
                }
                Ok(())
            }
            ImplEdge::TraitImplsTrait { subject, target } => {
                if subject == target {
                    return Err(TypeTraitGraphError::SelfTraitImpl {
                        tr: subject.clone(),
                    });
                }
                if !self.traits.contains(subject) {
                    return Err(TypeTraitGraphError::UnknownTrait {
                        tr: subject.clone(),
                    });
                }
                if !self.traits.contains(target) {
                    return Err(TypeTraitGraphError::UnknownTrait { tr: target.clone() });
                }
                Ok(())
            }
        }
    }

    fn render_named_set<T: fmt::Display>(output: &mut String, label: &str, values: &BTreeSet<T>) {
        let _ = writeln!(output, "{label}:");
        if values.is_empty() {
            let _ = writeln!(output, "  <none>");
        } else {
            for value in values {
                let _ = writeln!(output, "  {value}");
            }
        }
    }
}

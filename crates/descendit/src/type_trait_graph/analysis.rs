use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{TraitName, TypeName};

/// Structural graph cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GraphCost {
    /// Type node count.
    pub types: usize,
    /// Trait node count.
    pub traits: usize,
    /// Implementation edge count.
    pub impls: usize,
}

impl GraphCost {
    /// Total primitive count.
    pub fn total(self) -> usize {
        self.types
            .saturating_add(self.traits)
            .saturating_add(self.impls)
    }
}

/// Estimated primitive-count reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CostReduction {
    /// Removed trait nodes.
    pub traits: usize,
    /// Removed implementation edges.
    pub impls: usize,
}

impl CostReduction {
    /// Total primitive-count reduction.
    pub fn total(self) -> usize {
        self.traits.saturating_add(self.impls)
    }
}

/// Traits with the same direct type implementors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralTraitGroup {
    /// Structurally equivalent trait names.
    pub traits: BTreeSet<TraitName>,
    /// Shared direct type implementors.
    pub direct_type_implementors: BTreeSet<TypeName>,
    /// Best-case reduction if one trait remains and the others are removed.
    pub best_case_reduction: CostReduction,
}

/// A trait-to-trait implementation that may be collapsible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraitAliasCandidate {
    /// Alias-like trait.
    pub alias: TraitName,
    /// Implied target trait.
    pub target: TraitName,
    /// Direct type implementors of the alias.
    pub alias_direct_type_implementors: BTreeSet<TypeName>,
    /// Direct type implementors of the target.
    pub target_direct_type_implementors: BTreeSet<TypeName>,
    /// New type-to-target impls needed if the alias is removed.
    pub replacement_type_impls: BTreeSet<TypeName>,
    /// Best-case reduction if the alias is removed.
    pub best_case_reduction: CostReduction,
}

/// Simplification analysis result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeTraitGraphAnalysis {
    /// Current graph cost.
    pub cost: GraphCost,
    /// Trait groups with identical direct type implementors.
    pub structural_trait_groups: Vec<StructuralTraitGroup>,
    /// Alias-like trait implication edges.
    pub trait_aliases: Vec<TraitAliasCandidate>,
}

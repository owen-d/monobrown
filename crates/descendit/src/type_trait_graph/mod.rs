//! Pure type/trait implementation graph model.
//!
//! This module models Rust-ish capability structure with only two node kinds
//! and one edge kind:
//!
//! ```text
//! Type  -impls-> Trait
//! Trait -impls-> Trait
//! ```
//!
//! It intentionally does not model methods, modules, functions, or source text.
//! The point is to make trait-structure duplication visible before deciding
//! whether richer Rust semantics are worth adding.

mod analysis;
mod data;
mod diagram;
mod error;
mod rewrite;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;

pub use analysis::{
    CostReduction, GraphCost, StructuralTraitGroup, TraitAliasCandidate, TypeTraitGraphAnalysis,
};
pub use data::{ImplEdge, ItemId, TraitName, TypeName, TypeTraitGraphData};
pub use diagram::TypeTraitGraphDiagram;
pub use error::TypeTraitGraphError;
pub use rewrite::{GraphOp, RewriteCandidate, RewritePlan, RewriteRule};

/// Type/trait implementation graph.
#[derive(Debug, Clone)]
pub struct TypeTraitGraph {
    nodes: BTreeSet<ItemId>,
    impls: BTreeSet<ImplEdge>,
}

impl Default for TypeTraitGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTraitGraph {
    /// Build an empty type/trait graph.
    pub fn new() -> Self {
        Self {
            nodes: BTreeSet::new(),
            impls: BTreeSet::new(),
        }
    }

    /// Build a graph from serializable graph data.
    pub fn from_data(data: TypeTraitGraphData) -> Result<Self, TypeTraitGraphError> {
        let mut graph = Self::new();
        for ty in data.types {
            graph.add_type(ty)?;
        }
        for tr in data.traits {
            graph.add_trait(tr)?;
        }
        for edge in data.impls {
            graph.add_impl_from_edge(edge)?;
        }
        Ok(graph)
    }

    /// Return serializable graph data.
    pub fn to_data(&self) -> TypeTraitGraphData {
        let mut data = TypeTraitGraphData::default();
        for id in &self.nodes {
            match id {
                ItemId::Type(ty) => {
                    data.types.insert(ty.clone());
                }
                ItemId::Trait(tr) => {
                    data.traits.insert(tr.clone());
                }
            }
        }
        data.impls = self.impl_edges().into_iter().collect();
        data
    }

    /// Add a type node.
    pub fn add_type(&mut self, ty: TypeName) -> Result<(), TypeTraitGraphError> {
        if self.nodes.insert(ItemId::Type(ty.clone())) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::DuplicateType { ty })
        }
    }

    /// Add a trait node.
    pub fn add_trait(&mut self, tr: TraitName) -> Result<(), TypeTraitGraphError> {
        if self.nodes.insert(ItemId::Trait(tr.clone())) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::DuplicateTrait { tr })
        }
    }

    /// Add a type-implements-trait edge.
    pub fn type_impls_trait(
        &mut self,
        ty: TypeName,
        tr: TraitName,
    ) -> Result<(), TypeTraitGraphError> {
        self.expect_type(&ty)?;
        self.expect_trait(&tr)?;
        self.add_impl_edge(ItemId::Type(ty), tr)
    }

    /// Add a trait-implements-trait edge.
    pub fn trait_impls_trait(
        &mut self,
        subject: TraitName,
        target: TraitName,
    ) -> Result<(), TypeTraitGraphError> {
        self.expect_trait(&subject)?;
        self.expect_trait(&target)?;
        if subject == target {
            return Err(TypeTraitGraphError::SelfTraitImpl { tr: subject });
        }
        self.add_impl_edge(ItemId::Trait(subject), target)
    }

    /// Return the current primitive-count cost.
    pub fn cost(&self) -> GraphCost {
        let mut cost = GraphCost::default();
        for id in &self.nodes {
            match id {
                ItemId::Type(_) => cost.types += 1,
                ItemId::Trait(_) => cost.traits += 1,
            }
        }
        cost.impls = self.impls.len();
        cost
    }

    /// Return implementation edges in deterministic order.
    pub fn impl_edges(&self) -> Vec<ImplEdge> {
        self.impls.iter().cloned().collect()
    }

    /// Return direct type implementors for a trait.
    pub fn direct_type_implementors(&self, tr: &TraitName) -> BTreeSet<TypeName> {
        self.impls
            .iter()
            .filter_map(|edge| match edge {
                ImplEdge::TypeImplsTrait { ty, tr: target } if target == tr => Some(ty.clone()),
                _ => None,
            })
            .collect()
    }

    /// Return type implementors whose impl paths reach the trait.
    pub fn effective_type_implementors(&self, tr: &TraitName) -> BTreeSet<TypeName> {
        let mut implementors = BTreeSet::new();
        for id in &self.nodes {
            let Some(ty) = id.type_name() else {
                continue;
            };
            if self.type_reaches_trait(id, tr) {
                implementors.insert(ty.clone());
            }
        }
        implementors
    }

    /// Analyze structural simplification opportunities.
    pub fn analyze(&self) -> TypeTraitGraphAnalysis {
        TypeTraitGraphAnalysis {
            cost: self.cost(),
            structural_trait_groups: self.structural_trait_groups(),
            trait_aliases: self.trait_alias_candidates(),
        }
    }

    /// Apply a rewrite plan and return the rewritten graph.
    pub fn apply_plan(&self, plan: &RewritePlan) -> Result<Self, TypeTraitGraphError> {
        Self::from_data(self.to_data().apply_plan(plan)?)
    }

    /// Discover score-lowering rewrite candidates.
    pub fn discover_rewrite_candidates(&self) -> Vec<RewriteCandidate> {
        let mut plans = BTreeSet::new();
        self.add_alias_bypass_plans(&mut plans);
        self.add_duplicate_trait_merge_plans(&mut plans);

        let mut candidates = plans
            .into_iter()
            .filter_map(|plan| self.rewrite_candidate(plan))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .score_reduction()
                .cmp(&left.score_reduction())
                .then_with(|| left.plan.cmp(&right.plan))
        });
        candidates
    }

    /// Render a compact text graph.
    pub fn render_text_graph(&self) -> String {
        self.to_data().render_text_graph()
    }

    /// Render a human-readable Unicode diagram of the current graph.
    pub fn render_text_diagram(&self) -> Result<String, TypeTraitGraphError> {
        let data = self.to_data();
        render_type_trait_diagram(&data)
    }

    /// Render a before/plan/after artifact for a rewrite candidate.
    pub fn render_rewrite_candidate(
        &self,
        candidate: &RewriteCandidate,
    ) -> Result<String, TypeTraitGraphError> {
        let after = self.apply_plan(&candidate.plan)?;
        let before_data = self.to_data();
        let after_data = after.to_data();
        let mut output = String::new();
        let _ = writeln!(output, "rule: {}", candidate.plan.rule);
        let _ = writeln!(
            output,
            "score: {} -> {} (-{})",
            candidate.before_cost.total(),
            candidate.after_cost.total(),
            candidate.score_reduction()
        );
        let _ = writeln!(output);
        let _ = writeln!(output, "before:");
        let _ = writeln!(output, "  diagram:");
        Self::write_nested(&mut output, &render_type_trait_diagram(&before_data)?);
        let _ = writeln!(output, "  data:");
        Self::write_nested(&mut output, &before_data.render_text_graph());
        let _ = writeln!(output, "plan:");
        Self::write_indented(&mut output, &candidate.plan.render_text());
        let _ = writeln!(output, "after:");
        let _ = writeln!(output, "  diagram:");
        Self::write_nested(&mut output, &render_type_trait_diagram(&after_data)?);
        let _ = writeln!(output, "  data:");
        Self::write_nested(&mut output, &after_data.render_text_graph());
        Ok(output)
    }

    fn add_impl_from_edge(&mut self, edge: ImplEdge) -> Result<(), TypeTraitGraphError> {
        match edge {
            ImplEdge::TypeImplsTrait { ty, tr } => self.type_impls_trait(ty, tr),
            ImplEdge::TraitImplsTrait { subject, target } => {
                self.trait_impls_trait(subject, target)
            }
        }
    }

    fn add_impl_edge(
        &mut self,
        source: ItemId,
        target: TraitName,
    ) -> Result<(), TypeTraitGraphError> {
        let edge = match &source {
            ItemId::Type(ty) => ImplEdge::TypeImplsTrait {
                ty: ty.clone(),
                tr: target.clone(),
            },
            ItemId::Trait(subject) => ImplEdge::TraitImplsTrait {
                subject: subject.clone(),
                target: target.clone(),
            },
        };
        if self.impls.insert(edge) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::DuplicateImpl { source, target })
        }
    }

    fn add_alias_bypass_plans(&self, plans: &mut BTreeSet<RewritePlan>) {
        for candidate in self.trait_alias_candidates() {
            let mut ops = candidate
                .replacement_type_impls
                .into_iter()
                .map(|ty| {
                    GraphOp::AddImpl(ImplEdge::TypeImplsTrait {
                        ty,
                        tr: candidate.target.clone(),
                    })
                })
                .collect::<Vec<_>>();
            ops.push(GraphOp::RemoveTrait(candidate.alias));
            plans.insert(RewritePlan::new(RewriteRule::AliasBypass, ops));
        }
    }

    fn add_duplicate_trait_merge_plans(&self, plans: &mut BTreeSet<RewritePlan>) {
        for group in self.structural_trait_groups() {
            if group.direct_type_implementors.is_empty() {
                continue;
            }
            let ops = group
                .traits
                .iter()
                .skip(1)
                .cloned()
                .map(GraphOp::RemoveTrait)
                .collect::<Vec<_>>();
            plans.insert(RewritePlan::new(RewriteRule::DuplicateTraitMerge, ops));
        }
    }

    fn rewrite_candidate(&self, plan: RewritePlan) -> Option<RewriteCandidate> {
        let before_cost = self.cost();
        let after_cost = self.apply_plan(&plan).ok()?.cost();
        if after_cost.total() < before_cost.total() {
            Some(RewriteCandidate {
                plan,
                before_cost,
                after_cost,
            })
        } else {
            None
        }
    }

    fn write_indented(output: &mut String, text: &str) {
        for line in text.lines() {
            let _ = writeln!(output, "  {line}");
        }
    }

    fn write_nested(output: &mut String, text: &str) {
        for line in text.lines() {
            let _ = writeln!(output, "    {line}");
        }
    }

    fn expect_type(&self, ty: &TypeName) -> Result<(), TypeTraitGraphError> {
        let id = ItemId::Type(ty.clone());
        if self.nodes.contains(&id) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::UnknownType { ty: ty.clone() })
        }
    }

    fn expect_trait(&self, tr: &TraitName) -> Result<(), TypeTraitGraphError> {
        let id = ItemId::Trait(tr.clone());
        if self.nodes.contains(&id) {
            Ok(())
        } else {
            Err(TypeTraitGraphError::UnknownTrait { tr: tr.clone() })
        }
    }

    fn type_reaches_trait(&self, source: &ItemId, tr: &TraitName) -> bool {
        let target = ItemId::Trait(tr.clone());
        let mut queue = VecDeque::new();
        let mut visited = BTreeSet::new();
        queue.push_back(source.clone());

        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            for next in self.outgoing_ids(&id) {
                if next == target {
                    return true;
                }
                queue.push_back(next);
            }
        }
        false
    }

    fn trait_names(&self) -> Vec<TraitName> {
        self.nodes
            .iter()
            .filter_map(ItemId::trait_name)
            .cloned()
            .collect()
    }

    fn outgoing_ids(&self, source: &ItemId) -> Vec<ItemId> {
        self.impls
            .iter()
            .filter_map(|edge| match (source, edge) {
                (ItemId::Type(source_ty), ImplEdge::TypeImplsTrait { ty, tr })
                    if source_ty == ty =>
                {
                    Some(ItemId::Trait(tr.clone()))
                }
                (ItemId::Trait(source_tr), ImplEdge::TraitImplsTrait { subject, target })
                    if source_tr == subject =>
                {
                    Some(ItemId::Trait(target.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn structural_trait_groups(&self) -> Vec<StructuralTraitGroup> {
        let mut groups: BTreeMap<BTreeSet<TypeName>, BTreeSet<TraitName>> = BTreeMap::new();
        for tr in self.trait_names() {
            groups
                .entry(self.direct_type_implementors(&tr))
                .or_default()
                .insert(tr);
        }

        let mut result = Vec::new();
        for (direct_type_implementors, traits) in groups {
            if traits.len() < 2 {
                continue;
            }
            let removed_traits = traits.len() - 1;
            result.push(StructuralTraitGroup {
                traits,
                best_case_reduction: CostReduction {
                    traits: removed_traits,
                    impls: removed_traits.saturating_mul(direct_type_implementors.len()),
                },
                direct_type_implementors,
            });
        }
        result
    }

    fn trait_alias_candidates(&self) -> Vec<TraitAliasCandidate> {
        let mut candidates = Vec::new();
        for edge in self.impl_edges() {
            let ImplEdge::TraitImplsTrait { subject, target } = edge else {
                continue;
            };
            let alias_direct_type_implementors = self.direct_type_implementors(&subject);
            let target_direct_type_implementors = self.direct_type_implementors(&target);
            let replacement_type_impls = alias_direct_type_implementors
                .difference(&target_direct_type_implementors)
                .cloned()
                .collect::<BTreeSet<_>>();
            let removed_impls = alias_direct_type_implementors.len().saturating_add(1);
            let added_impls = replacement_type_impls.len();

            candidates.push(TraitAliasCandidate {
                alias: subject,
                target,
                best_case_reduction: CostReduction {
                    traits: 1,
                    impls: removed_impls.saturating_sub(added_impls),
                },
                alias_direct_type_implementors,
                target_direct_type_implementors,
                replacement_type_impls,
            });
        }
        candidates
    }
}

fn render_type_trait_diagram(data: &TypeTraitGraphData) -> Result<String, TypeTraitGraphError> {
    TypeTraitGraphDiagram::new(data)
        .render_unicode()
        .map_err(|source| TypeTraitGraphError::Diagram { source })
}

#[cfg(test)]
mod tests;

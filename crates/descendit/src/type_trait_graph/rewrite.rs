use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

use super::{GraphCost, ImplEdge, TraitName};

/// Rewrite rule used to build a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RewriteRule {
    /// Bypass an alias-like trait edge.
    AliasBypass,
    /// Merge structurally duplicate traits.
    DuplicateTraitMerge,
}

impl fmt::Display for RewriteRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AliasBypass => formatter.write_str("alias_bypass"),
            Self::DuplicateTraitMerge => formatter.write_str("duplicate_trait_merge"),
        }
    }
}

/// Primitive graph rewrite operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GraphOp {
    /// Add an implementation edge.
    AddImpl(ImplEdge),
    /// Remove an implementation edge.
    RemoveImpl(ImplEdge),
    /// Remove a trait and every incident implementation edge.
    RemoveTrait(TraitName),
}

impl GraphOp {
    pub(crate) fn render_text(&self) -> String {
        match self {
            Self::AddImpl(edge) => format!("+ impl {edge}"),
            Self::RemoveImpl(edge) => format!("- impl {edge}"),
            Self::RemoveTrait(tr) => format!("- trait {tr}"),
        }
    }
}

/// Data-only graph rewrite plan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RewritePlan {
    /// Rule that produced the plan.
    pub rule: RewriteRule,
    /// Primitive graph operations.
    pub ops: Vec<GraphOp>,
}

impl RewritePlan {
    /// Build a rewrite plan.
    pub fn new(rule: RewriteRule, ops: Vec<GraphOp>) -> Self {
        Self { rule, ops }
    }

    /// Render a compact text plan.
    pub fn render_text(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "rule: {}", self.rule);
        let _ = writeln!(output, "ops:");
        if self.ops.is_empty() {
            let _ = writeln!(output, "  <none>");
        } else {
            for op in &self.ops {
                let _ = writeln!(output, "  {}", op.render_text());
            }
        }
        output
    }
}

/// A discovered score-lowering rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewriteCandidate {
    /// Rewrite plan.
    pub plan: RewritePlan,
    /// Cost before applying the plan.
    pub before_cost: GraphCost,
    /// Cost after applying the plan.
    pub after_cost: GraphCost,
}

impl RewriteCandidate {
    /// Total score reduction.
    pub fn score_reduction(&self) -> usize {
        self.before_cost
            .total()
            .saturating_sub(self.after_cost.total())
    }
}

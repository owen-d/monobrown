//! Output types matching the `SemanticData` JSON schema from `descendit`.
//!
//! These types are intentionally duplicated (not shared via dependency) to
//! avoid circular deps between `descendit-ra` and `descendit`. Both serialize
//! to identical JSON.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticData {
    pub crate_name: String,
    pub type_cardinalities: Vec<ResolvedTypeCardinality>,
    pub function_cardinalities: Vec<ResolvedFunctionCardinality>,
    pub call_edges: Vec<CallEdge>,
    #[serde(default)]
    pub type_trait_facts: Vec<TypeTraitFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedTypeCardinality {
    pub file: String,
    pub module_path: String,
    pub name: String,
    pub cardinality_log2: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFunctionCardinality {
    pub file: String,
    pub module_path: String,
    pub name: String,
    pub line: usize,
    pub internal_state_cardinality_log2: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct CallEdge {
    pub caller_module: String,
    pub caller_file: String,
    pub caller_function: String,
    pub caller_line: usize,
    pub callee_module: String,
    pub callee_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypeTraitFact {
    /// Workspace-relative source file when available.
    pub file: String,
    /// Rust module path containing the declaration or impl.
    pub module_path: String,
    /// 1-based line for source-backed reports.
    pub line: usize,
    #[serde(flatten)]
    pub kind: TypeTraitFactKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeTraitFactKind {
    /// Trait declaration.
    TraitDecl { name: String },
    /// Type-to-trait implementation.
    TypeImplTrait { ty: String, tr: String },
    /// Trait-to-trait implication.
    TraitImplTrait { subject: String, target: String },
}

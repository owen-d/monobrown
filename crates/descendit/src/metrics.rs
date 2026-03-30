//! Metric types for code analysis results.
//!
//! These types capture structural properties of Rust source code at the
//! function, type, and codebase levels. All types are serializable and
//! designed for deterministic computation: same source always produces
//! the same metrics.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::duplication::DuplicationReport;

/// A segment in an item's scope path, capturing the nesting context.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name")]
#[serde(rename_all = "snake_case")]
pub enum ScopeSegment {
    Module(String),
    Type(String),
    Function(String),
}

impl fmt::Display for ScopeSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeSegment::Module(n) => write!(f, "mod {n}"),
            ScopeSegment::Type(n) => write!(f, "type {n}"),
            ScopeSegment::Function(n) => write!(f, "fn {n}"),
        }
    }
}

/// Derive `module_path` from a scope path by extracting Module segments.
pub fn module_path_from_scope(scope: &[ScopeSegment]) -> String {
    scope
        .iter()
        .filter_map(|s| match s {
            ScopeSegment::Module(name) => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("::")
}

/// Per-function structural metrics extracted from the AST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionMetrics {
    /// Fully qualified function name.
    pub name: String,
    /// Source file path relative to the analysis root.
    pub file: String,
    /// Module path within the crate (e.g., "net::protocol").
    /// Empty string for items at the crate root.
    #[serde(default)]
    pub module_path: String,
    /// Full nesting context of this function (modules, types, and the function itself).
    #[serde(default)]
    pub scope_path: Vec<ScopeSegment>,
    /// Line number where the function is defined.
    pub line: usize,
    /// Number of lines in the function body (opening `{` to closing `}`).
    pub lines: usize,
    /// Number of parameters (excluding `self` variants for methods).
    pub params: usize,
    /// Maximum nesting depth within the function body.
    pub nesting_depth: usize,
    /// Cyclomatic complexity (branches + 1).
    pub cyclomatic: usize,
    /// Count of `let mut` bindings in the function body.
    pub mutable_bindings: usize,
    /// log2 of the product of per-binding state cardinalities for all `let mut` bindings.
    /// Inferred from initializer expressions: bool/Option/Result literals → 2, scalars → 1.
    /// Zero when the function has no mutable bindings.
    pub internal_state_cardinality_log2: f64,
    /// Count of assertion macros (`assert!`, `assert_eq!`, `debug_assert!`, etc.).
    pub assertions: usize,
    /// Count of assertions that reference at least one real identifier (not just literals).
    pub meaningful_assertions: usize,
    /// Whether this function is a test (`#[test]` attribute or inside `#[cfg(test)]` module).
    pub is_test: bool,
    /// Whether this function has `pub` visibility (standalone API surface).
    pub is_pub: bool,
}

/// Classification of a type definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeKind {
    Struct,
    Enum,
}

/// Per-type structural metrics extracted from the AST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeMetrics {
    /// Type name.
    pub name: String,
    /// Source file path relative to the analysis root.
    pub file: String,
    /// Module path within the crate (e.g., "net::protocol").
    /// Empty string for items at the crate root.
    #[serde(default)]
    pub module_path: String,
    /// Full nesting context of this type (modules and the type itself).
    #[serde(default)]
    pub scope_path: Vec<ScopeSegment>,
    /// Line number where the type is defined.
    pub line: usize,
    /// Whether this is a struct or enum.
    pub kind: TypeKind,
    /// Count of fields with type `bool`.
    pub bool_fields: usize,
    /// Count of fields with type `Option<T>`.
    pub option_fields: usize,
    /// Total field count (struct) or variant count (enum).
    pub total_fields: usize,
    /// Approximate number of representable states (structural cardinality).
    pub state_cardinality: u64,
    /// log2 of the approximate number of representable states.
    /// Derived from `state_cardinality`: `(state_cardinality as f64).log2()`, or 0.0 if <= 1.
    pub state_cardinality_log2: f64,
}

/// Aggregate summary statistics across all analyzed code.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub function_count: usize,
    pub max_function_lines: usize,
    pub mean_function_lines: f64,
    /// Count of functions exceeding the 70-line structural limit.
    pub functions_over_70_lines: usize,
    pub max_nesting_depth: usize,
    pub mean_nesting_depth: f64,
    pub max_cyclomatic: usize,
    pub mean_cyclomatic: f64,
    pub max_params: usize,
    pub total_mutable_bindings: usize,
    pub type_count: usize,
    pub total_bool_fields: usize,
    pub total_option_fields: usize,
    pub max_state_cardinality_log2: f64,
    /// Count of functions with fewer than 2 assertions (structural violation).
    pub functions_under_2_assertions: usize,
    /// Functions with lines > 5 AND cyclomatic > 1 that have < 2 assertions.
    pub nontrivial_functions_under_2_assertions: usize,
    /// Total non-trivial functions (lines > 5 AND cyclomatic > 1).
    pub nontrivial_function_count: usize,
    /// Total assertion count across all functions.
    pub total_assertions: usize,
    /// Mean assertions per function.
    pub mean_assertions_per_function: f64,
    /// Total meaningful assertions (assertions referencing real identifiers).
    pub total_meaningful_assertions: usize,
    /// Mean meaningful assertions per function.
    pub mean_meaningful_assertions_per_function: f64,
    /// Number of test functions (`#[test]` or inside `#[cfg(test)]`).
    pub test_function_count: usize,
    /// Number of production (non-test) functions.
    pub production_function_count: usize,
    /// Number of non-test functions with `pub` visibility.
    pub public_function_count: usize,
    /// Count of `macro_rules!` definitions containing control flow (if/match/while/for/loop).
    /// These are treated as equivalent to private functions for code economy.
    #[serde(default)]
    pub macro_fn_count: usize,
    /// Subset of `macro_fn_count` that have `#[macro_export]` (treated as public).
    #[serde(default)]
    pub macro_export_fn_count: usize,
    /// Ratio of non-test functions to public functions (overhead ratio).
    /// 0.0 when there are no public functions.
    pub function_overhead_ratio: f64,
    /// Test assertions / production cyclomatic complexity. Higher = better tested.
    pub test_density: f64,
    /// Sum of cyclomatic complexity for non-test functions.
    pub total_production_cyclomatic: usize,
    /// Sum of body lines for non-test functions.
    pub production_lines: usize,
    /// Number of groups of exactly structurally identical functions.
    pub exact_duplicate_groups: usize,
    /// Number of near-duplicate function pairs (Jaccard > 0.8).
    pub near_duplicate_pairs: usize,
    /// Fraction of functions that are exact or near duplicates.
    pub duplication_score: f64,
}

/// Per-file token entropy metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntropy {
    pub file: String,
    pub tokens: usize,
    pub vocabulary: usize,
    pub entropy_bits: f64,
    pub normalized_entropy: f64,
}

/// Aggregate token-level entropy metrics across all analyzed files.
///
/// Shannon entropy measures the information density of the token stream.
/// Lower normalized entropy indicates more repetitive/redundant code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntropyMetrics {
    /// Total token count across all files.
    pub total_tokens: usize,
    /// Unique token count (vocabulary size).
    pub vocabulary_size: usize,
    /// Shannon entropy in bits.
    pub entropy_bits: f64,
    /// Normalized entropy (0.0 = all same token, 1.0 = uniform distribution).
    pub normalized_entropy: f64,
    /// Per-file entropy, sorted by normalized entropy ascending (most repetitive first).
    pub per_file: Vec<FileEntropy>,
}

/// Aggregate metrics derived from semantic analysis.
/// Present only when semantic data was available at analysis time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticSummary {
    pub coupling_density: f64,
    pub coupling_module_count: usize,
    pub coupling_edge_count: usize,
}

/// Complete analysis report for a codebase or file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_root: Option<String>,
    pub files_analyzed: usize,
    pub total_lines: usize,
    pub functions: Vec<FunctionMetrics>,
    pub types: Vec<TypeMetrics>,
    pub entropy: EntropyMetrics,
    pub duplication: DuplicationReport,
    pub summary: Summary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<SemanticSummary>,
}

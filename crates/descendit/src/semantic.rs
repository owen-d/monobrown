//! Semantic data overlay from the semantic analysis backend.
//!
//! This module defines the schema for semantic data emitted by the semantic
//! analysis backend, and the processed [`SemanticOverlay`] consumed by the
//! compliance pipeline.
//!
//! # Consumption flow
//!
//! - [`SemanticOverlay::load()`] deserializes `semantic.json` into [`SemanticData`],
//!   then calls [`SemanticOverlay::from_data()`] to build lookup tables.
//! - [`SemanticOverlay::compute_coupling()`] builds per-module and per-function
//!   outgoing edge counts from raw [`CallEdge`] data, producing [`CouplingData`].
//!
//! # Fallback behavior
//!
//! Compliance functions check `if let Some(semantic) = ...` before using the
//! overlay. When present, compiler-resolved data takes precedence. When absent:
//! - Coupling density returns a vacuous 1.0 (no penalty, no reward).
//! - Type/function cardinality falls back to syn-based heuristic estimates.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Semantic data from the analysis backend, loaded from JSON.
/// This is the raw data emitted by the backend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticData {
    pub crate_name: String,
    pub type_cardinalities: Vec<ResolvedTypeCardinality>,
    pub function_cardinalities: Vec<ResolvedFunctionCardinality>,
    pub call_edges: Vec<CallEdge>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_module: String,
    pub caller_file: String,
    pub callee_module: String,
    pub callee_file: String,
    /// Function name of the caller, when available from the rust-analyzer backend.
    /// Empty string means legacy data without per-function attribution.
    #[serde(default)]
    pub caller_function: String,
    /// Line number of the caller function definition, when available.
    #[serde(default)]
    pub caller_line: usize,
}

/// Processed semantic overlay, ready for consumption by the compliance pipeline.
/// Constructed from SemanticData.
///
/// Note: The backend-resolved enum cardinality uses max-variant cardinality
/// (only intra-variant boolean soup is penalized), while the syn-based
/// computation may differ. When the overlay is present, the backend values
/// take precedence.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticOverlay {
    /// (file, module_path, type_name) -> resolved cardinality_log2
    pub type_cardinalities: BTreeMap<(String, String, String), f64>,
    /// (file, module_path, fn_name, line) -> resolved internal_state_cardinality_log2
    pub function_cardinalities: BTreeMap<(String, String, String, usize), f64>,
    /// Coupling density computed from call edges.
    pub coupling: CouplingData,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CouplingData {
    /// The raw density value: |E| / (N * (N-1)), range [0, 1].
    pub density: f64,
    /// Number of modules with at least one function.
    pub module_count: usize,
    /// Number of distinct directed cross-module edges.
    pub edge_count: usize,
    /// Per-module outgoing edge counts (module_path -> count), for heatmap attribution.
    pub module_outgoing_edges: BTreeMap<String, usize>,
    /// All modules that participate in cross-module call edges (union of callers and callees).
    #[serde(default)]
    pub all_modules: BTreeSet<String>,
    /// First `caller_file` seen for each caller module, for file attribution.
    #[serde(default)]
    pub module_files: BTreeMap<String, String>,
    /// Per-function outgoing cross-module edge counts.
    /// Key: (module, function_name, line). Value: count of distinct callee modules.
    #[serde(default)]
    pub function_outgoing_edges: BTreeMap<(String, String, usize), usize>,
    /// File path for each function key.
    #[serde(default)]
    pub function_files: BTreeMap<(String, String, usize), String>,
}

impl crate::metrics::SemanticSummary {
    pub fn from_overlay(overlay: &SemanticOverlay) -> Self {
        crate::metrics::SemanticSummary {
            coupling_density: overlay.coupling.density,
            coupling_module_count: overlay.coupling.module_count,
            coupling_edge_count: overlay.coupling.edge_count,
        }
    }
}

/// Check whether two file paths refer to the same file via suffix matching.
///
/// Returns true when one path is a suffix of the other with a `/` boundary
/// (or they are equal). This handles the mismatch between syn-relative paths
/// like `agents/mod.rs` and workspace-relative paths like
/// `crates/myapp/src/agents/mod.rs`.
pub fn paths_suffix_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (longer, shorter) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    longer.ends_with(shorter) && longer.as_bytes()[longer.len() - shorter.len() - 1] == b'/'
}

impl SemanticOverlay {
    /// Build a SemanticOverlay from raw SemanticData.
    pub fn from_data(data: &SemanticData) -> Self {
        let mut type_cardinalities = BTreeMap::new();
        for tc in &data.type_cardinalities {
            type_cardinalities.insert(
                (tc.file.clone(), tc.module_path.clone(), tc.name.clone()),
                tc.cardinality_log2,
            );
        }

        let mut function_cardinalities = BTreeMap::new();
        for fc in &data.function_cardinalities {
            function_cardinalities.insert(
                (
                    fc.file.clone(),
                    fc.module_path.clone(),
                    fc.name.clone(),
                    fc.line,
                ),
                fc.internal_state_cardinality_log2,
            );
        }

        // Compute coupling from call edges.
        let coupling = Self::compute_coupling(&data.call_edges);

        SemanticOverlay {
            type_cardinalities,
            function_cardinalities,
            coupling,
        }
    }

    fn compute_coupling(edges: &[CallEdge]) -> CouplingData {
        // Collect all modules that participate as caller or callee.
        let mut modules: BTreeSet<String> = BTreeSet::new();
        let mut directed_edges: BTreeSet<(String, String)> = BTreeSet::new();
        let mut module_outgoing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut module_files: BTreeMap<String, String> = BTreeMap::new();

        for edge in edges {
            if edge.caller_module == edge.callee_module {
                continue; // Skip intra-module calls.
            }
            modules.insert(edge.caller_module.clone());
            modules.insert(edge.callee_module.clone());
            directed_edges.insert((edge.caller_module.clone(), edge.callee_module.clone()));
            module_outgoing
                .entry(edge.caller_module.clone())
                .or_default()
                .insert(edge.callee_module.clone());
            module_files
                .entry(edge.caller_module.clone())
                .or_insert_with(|| edge.caller_file.clone());
        }

        let n = modules.len();
        let e = directed_edges.len();
        let density = if n <= 1 {
            0.0
        } else {
            e as f64 / (n * (n - 1)) as f64
        };

        let module_outgoing_edges = module_outgoing
            .into_iter()
            .map(|(m, targets)| (m, targets.len()))
            .collect();

        // Per-function pass: group by (caller_module, caller_function, caller_line)
        // and count distinct callee modules per group. Only process edges where
        // `caller_function` is non-empty (populated by the rust-analyzer backend).
        let mut fn_outgoing: BTreeMap<(String, String, usize), BTreeSet<String>> = BTreeMap::new();
        let mut function_files: BTreeMap<(String, String, usize), String> = BTreeMap::new();

        for edge in edges {
            if edge.caller_module == edge.callee_module {
                continue;
            }
            if edge.caller_function.is_empty() {
                continue;
            }
            let key = (
                edge.caller_module.clone(),
                edge.caller_function.clone(),
                edge.caller_line,
            );
            fn_outgoing
                .entry(key.clone())
                .or_default()
                .insert(edge.callee_module.clone());
            function_files
                .entry(key)
                .or_insert_with(|| edge.caller_file.clone());
        }

        let function_outgoing_edges = fn_outgoing
            .into_iter()
            .map(|(k, targets)| (k, targets.len()))
            .collect();

        CouplingData {
            density,
            module_count: n,
            edge_count: e,
            module_outgoing_edges,
            all_modules: modules,
            module_files,
            function_outgoing_edges,
            function_files,
        }
    }

    /// Look up resolved type cardinality.
    ///
    /// Uses suffix matching on file path and relaxed module_path matching to
    /// handle mismatches between syn (relative paths, empty module_path) and
    /// the semantic analysis backend (workspace-relative paths, resolved module_path).
    pub fn type_cardinality(&self, file: &str, module_path: &str, name: &str) -> Option<f64> {
        // Fast path: exact match.
        let key = (file.to_string(), module_path.to_string(), name.to_string());
        if let Some(&v) = self.type_cardinalities.get(&key) {
            return Some(v);
        }
        // Slow path: suffix match on file, ignore module_path mismatch (syn
        // often sets module_path="" for file-level items that the backend resolves).
        for ((stored_file, _stored_mod, stored_name), &v) in &self.type_cardinalities {
            if stored_name == name && paths_suffix_match(stored_file, file) {
                return Some(v);
            }
        }
        None
    }

    /// Look up resolved function internal state cardinality.
    ///
    /// Uses suffix matching on the file path (see `type_cardinality`).
    pub fn function_cardinality(
        &self,
        file: &str,
        module_path: &str,
        name: &str,
        line: usize,
    ) -> Option<f64> {
        // Fast path: exact match.
        let key = (
            file.to_string(),
            module_path.to_string(),
            name.to_string(),
            line,
        );
        if let Some(&v) = self.function_cardinalities.get(&key) {
            return Some(v);
        }
        // Slow path: suffix match on file, ignore module_path mismatch.
        for ((stored_file, _stored_mod, stored_name, stored_line), &v) in
            &self.function_cardinalities
        {
            if stored_name == name && *stored_line == line && paths_suffix_match(stored_file, file)
            {
                return Some(v);
            }
        }
        None
    }

    /// Load SemanticData from a JSON file and build the overlay.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            format!(
                "Failed to read semantic data from {}: {}",
                path.display(),
                e
            )
        })?;
        let data: SemanticData = serde_json::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse semantic data from {}: {}",
                path.display(),
                e
            )
        })?;
        Ok(Self::from_data(&data))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_from_data_empty() {
        let data = SemanticData::default();
        let overlay = SemanticOverlay::from_data(&data);

        assert!(overlay.type_cardinalities.is_empty());
        assert!(overlay.function_cardinalities.is_empty());
        assert!((overlay.coupling.density - 0.0).abs() < f64::EPSILON);
        assert_eq!(overlay.coupling.module_count, 0);
        assert_eq!(overlay.coupling.edge_count, 0);
    }

    #[test]
    fn test_coupling_density_with_edges() {
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: vec![
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "b".into(),
                    callee_file: "b.rs".into(),
                    caller_function: "do_stuff".into(),
                    caller_line: 10,
                },
                CallEdge {
                    caller_module: "b".into(),
                    caller_file: "b.rs".into(),
                    callee_module: "c".into(),
                    callee_file: "c.rs".into(),
                    caller_function: "handle".into(),
                    caller_line: 20,
                },
                // Intra-module call should be skipped.
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "a".into(),
                    callee_file: "a.rs".into(),
                    caller_function: "internal".into(),
                    caller_line: 30,
                },
            ],
        };
        let overlay = SemanticOverlay::from_data(&data);

        // 3 modules, 2 directed edges. density = 2 / (3 * 2) = 1/3.
        assert_eq!(overlay.coupling.module_count, 3);
        assert_eq!(overlay.coupling.edge_count, 2);
        assert!((overlay.coupling.density - 1.0 / 3.0).abs() < 1e-10);
        assert_eq!(overlay.coupling.module_outgoing_edges.len(), 2);
        assert_eq!(overlay.coupling.module_outgoing_edges["a"], 1);
        assert_eq!(overlay.coupling.module_outgoing_edges["b"], 1);
    }

    #[test]
    fn test_full_coupling_density() {
        // 2 modules, both edges present => density = 2 / (2 * 1) = 1.0.
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: vec![
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "b".into(),
                    callee_file: "b.rs".into(),
                    caller_function: "call_b".into(),
                    caller_line: 5,
                },
                CallEdge {
                    caller_module: "b".into(),
                    caller_file: "b.rs".into(),
                    callee_module: "a".into(),
                    callee_file: "a.rs".into(),
                    caller_function: "call_a".into(),
                    caller_line: 15,
                },
            ],
        };
        let overlay = SemanticOverlay::from_data(&data);

        assert!((overlay.coupling.density - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_type_and_function_cardinality_lookups() {
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: vec![ResolvedTypeCardinality {
                file: "lib.rs".into(),
                module_path: "net".into(),
                name: "Connection".into(),
                cardinality_log2: 4.0,
            }],
            function_cardinalities: vec![ResolvedFunctionCardinality {
                file: "lib.rs".into(),
                module_path: "net".into(),
                name: "connect".into(),
                line: 42,
                internal_state_cardinality_log2: 3.0,
            }],
            call_edges: Vec::new(),
        };
        let overlay = SemanticOverlay::from_data(&data);

        assert_eq!(
            overlay.type_cardinality("lib.rs", "net", "Connection"),
            Some(4.0)
        );
        assert_eq!(overlay.type_cardinality("lib.rs", "net", "Missing"), None);
        assert_eq!(
            overlay.function_cardinality("lib.rs", "net", "connect", 42),
            Some(3.0)
        );
        assert_eq!(
            overlay.function_cardinality("lib.rs", "net", "connect", 99),
            None
        );
    }

    #[test]
    fn test_paths_suffix_match() {
        // Equal paths.
        assert!(paths_suffix_match("agents/mod.rs", "agents/mod.rs"));
        // Short query, long stored (syn -> backend).
        assert!(paths_suffix_match(
            "crates/myapp/src/agents/mod.rs",
            "agents/mod.rs"
        ));
        // Long query, short stored (backend -> syn).
        assert!(paths_suffix_match(
            "agents/mod.rs",
            "crates/myapp/src/agents/mod.rs"
        ));
        // Must not match without a `/` boundary.
        assert!(!paths_suffix_match("barfoo/mod.rs", "foo/mod.rs"));
        // Bare filename should not accidentally match a different prefix.
        assert!(!paths_suffix_match("other_mod.rs", "mod.rs"));
        // Bare filename exact match.
        assert!(paths_suffix_match("mod.rs", "mod.rs"));
        // Completely different paths.
        assert!(!paths_suffix_match("a.rs", "b.rs"));
    }

    #[test]
    fn test_type_cardinality_suffix_match() {
        // Overlay built with workspace-relative path (backend).
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: vec![ResolvedTypeCardinality {
                file: "crates/myapp/src/agents/mod.rs".into(),
                module_path: "agents".into(),
                name: "AgentState".into(),
                cardinality_log2: 5.0,
            }],
            function_cardinalities: Vec::new(),
            call_edges: Vec::new(),
        };
        let overlay = SemanticOverlay::from_data(&data);

        // Lookup with short syn-relative path should match.
        assert_eq!(
            overlay.type_cardinality("agents/mod.rs", "agents", "AgentState"),
            Some(5.0)
        );
        // Exact match still works.
        assert_eq!(
            overlay.type_cardinality("crates/myapp/src/agents/mod.rs", "agents", "AgentState"),
            Some(5.0)
        );
        // Wrong name returns None (module_path is ignored in fallback).
        assert_eq!(
            overlay.type_cardinality("agents/mod.rs", "agents", "Missing"),
            None
        );
    }

    #[test]
    fn test_type_cardinality_suffix_match_reverse() {
        // Overlay built with short syn-relative path, query with long path.
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: vec![ResolvedTypeCardinality {
                file: "agents/mod.rs".into(),
                module_path: "agents".into(),
                name: "AgentState".into(),
                cardinality_log2: 5.0,
            }],
            function_cardinalities: Vec::new(),
            call_edges: Vec::new(),
        };
        let overlay = SemanticOverlay::from_data(&data);

        assert_eq!(
            overlay.type_cardinality("crates/myapp/src/agents/mod.rs", "agents", "AgentState"),
            Some(5.0)
        );
    }

    #[test]
    fn test_function_cardinality_suffix_match() {
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: Vec::new(),
            function_cardinalities: vec![ResolvedFunctionCardinality {
                file: "crates/myapp/src/net/client.rs".into(),
                module_path: "net".into(),
                name: "connect".into(),
                line: 10,
                internal_state_cardinality_log2: 3.0,
            }],
            call_edges: Vec::new(),
        };
        let overlay = SemanticOverlay::from_data(&data);

        // Short path lookup.
        assert_eq!(
            overlay.function_cardinality("net/client.rs", "net", "connect", 10),
            Some(3.0)
        );
        // Exact match.
        assert_eq!(
            overlay.function_cardinality("crates/myapp/src/net/client.rs", "net", "connect", 10),
            Some(3.0)
        );
        // Wrong line still returns None.
        assert_eq!(
            overlay.function_cardinality("net/client.rs", "net", "connect", 99),
            None
        );
    }

    #[test]
    fn test_roundtrip_serialization() {
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: vec![ResolvedTypeCardinality {
                file: "lib.rs".into(),
                module_path: "".into(),
                name: "Foo".into(),
                cardinality_log2: 2.0,
            }],
            function_cardinalities: Vec::new(),
            call_edges: vec![CallEdge {
                caller_module: "a".into(),
                caller_file: "a.rs".into(),
                callee_module: "b".into(),
                callee_file: "b.rs".into(),
                caller_function: "init".into(),
                caller_line: 1,
            }],
        };

        let json = serde_json::to_string(&data).expect("serialize");
        let parsed: SemanticData = serde_json::from_str(&json).expect("deserialize");
        let overlay = SemanticOverlay::from_data(&parsed);

        assert_eq!(overlay.type_cardinality("lib.rs", "", "Foo"), Some(2.0));
        assert_eq!(overlay.coupling.edge_count, 1);
    }

    #[test]
    fn test_function_outgoing_edges_populated() {
        // Two functions in module "a" calling different modules.
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: vec![
                // fn_one in module a calls b and c.
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "b".into(),
                    callee_file: "b.rs".into(),
                    caller_function: "fn_one".into(),
                    caller_line: 10,
                },
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "c".into(),
                    callee_file: "c.rs".into(),
                    caller_function: "fn_one".into(),
                    caller_line: 10,
                },
                // fn_two in module a calls only b.
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "b".into(),
                    callee_file: "b.rs".into(),
                    caller_function: "fn_two".into(),
                    caller_line: 30,
                },
            ],
        };
        let overlay = SemanticOverlay::from_data(&data);

        // fn_one calls 2 distinct modules (b, c).
        assert_eq!(
            overlay.coupling.function_outgoing_edges[&("a".into(), "fn_one".into(), 10)],
            2
        );
        // fn_two calls 1 distinct module (b).
        assert_eq!(
            overlay.coupling.function_outgoing_edges[&("a".into(), "fn_two".into(), 30)],
            1
        );
        // File attribution is populated.
        assert_eq!(
            overlay.coupling.function_files[&("a".into(), "fn_one".into(), 10)],
            "a.rs"
        );
    }

    #[test]
    fn test_call_edge_backward_compat_deserialization() {
        // Legacy JSON without caller_function and caller_line should deserialize
        // with defaults (empty string and 0).
        let json = r#"{
            "caller_module": "a",
            "caller_file": "a.rs",
            "callee_module": "b",
            "callee_file": "b.rs"
        }"#;
        let edge: CallEdge = serde_json::from_str(json).expect("deserialize");
        assert_eq!(edge.caller_function, "");
        assert_eq!(edge.caller_line, 0);
    }
}

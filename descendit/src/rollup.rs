//! Hierarchical heatmap rollup tree.
//!
//! Builds a tree from a flat `Vec<HeatmapEntry>` by grouping entries along
//! their `scope_path` segments. Interior nodes aggregate responsibility;
//! leaves carry the original entries.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::compliance::HeatmapEntry;
use crate::metrics::ScopeSegment;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A node in the hierarchical heatmap rollup tree.
///
/// Interior nodes have non-empty `children`; leaf nodes have non-empty
/// `entries`. Each node's `responsibility` equals the sum of its children's
/// responsibilities (or the sum of its entries' responsibilities for leaves).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapTreeNode {
    pub segment: ScopeSegment,
    pub children: Vec<HeatmapTreeNode>,
    pub responsibility: f64,
    /// Per-dimension responsibility breakdown.
    pub dimension_responsibilities: BTreeMap<String, f64>,
    /// Original heatmap entries (non-empty only at leaf nodes).
    pub entries: Vec<HeatmapEntry>,
}

// ---------------------------------------------------------------------------
// Intermediate trie (private)
// ---------------------------------------------------------------------------

struct TrieNode {
    segment: ScopeSegment,
    children: HashMap<ScopeSegment, TrieNode>,
    entries: Vec<HeatmapEntry>,
}

impl TrieNode {
    fn new(segment: ScopeSegment) -> Self {
        Self {
            segment,
            children: HashMap::new(),
            entries: Vec::new(),
        }
    }

    /// Insert an entry at the appropriate depth in the trie.
    fn insert(&mut self, path: &[ScopeSegment], entry: &HeatmapEntry) {
        if path.is_empty() {
            self.entries.push(entry.clone());
            return;
        }
        let child = self
            .children
            .entry(path[0].clone())
            .or_insert_with(|| TrieNode::new(path[0].clone()));
        child.insert(&path[1..], entry);
    }

    /// Convert to the final `HeatmapTreeNode`, aggregating bottom-up.
    fn into_tree_node(self) -> HeatmapTreeNode {
        let mut children: Vec<HeatmapTreeNode> = self
            .children
            .into_values()
            .map(TrieNode::into_tree_node)
            .collect();

        // Sort: responsibility descending, then segment name ascending for ties.
        children.sort_by(|a, b| {
            b.responsibility
                .partial_cmp(&a.responsibility)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| segment_name(&a.segment).cmp(segment_name(&b.segment)))
        });

        // Aggregate responsibility from children and local entries.
        let mut responsibility: f64 = children.iter().map(|c| c.responsibility).sum();
        let mut dimension_responsibilities: BTreeMap<String, f64> = BTreeMap::new();

        for child in &children {
            for (dim, &val) in &child.dimension_responsibilities {
                *dimension_responsibilities.entry(dim.clone()).or_default() += val;
            }
        }

        for entry in &self.entries {
            responsibility += entry.responsibility;
            *dimension_responsibilities
                .entry(entry.dimension.clone())
                .or_default() += entry.responsibility;
        }

        HeatmapTreeNode {
            segment: self.segment,
            children,
            responsibility,
            dimension_responsibilities,
            entries: self.entries,
        }
    }
}

fn segment_name(seg: &ScopeSegment) -> &str {
    match seg {
        ScopeSegment::Module(n) | ScopeSegment::Type(n) | ScopeSegment::Function(n) => n.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a rollup tree from flat heatmap entries using their `scope_path`.
///
/// Entries with empty `scope_path` become root-level leaf nodes with a
/// synthetic `Function` segment derived from the entry's `function_name`.
/// Children at each level are sorted by responsibility (descending),
/// then by segment name for ties.
pub fn build_heatmap_tree(entries: &[HeatmapEntry]) -> Vec<HeatmapTreeNode> {
    // Collect into a root-level map keyed by the first segment.
    let mut roots: HashMap<ScopeSegment, TrieNode> = HashMap::new();

    for entry in entries {
        let path = if entry.scope_path.is_empty() {
            // Synthesize a single-segment path so the entry appears as a
            // root-level leaf node.
            vec![ScopeSegment::Function(entry.function_name.clone())]
        } else {
            entry.scope_path.clone()
        };

        let root = roots
            .entry(path[0].clone())
            .or_insert_with(|| TrieNode::new(path[0].clone()));
        root.insert(&path[1..], entry);
    }

    let mut nodes: Vec<HeatmapTreeNode> =
        roots.into_values().map(TrieNode::into_tree_node).collect();

    nodes.sort_by(|a, b| {
        b.responsibility
            .partial_cmp(&a.responsibility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| segment_name(&a.segment).cmp(segment_name(&b.segment)))
    });

    nodes
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        function_name: &str,
        dimension: &str,
        responsibility: f64,
        scope_path: Vec<ScopeSegment>,
    ) -> HeatmapEntry {
        HeatmapEntry {
            file: "a.rs".into(),
            line: 1,
            function_name: function_name.into(),
            dimension: dimension.into(),
            responsibility,
            detail: String::new(),
            scope_path,
        }
    }

    #[test]
    fn test_empty_input() {
        let tree = build_heatmap_tree(&[]);
        assert!(tree.is_empty());
    }

    #[test]
    fn test_single_entry() {
        let entries = vec![entry(
            "foo",
            "bloat",
            0.5,
            vec![
                ScopeSegment::Module("a".into()),
                ScopeSegment::Function("foo".into()),
            ],
        )];
        let tree = build_heatmap_tree(&entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].segment, ScopeSegment::Module("a".into()));
        assert!((tree[0].responsibility - 0.5).abs() < 1e-10);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(
            tree[0].children[0].segment,
            ScopeSegment::Function("foo".into())
        );
        assert_eq!(tree[0].children[0].entries.len(), 1);
    }

    #[test]
    fn test_multi_dimension_same_function() {
        let entries = vec![
            entry(
                "foo",
                "bloat",
                0.3,
                vec![ScopeSegment::Function("foo".into())],
            ),
            entry(
                "foo",
                "state_cardinality",
                0.2,
                vec![ScopeSegment::Function("foo".into())],
            ),
        ];
        let tree = build_heatmap_tree(&entries);
        assert_eq!(tree.len(), 1);
        assert!((tree[0].responsibility - 0.5).abs() < 1e-10);
        assert!((tree[0].dimension_responsibilities["bloat"] - 0.3).abs() < 1e-10);
        assert!((tree[0].dimension_responsibilities["state_cardinality"] - 0.2).abs() < 1e-10);
    }

    #[test]
    fn test_nested_structure() {
        let entries = vec![
            entry(
                "method",
                "bloat",
                0.4,
                vec![
                    ScopeSegment::Module("a".into()),
                    ScopeSegment::Type("Foo".into()),
                    ScopeSegment::Function("method".into()),
                ],
            ),
            entry(
                "free_fn",
                "bloat",
                0.1,
                vec![
                    ScopeSegment::Module("a".into()),
                    ScopeSegment::Function("free_fn".into()),
                ],
            ),
        ];
        let tree = build_heatmap_tree(&entries);
        // Single root: Module("a") with responsibility 0.5
        assert_eq!(tree.len(), 1);
        assert!((tree[0].responsibility - 0.5).abs() < 1e-10);
        // Two children: Type("Foo") and Function("free_fn")
        assert_eq!(tree[0].children.len(), 2);
        // Sorted by responsibility desc: Foo(0.4) before free_fn(0.1)
        assert_eq!(
            tree[0].children[0].segment,
            ScopeSegment::Type("Foo".into())
        );
        assert_eq!(
            tree[0].children[1].segment,
            ScopeSegment::Function("free_fn".into())
        );
    }

    #[test]
    fn test_responsibility_sum_invariant() {
        let entries = vec![
            entry(
                "f1",
                "bloat",
                0.3,
                vec![
                    ScopeSegment::Module("m".into()),
                    ScopeSegment::Function("f1".into()),
                ],
            ),
            entry(
                "f2",
                "bloat",
                0.7,
                vec![
                    ScopeSegment::Module("m".into()),
                    ScopeSegment::Function("f2".into()),
                ],
            ),
        ];
        let tree = build_heatmap_tree(&entries);
        let root = &tree[0];
        let child_sum: f64 = root.children.iter().map(|c| c.responsibility).sum();
        assert!((root.responsibility - child_sum).abs() < 1e-10);
    }

    #[test]
    fn test_empty_scope_path_becomes_root_leaf() {
        let entries = vec![HeatmapEntry {
            file: "<codebase>".into(),
            line: 0,
            function_name: "coupling".into(),
            dimension: "coupling_density".into(),
            responsibility: 0.1,
            detail: String::new(),
            scope_path: Vec::new(),
        }];
        let tree = build_heatmap_tree(&entries);
        assert_eq!(tree.len(), 1);
        assert!((tree[0].responsibility - 0.1).abs() < 1e-10);
        // The synthetic segment should use the function_name.
        assert_eq!(tree[0].segment, ScopeSegment::Function("coupling".into()));
    }

    #[test]
    fn test_sort_tiebreak_by_name() {
        // Two root nodes with equal responsibility should sort alphabetically.
        let entries = vec![
            entry(
                "beta",
                "bloat",
                0.5,
                vec![ScopeSegment::Module("beta".into())],
            ),
            entry(
                "alpha",
                "bloat",
                0.5,
                vec![ScopeSegment::Module("alpha".into())],
            ),
        ];
        let tree = build_heatmap_tree(&entries);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].segment, ScopeSegment::Module("alpha".into()));
        assert_eq!(tree[1].segment, ScopeSegment::Module("beta".into()));
    }

    #[test]
    fn test_dimension_rollup_through_interior() {
        // Two different dimensions under the same module should aggregate.
        let entries = vec![
            entry(
                "f1",
                "bloat",
                0.3,
                vec![
                    ScopeSegment::Module("m".into()),
                    ScopeSegment::Function("f1".into()),
                ],
            ),
            entry(
                "f2",
                "state_cardinality",
                0.2,
                vec![
                    ScopeSegment::Module("m".into()),
                    ScopeSegment::Function("f2".into()),
                ],
            ),
        ];
        let tree = build_heatmap_tree(&entries);
        let root = &tree[0];
        assert!((root.dimension_responsibilities["bloat"] - 0.3).abs() < 1e-10);
        assert!((root.dimension_responsibilities["state_cardinality"] - 0.2).abs() < 1e-10);
    }
}

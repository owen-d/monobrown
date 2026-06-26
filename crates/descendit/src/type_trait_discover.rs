//! Scoped type/trait rewrite discovery reports.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Context;
use regex_lite::Regex;
use serde::{Deserialize, Serialize};

use crate::semantic::{SemanticData, TypeTraitFact, TypeTraitFactKind};
use crate::type_trait_graph::{
    GraphOp, ImplEdge, RewriteCandidate, RewritePlan, TraitName, TypeName, TypeTraitGraph,
    TypeTraitGraphData,
};

/// Query for type/trait simplification discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeTraitQuery {
    /// Crate or workspace path used when semantic data must be generated.
    pub path: PathBuf,
    /// Optional saved semantic JSON path.
    #[serde(default)]
    pub semantic_path: Option<PathBuf>,
    /// Scope expression selecting local nodes.
    #[serde(default)]
    pub scope: Option<ScopeExpr>,
    /// Include one-hop neighbors around local nodes.
    #[serde(default = "default_include_one_hop")]
    pub include_one_hop: bool,
    /// Trait names hidden from the graph before discovery.
    #[serde(default = "default_ignored_traits")]
    pub ignored_traits: Vec<String>,
    /// Output format.
    #[serde(default)]
    pub emit: TypeTraitEmit,
}

/// Output format requested by the query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeTraitEmit {
    /// Compact text report.
    #[default]
    Text,
    /// JSON report.
    Json,
}

/// Composable scope predicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScopeExpr {
    /// Match semantic crate name.
    Crate {
        #[serde(rename = "crate", alias = "crate_name")]
        crate_name: String,
    },
    /// Match source path prefix.
    PathPrefix { path_prefix: String },
    /// Match module path prefix.
    ModulePrefix { module_prefix: String },
    /// Match node name regex.
    Name { name: String },
    /// Any child scope may match.
    Any { any: Vec<ScopeExpr> },
    /// Every child scope must match.
    All { all: Vec<ScopeExpr> },
    /// Negate a child scope.
    Not { not: Box<ScopeExpr> },
}

/// Discovery report.
#[derive(Debug, Clone, Serialize)]
pub struct TypeTraitDiscoverReport {
    /// Scoped graph used for discovery.
    pub graph: TypeTraitGraphData,
    /// Ranked candidate reports.
    pub candidates: Vec<TypeTraitCandidateReport>,
}

impl TypeTraitDiscoverReport {
    /// Render a compact text report.
    pub fn render_text(&self) -> String {
        let mut output = String::new();
        output.push_str("type-trait discovery\n");
        output.push_str("====================\n\n");
        output.push_str(&self.graph.render_text_graph());
        output.push('\n');
        if self.candidates.is_empty() {
            output.push_str("candidates:\n  <none>\n");
            return output;
        }
        output.push_str("candidates:\n");
        for (index, candidate) in self.candidates.iter().enumerate() {
            output.push_str(&format!(
                "\n{}. {} score {} -> {} (-{})\n",
                index + 1,
                candidate.rule,
                candidate.before_cost.total(),
                candidate.after_cost.total(),
                candidate.score_reduction
            ));
            output.push_str(&indent(&candidate.text, 2));
            output.push_str("  sources:\n");
            for source in &candidate.sources {
                output.push_str(&format!(
                    "    {} {}:{} {}\n",
                    source.kind, source.file, source.line, source.name
                ));
            }
        }
        output
    }
}

/// Single candidate report.
#[derive(Debug, Clone, Serialize)]
pub struct TypeTraitCandidateReport {
    /// Rule name.
    pub rule: String,
    /// Score reduction.
    pub score_reduction: usize,
    /// Cost before applying the plan.
    pub before_cost: crate::type_trait_graph::GraphCost,
    /// Cost after applying the plan.
    pub after_cost: crate::type_trait_graph::GraphCost,
    /// Rewrite plan.
    pub plan: RewritePlan,
    /// Human-readable before/plan/after text.
    pub text: String,
    /// Source references touched by the plan.
    pub sources: Vec<TypeTraitSourceRef>,
}

/// Source reference for a graph node or edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct TypeTraitSourceRef {
    /// Source kind.
    pub kind: SourceRefKind,
    /// Graph node or edge label.
    pub name: String,
    /// Source file.
    pub file: String,
    /// Module path.
    pub module_path: String,
    /// 1-based source line.
    pub line: usize,
}

/// Source reference kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRefKind {
    /// Type node source.
    Type,
    /// Trait node source.
    Trait,
    /// Impl edge source.
    Impl,
}

impl std::fmt::Display for SourceRefKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type => formatter.write_str("type"),
            Self::Trait => formatter.write_str("trait"),
            Self::Impl => formatter.write_str("impl"),
        }
    }
}

/// Build a scoped graph and candidate report from semantic data.
pub fn discover_type_trait_rewrites(
    data: &SemanticData,
    query: &TypeTraitQuery,
) -> anyhow::Result<TypeTraitDiscoverReport> {
    let facts = FactIndex::new(data, &query.ignored_traits);
    let scoped = facts.scoped_graph(query)?;
    let graph = TypeTraitGraph::from_data(scoped.graph.clone())?;
    let candidates = graph
        .discover_rewrite_candidates()
        .into_iter()
        .filter(|candidate| scoped.plan_touches_local_trait(&candidate.plan))
        .map(|candidate| build_candidate_report(&graph, &facts, candidate))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(TypeTraitDiscoverReport {
        graph: scoped.graph,
        candidates,
    })
}

fn build_candidate_report(
    graph: &TypeTraitGraph,
    facts: &FactIndex,
    candidate: RewriteCandidate,
) -> anyhow::Result<TypeTraitCandidateReport> {
    let mut sources = BTreeSet::new();
    for op in &candidate.plan.ops {
        facts.collect_sources_for_op(op, &mut sources);
    }
    let text = graph
        .render_rewrite_candidate(&candidate)
        .context("failed to render rewrite candidate")?;
    Ok(TypeTraitCandidateReport {
        rule: candidate.plan.rule.to_string(),
        score_reduction: candidate.score_reduction(),
        before_cost: candidate.before_cost,
        after_cost: candidate.after_cost,
        plan: candidate.plan,
        text,
        sources: sources.into_iter().collect(),
    })
}

struct ScopedGraph {
    graph: TypeTraitGraphData,
    local_traits: BTreeSet<TraitName>,
}

impl ScopedGraph {
    fn plan_touches_local_trait(&self, plan: &RewritePlan) -> bool {
        plan.ops.iter().any(|op| match op {
            GraphOp::AddImpl(edge) | GraphOp::RemoveImpl(edge) => {
                self.edge_references_local_trait(edge)
            }
            GraphOp::RemoveTrait(tr) => self.local_traits.contains(tr),
        })
    }

    fn edge_references_local_trait(&self, edge: &ImplEdge) -> bool {
        match edge {
            ImplEdge::TypeImplsTrait { tr, .. } => self.local_traits.contains(tr),
            ImplEdge::TraitImplsTrait { subject, target } => {
                self.local_traits.contains(subject) || self.local_traits.contains(target)
            }
        }
    }
}

struct FactIndex {
    crate_name: String,
    nodes: BTreeMap<NodeKey, TypeTraitSourceRef>,
    edges: BTreeMap<ImplEdge, TypeTraitSourceRef>,
}

impl FactIndex {
    fn new(data: &SemanticData, ignored_traits: &[String]) -> Self {
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        for fact in &data.type_trait_facts {
            if ignores_fact(fact, ignored_traits) {
                continue;
            }
            Self::insert_fact(fact, &mut nodes, &mut edges);
        }
        Self {
            crate_name: data.crate_name.clone(),
            nodes,
            edges,
        }
    }

    fn scoped_graph(&self, query: &TypeTraitQuery) -> anyhow::Result<ScopedGraph> {
        let local_nodes = self.local_nodes(query)?;
        let mut graph = TypeTraitGraphData::default();
        for node in &local_nodes {
            Self::insert_node(&mut graph, node);
        }
        for edge in self.edges.keys() {
            let source = NodeKey::from_edge_source(edge);
            let target = NodeKey::Trait(edge.target_trait().clone());
            let include = if query.include_one_hop {
                local_nodes.contains(&source) || local_nodes.contains(&target)
            } else {
                local_nodes.contains(&source) && local_nodes.contains(&target)
            };
            if include {
                Self::insert_node(&mut graph, &source);
                Self::insert_node(&mut graph, &target);
                graph.impls.insert(edge.clone());
            }
        }
        let local_traits = local_nodes
            .into_iter()
            .filter_map(|node| match node {
                NodeKey::Type(_) => None,
                NodeKey::Trait(tr) => Some(tr),
            })
            .collect();
        Ok(ScopedGraph {
            graph,
            local_traits,
        })
    }

    fn local_nodes(&self, query: &TypeTraitQuery) -> anyhow::Result<BTreeSet<NodeKey>> {
        let mut result = BTreeSet::new();
        for (node, source) in &self.nodes {
            if self.matches_scope(source, query.scope.as_ref())? {
                result.insert(node.clone());
            }
        }
        Ok(result)
    }

    fn matches_scope(
        &self,
        source: &TypeTraitSourceRef,
        scope: Option<&ScopeExpr>,
    ) -> anyhow::Result<bool> {
        match scope {
            Some(scope) => self.matches_scope_expr(source, scope),
            None => Ok(true),
        }
    }

    fn matches_scope_expr(
        &self,
        source: &TypeTraitSourceRef,
        scope: &ScopeExpr,
    ) -> anyhow::Result<bool> {
        match scope {
            ScopeExpr::Crate { crate_name } => Ok(&self.crate_name == crate_name),
            ScopeExpr::PathPrefix { path_prefix } => Ok(source.file.starts_with(path_prefix)),
            ScopeExpr::ModulePrefix { module_prefix } => {
                Ok(source.module_path.starts_with(module_prefix))
            }
            ScopeExpr::Name { name } => Ok(Regex::new(name)?.is_match(&source.name)),
            ScopeExpr::Any { any } => self.matches_any(source, any),
            ScopeExpr::All { all } => self.matches_all(source, all),
            ScopeExpr::Not { not } => self.matches_scope_expr(source, not).map(|value| !value),
        }
    }

    fn matches_any(
        &self,
        source: &TypeTraitSourceRef,
        scopes: &[ScopeExpr],
    ) -> anyhow::Result<bool> {
        for scope in scopes {
            if self.matches_scope_expr(source, scope)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn matches_all(
        &self,
        source: &TypeTraitSourceRef,
        scopes: &[ScopeExpr],
    ) -> anyhow::Result<bool> {
        for scope in scopes {
            if !self.matches_scope_expr(source, scope)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn collect_sources_for_op(&self, op: &GraphOp, sources: &mut BTreeSet<TypeTraitSourceRef>) {
        match op {
            GraphOp::AddImpl(edge) | GraphOp::RemoveImpl(edge) => {
                self.collect_sources_for_edge(edge, sources);
            }
            GraphOp::RemoveTrait(tr) => {
                self.collect_sources_for_node(&NodeKey::Trait(tr.clone()), sources);
                for edge in self.edges.keys() {
                    if edge.references_trait(tr) {
                        self.collect_sources_for_edge(edge, sources);
                    }
                }
            }
        }
    }

    fn collect_sources_for_edge(
        &self,
        edge: &ImplEdge,
        sources: &mut BTreeSet<TypeTraitSourceRef>,
    ) {
        if let Some(source) = self.edges.get(edge) {
            sources.insert(source.clone());
        }
        self.collect_sources_for_node(&NodeKey::from_edge_source(edge), sources);
        self.collect_sources_for_node(&NodeKey::Trait(edge.target_trait().clone()), sources);
    }

    fn collect_sources_for_node(&self, node: &NodeKey, sources: &mut BTreeSet<TypeTraitSourceRef>) {
        if let Some(source) = self.nodes.get(node) {
            sources.insert(source.clone());
        }
    }

    fn insert_fact(
        fact: &TypeTraitFact,
        nodes: &mut BTreeMap<NodeKey, TypeTraitSourceRef>,
        edges: &mut BTreeMap<ImplEdge, TypeTraitSourceRef>,
    ) {
        match &fact.kind {
            TypeTraitFactKind::TraitDecl { name } => {
                let tr = TraitName::new(name.clone());
                nodes
                    .entry(NodeKey::Trait(tr))
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Trait, name));
            }
            TypeTraitFactKind::TypeImplTrait { ty, tr } => {
                let ty_name = TypeName::new(ty.clone());
                let tr_name = TraitName::new(tr.clone());
                nodes
                    .entry(NodeKey::Type(ty_name.clone()))
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Type, ty));
                nodes
                    .entry(NodeKey::Trait(tr_name.clone()))
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Trait, tr));
                edges
                    .entry(ImplEdge::TypeImplsTrait {
                        ty: ty_name,
                        tr: tr_name,
                    })
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Impl, tr));
            }
            TypeTraitFactKind::TraitImplTrait { subject, target } => {
                let subject_name = TraitName::new(subject.clone());
                let target_name = TraitName::new(target.clone());
                nodes
                    .entry(NodeKey::Trait(subject_name.clone()))
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Trait, subject));
                nodes
                    .entry(NodeKey::Trait(target_name.clone()))
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Trait, target));
                edges
                    .entry(ImplEdge::TraitImplsTrait {
                        subject: subject_name,
                        target: target_name,
                    })
                    .or_insert_with(|| source_ref(fact, SourceRefKind::Impl, target));
            }
        }
    }

    fn insert_node(graph: &mut TypeTraitGraphData, node: &NodeKey) {
        match node {
            NodeKey::Type(ty) => {
                graph.types.insert(ty.clone());
            }
            NodeKey::Trait(tr) => {
                graph.traits.insert(tr.clone());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKey {
    Type(TypeName),
    Trait(TraitName),
}

impl NodeKey {
    fn from_edge_source(edge: &ImplEdge) -> Self {
        match edge {
            ImplEdge::TypeImplsTrait { ty, .. } => Self::Type(ty.clone()),
            ImplEdge::TraitImplsTrait { subject, .. } => Self::Trait(subject.clone()),
        }
    }
}

fn source_ref(fact: &TypeTraitFact, kind: SourceRefKind, name: &str) -> TypeTraitSourceRef {
    TypeTraitSourceRef {
        kind,
        name: name.to_owned(),
        file: fact.file.clone(),
        module_path: fact.module_path.clone(),
        line: fact.line,
    }
}

fn indent(text: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    let mut output = String::new();
    for line in text.lines() {
        output.push_str(&prefix);
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn default_include_one_hop() -> bool {
    true
}

fn default_ignored_traits() -> Vec<String> {
    [
        "AsMut",
        "AsRef",
        "Clone",
        "Copy",
        "Debug",
        "Default",
        "Deref",
        "DerefMut",
        "Deserialize",
        "Display",
        "Drop",
        "Error",
        "From",
        "Into",
        "JsonSchema",
        "Send",
        "Serialize",
        "Sized",
        "Sync",
        "TryFrom",
        "TryInto",
        "Unpin",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn ignores_fact(fact: &TypeTraitFact, ignored_traits: &[String]) -> bool {
    match &fact.kind {
        TypeTraitFactKind::TraitDecl { name } => ignores_trait(name, ignored_traits),
        TypeTraitFactKind::TypeImplTrait { tr, .. } => ignores_trait(tr, ignored_traits),
        TypeTraitFactKind::TraitImplTrait { subject, target } => {
            ignores_trait(subject, ignored_traits) || ignores_trait(target, ignored_traits)
        }
    }
}

fn ignores_trait(name: &str, ignored_traits: &[String]) -> bool {
    let leaf = name.rsplit("::").next().unwrap_or(name);
    ignored_traits.iter().any(|ignored| {
        name == ignored || leaf == ignored || name.ends_with(&format!("::{ignored}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn semantic_data() -> SemanticData {
        SemanticData {
            crate_name: "mmui-storage".to_owned(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: Vec::new(),
            type_trait_facts: vec![
                trait_decl(
                    "crates/mmui-storage/src/families/a.rs",
                    "mmui_storage::families::a",
                    "mmui_storage::families::a::FamilyReadA",
                ),
                trait_decl(
                    "crates/mmui-storage/src/families/b.rs",
                    "mmui_storage::families::b",
                    "mmui_storage::families::b::FamilyReadB",
                ),
                trait_decl(
                    "crates/mmui-storage/src/substrate/log.rs",
                    "mmui_storage::substrate::log",
                    "mmui_storage::substrate::log::SlateDbLogRead",
                ),
                type_impl(
                    "crates/mmui-storage/src/families/a.rs",
                    "mmui_storage::families::a",
                    "mmui_storage::families::a::ReaderA",
                    "mmui_storage::families::a::FamilyReadA",
                ),
                type_impl(
                    "crates/mmui-storage/src/families/b.rs",
                    "mmui_storage::families::b",
                    "mmui_storage::families::b::ReaderB",
                    "mmui_storage::families::b::FamilyReadB",
                ),
                trait_impl(
                    "crates/mmui-storage/src/families/a.rs",
                    "mmui_storage::families::a",
                    "mmui_storage::families::a::FamilyReadA",
                    "mmui_storage::substrate::log::SlateDbLogRead",
                ),
                trait_impl(
                    "crates/mmui-storage/src/families/b.rs",
                    "mmui_storage::families::b",
                    "mmui_storage::families::b::FamilyReadB",
                    "mmui_storage::substrate::log::SlateDbLogRead",
                ),
            ],
        }
    }

    fn trait_decl(file: &str, module_path: &str, name: &str) -> TypeTraitFact {
        TypeTraitFact {
            file: file.to_owned(),
            module_path: module_path.to_owned(),
            line: 1,
            kind: TypeTraitFactKind::TraitDecl {
                name: name.to_owned(),
            },
        }
    }

    fn type_impl(file: &str, module_path: &str, ty: &str, tr: &str) -> TypeTraitFact {
        TypeTraitFact {
            file: file.to_owned(),
            module_path: module_path.to_owned(),
            line: 10,
            kind: TypeTraitFactKind::TypeImplTrait {
                ty: ty.to_owned(),
                tr: tr.to_owned(),
            },
        }
    }

    fn trait_impl(file: &str, module_path: &str, subject: &str, target: &str) -> TypeTraitFact {
        TypeTraitFact {
            file: file.to_owned(),
            module_path: module_path.to_owned(),
            line: 5,
            kind: TypeTraitFactKind::TraitImplTrait {
                subject: subject.to_owned(),
                target: target.to_owned(),
            },
        }
    }

    #[test]
    fn scoped_query_keeps_one_hop_external_context() -> anyhow::Result<()> {
        let query = TypeTraitQuery {
            path: PathBuf::from("crates/mmui-storage"),
            semantic_path: None,
            include_one_hop: true,
            ignored_traits: default_ignored_traits(),
            emit: TypeTraitEmit::Json,
            scope: Some(ScopeExpr::PathPrefix {
                path_prefix: "crates/mmui-storage/src/families/a".to_owned(),
            }),
        };

        let report = discover_type_trait_rewrites(&semantic_data(), &query)?;

        assert_eq!(report.graph.traits.len(), 2);
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].rule, "alias_bypass");
        assert!(report.graph.traits.contains(&TraitName::from(
            "mmui_storage::substrate::log::SlateDbLogRead"
        )));
        Ok(())
    }

    #[test]
    fn scopes_can_be_anded_and_ored() -> anyhow::Result<()> {
        let query = TypeTraitQuery {
            path: PathBuf::from("crates/mmui-storage"),
            semantic_path: None,
            include_one_hop: true,
            ignored_traits: default_ignored_traits(),
            emit: TypeTraitEmit::Json,
            scope: Some(ScopeExpr::All {
                all: vec![
                    ScopeExpr::Crate {
                        crate_name: "mmui-storage".to_owned(),
                    },
                    ScopeExpr::Any {
                        any: vec![
                            ScopeExpr::PathPrefix {
                                path_prefix: "crates/mmui-storage/src/families/a".to_owned(),
                            },
                            ScopeExpr::PathPrefix {
                                path_prefix: "crates/mmui-storage/src/substrate/log".to_owned(),
                            },
                        ],
                    },
                ],
            }),
        };

        let report = discover_type_trait_rewrites(&semantic_data(), &query)?;

        assert_eq!(report.candidates.len(), 1);
        assert!(report.render_text().contains("alias_bypass"));
        Ok(())
    }

    #[test]
    fn query_deserializes_from_llm_friendly_json() -> anyhow::Result<()> {
        let json = r#"{
          "path": "crates/mmui-storage",
          "emit": "json",
          "scope": {
            "all": [
              { "crate": "mmui-storage" },
              { "name": "FamilyReadA|SlateDbLogRead" }
            ]
          }
        }"#;

        let query = serde_json::from_str::<TypeTraitQuery>(json)?;
        let report = discover_type_trait_rewrites(&semantic_data(), &query)?;

        assert_eq!(query.emit, TypeTraitEmit::Json);
        assert!(query.ignored_traits.contains(&"Display".to_owned()));
        assert_eq!(report.candidates.len(), 1);
        Ok(())
    }

    #[test]
    fn default_ignored_traits_hide_standard_trait_noise() -> anyhow::Result<()> {
        let data = SemanticData {
            crate_name: "sample".to_owned(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: Vec::new(),
            type_trait_facts: vec![
                trait_decl("src/lib.rs", "sample", "fmt::Display"),
                trait_decl("src/lib.rs", "sample", "sample::DomainRead"),
                type_impl(
                    "src/lib.rs",
                    "sample",
                    "sample::DomainError",
                    "fmt::Display",
                ),
                type_impl(
                    "src/lib.rs",
                    "sample",
                    "sample::Reader",
                    "sample::DomainRead",
                ),
            ],
        };
        let query = serde_json::from_str::<TypeTraitQuery>(
            r#"{"path":"crates/sample","scope":{"path_prefix":"src"}}"#,
        )?;

        let report = discover_type_trait_rewrites(&data, &query)?;

        assert!(
            !report
                .graph
                .traits
                .contains(&TraitName::from("fmt::Display"))
        );
        assert!(
            !report
                .graph
                .types
                .contains(&TypeName::from("sample::DomainError"))
        );
        assert!(
            report
                .graph
                .traits
                .contains(&TraitName::from("sample::DomainRead"))
        );
        assert!(
            report
                .graph
                .types
                .contains(&TypeName::from("sample::Reader"))
        );
        Ok(())
    }

    #[test]
    fn ignored_traits_can_be_cleared() -> anyhow::Result<()> {
        let data = SemanticData {
            crate_name: "sample".to_owned(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: Vec::new(),
            type_trait_facts: vec![
                trait_decl("src/lib.rs", "sample", "fmt::Display"),
                type_impl(
                    "src/lib.rs",
                    "sample",
                    "sample::DomainError",
                    "fmt::Display",
                ),
            ],
        };
        let query = serde_json::from_str::<TypeTraitQuery>(
            r#"{"path":"crates/sample","ignored_traits":[],"scope":{"path_prefix":"src"}}"#,
        )?;

        let report = discover_type_trait_rewrites(&data, &query)?;

        assert!(
            report
                .graph
                .traits
                .contains(&TraitName::from("fmt::Display"))
        );
        assert!(
            report
                .graph
                .types
                .contains(&TypeName::from("sample::DomainError"))
        );
        Ok(())
    }
}

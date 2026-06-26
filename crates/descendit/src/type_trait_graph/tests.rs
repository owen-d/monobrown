use std::collections::BTreeSet;

use super::*;

fn add_type(graph: &mut TypeTraitGraph, name: &str) -> Result<(), TypeTraitGraphError> {
    graph.add_type(TypeName::from(name))
}

fn add_trait(graph: &mut TypeTraitGraph, name: &str) -> Result<(), TypeTraitGraphError> {
    graph.add_trait(TraitName::from(name))
}

fn type_impls_trait(
    graph: &mut TypeTraitGraph,
    ty: &str,
    tr: &str,
) -> Result<(), TypeTraitGraphError> {
    graph.type_impls_trait(TypeName::from(ty), TraitName::from(tr))
}

fn trait_impls_trait(
    graph: &mut TypeTraitGraph,
    subject: &str,
    target: &str,
) -> Result<(), TypeTraitGraphError> {
    graph.trait_impls_trait(TraitName::from(subject), TraitName::from(target))
}

#[test]
fn finds_traits_with_the_same_direct_implementors() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReadWriter")?;
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    add_trait(&mut graph, "QueryExecutionLogReadBackend")?;
    type_impls_trait(&mut graph, "SlateDbLogReadWriter", "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "SlateDbLogRead")?;
    type_impls_trait(
        &mut graph,
        "SlateDbLogReadWriter",
        "QueryExecutionLogReadBackend",
    )?;
    type_impls_trait(
        &mut graph,
        "SlateDbLogReader",
        "QueryExecutionLogReadBackend",
    )?;

    let analysis = graph.analyze();

    assert_eq!(analysis.cost.total(), 8);
    assert_eq!(analysis.structural_trait_groups.len(), 1);
    let group = &analysis.structural_trait_groups[0];
    assert_eq!(
        group.traits,
        BTreeSet::from([
            TraitName::from("QueryExecutionLogReadBackend"),
            TraitName::from("SlateDbLogRead"),
        ])
    );
    assert_eq!(
        group.direct_type_implementors,
        BTreeSet::from([
            TypeName::from("SlateDbLogReader"),
            TypeName::from("SlateDbLogReadWriter"),
        ])
    );
    assert_eq!(
        group.best_case_reduction,
        CostReduction {
            traits: 1,
            impls: 2
        }
    );
    Ok(())
}

#[test]
fn keeps_same_traits_with_different_implementors() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReadWriter")?;
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    add_trait(&mut graph, "WriterOnlyRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReadWriter", "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReadWriter", "WriterOnlyRead")?;

    let analysis = graph.analyze();

    assert!(analysis.structural_trait_groups.is_empty());
    Ok(())
}

#[test]
fn finds_trait_impl_edges_as_alias_candidates() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    add_trait(&mut graph, "FamilyLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "FamilyLogRead")?;
    trait_impls_trait(&mut graph, "FamilyLogRead", "SlateDbLogRead")?;

    let analysis = graph.analyze();

    assert_eq!(analysis.trait_aliases.len(), 1);
    assert_eq!(
        analysis.trait_aliases[0].alias,
        TraitName::from("FamilyLogRead")
    );
    assert_eq!(
        analysis.trait_aliases[0].target,
        TraitName::from("SlateDbLogRead")
    );
    assert_eq!(
        analysis.trait_aliases[0].replacement_type_impls,
        BTreeSet::from([TypeName::from("SlateDbLogReader")])
    );
    assert_eq!(
        analysis.trait_aliases[0].best_case_reduction,
        CostReduction {
            traits: 1,
            impls: 1
        }
    );
    Ok(())
}

#[test]
fn alias_candidate_reuses_existing_target_impls() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    add_trait(&mut graph, "FamilyLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "FamilyLogRead")?;
    trait_impls_trait(&mut graph, "FamilyLogRead", "SlateDbLogRead")?;

    let analysis = graph.analyze();

    assert_eq!(
        analysis.trait_aliases[0].replacement_type_impls,
        BTreeSet::new()
    );
    assert_eq!(
        analysis.trait_aliases[0].best_case_reduction,
        CostReduction {
            traits: 1,
            impls: 2
        }
    );
    Ok(())
}

#[test]
fn effective_implementors_follow_trait_impl_edges() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "Concrete")?;
    add_trait(&mut graph, "SpecificRead")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "Concrete", "SpecificRead")?;
    trait_impls_trait(&mut graph, "SpecificRead", "SlateDbLogRead")?;

    assert_eq!(
        graph.effective_type_implementors(&TraitName::from("SlateDbLogRead")),
        BTreeSet::from([TypeName::from("Concrete")])
    );
    Ok(())
}

#[test]
fn graph_data_roundtrips_through_json() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "FamilyLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "FamilyLogRead")?;

    let json = serde_json::to_string(&graph.to_data())?;
    let data = serde_json::from_str::<TypeTraitGraphData>(&json)?;
    let roundtrip = TypeTraitGraph::from_data(data)?;

    assert_eq!(roundtrip.to_data(), graph.to_data());
    Ok(())
}

#[test]
fn discovers_and_applies_alias_bypass_plan() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "FamilyLogRead")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "FamilyLogRead")?;
    trait_impls_trait(&mut graph, "FamilyLogRead", "SlateDbLogRead")?;

    let candidates = graph.discover_rewrite_candidates();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].plan.rule, RewriteRule::AliasBypass);
    assert_eq!(candidates[0].before_cost.total(), 5);
    assert_eq!(candidates[0].after_cost.total(), 3);
    let rewritten = graph.apply_plan(&candidates[0].plan)?;
    assert_eq!(
        rewritten.to_data().impls,
        BTreeSet::from([ImplEdge::TypeImplsTrait {
            ty: TypeName::from("SlateDbLogReader"),
            tr: TraitName::from("SlateDbLogRead"),
        }])
    );
    Ok(())
}

#[test]
fn discovers_and_applies_duplicate_trait_merge_plan() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "Concrete")?;
    add_trait(&mut graph, "TraitA")?;
    add_trait(&mut graph, "TraitB")?;
    type_impls_trait(&mut graph, "Concrete", "TraitA")?;
    type_impls_trait(&mut graph, "Concrete", "TraitB")?;

    let candidates = graph.discover_rewrite_candidates();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].plan.rule, RewriteRule::DuplicateTraitMerge);
    assert_eq!(candidates[0].score_reduction(), 2);
    let rewritten = graph.apply_plan(&candidates[0].plan)?;
    assert_eq!(
        rewritten.to_data().traits,
        BTreeSet::from([TraitName::from("TraitA")])
    );
    assert_eq!(rewritten.cost().total(), 3);
    Ok(())
}

#[test]
fn renders_rewrite_candidate_as_text_artifact() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "FamilyLogRead")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "FamilyLogRead")?;
    trait_impls_trait(&mut graph, "FamilyLogRead", "SlateDbLogRead")?;

    let candidates = graph.discover_rewrite_candidates();
    let rendered = graph.render_rewrite_candidate(&candidates[0])?;

    assert!(rendered.contains("rule: alias_bypass"));
    assert!(rendered.contains("score: 5 -> 3 (-2)"));
    assert!(rendered.contains("before:"));
    assert!(rendered.contains("plan:"));
    assert!(rendered.contains("after:"));
    assert!(rendered.contains("+ impl SlateDbLogReader -> SlateDbLogRead"));
    Ok(())
}

#[test]
fn skips_empty_trait_duplicates_as_low_value_noise() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_trait(&mut graph, "UnusedReadA")?;
    add_trait(&mut graph, "UnusedReadB")?;

    let analysis = graph.analyze();
    let candidates = graph.discover_rewrite_candidates();

    assert_eq!(analysis.structural_trait_groups.len(), 1);
    assert!(candidates.is_empty());
    Ok(())
}

#[test]
fn reports_tiny_one_off_alias_as_local_candidate() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "SlateDbLogReader")?;
    add_trait(&mut graph, "FamilyRead")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "SlateDbLogReader", "FamilyRead")?;
    trait_impls_trait(&mut graph, "FamilyRead", "SlateDbLogRead")?;

    let candidates = graph.discover_rewrite_candidates();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].plan.rule, RewriteRule::AliasBypass);
    assert_eq!(candidates[0].score_reduction(), 2);
    Ok(())
}

#[test]
fn reports_repeated_family_wrappers_to_same_shared_trait() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "QueryExecutionReader")?;
    add_type(&mut graph, "SessionReader")?;
    add_type(&mut graph, "AuditReader")?;
    add_trait(&mut graph, "QueryExecutionRead")?;
    add_trait(&mut graph, "SessionRead")?;
    add_trait(&mut graph, "AuditRead")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "QueryExecutionReader", "QueryExecutionRead")?;
    type_impls_trait(&mut graph, "SessionReader", "SessionRead")?;
    type_impls_trait(&mut graph, "AuditReader", "AuditRead")?;
    trait_impls_trait(&mut graph, "QueryExecutionRead", "SlateDbLogRead")?;
    trait_impls_trait(&mut graph, "SessionRead", "SlateDbLogRead")?;
    trait_impls_trait(&mut graph, "AuditRead", "SlateDbLogRead")?;

    let candidates = graph.discover_rewrite_candidates();

    assert_eq!(candidates.len(), 3);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.plan.rule == RewriteRule::AliasBypass)
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.score_reduction() == 2)
    );
    Ok(())
}

#[test]
fn does_not_flag_trait_hub_as_simplification() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "ReaderA")?;
    add_type(&mut graph, "ReaderB")?;
    add_type(&mut graph, "ReaderC")?;
    add_trait(&mut graph, "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "ReaderA", "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "ReaderB", "SlateDbLogRead")?;
    type_impls_trait(&mut graph, "ReaderC", "SlateDbLogRead")?;

    assert!(graph.discover_rewrite_candidates().is_empty());
    Ok(())
}

#[test]
fn does_not_flag_near_duplicate_traits() -> Result<(), TypeTraitGraphError> {
    let mut graph = TypeTraitGraph::new();
    add_type(&mut graph, "ReaderA")?;
    add_type(&mut graph, "ReaderB")?;
    add_trait(&mut graph, "ReadA")?;
    add_trait(&mut graph, "ReadB")?;
    type_impls_trait(&mut graph, "ReaderA", "ReadA")?;
    type_impls_trait(&mut graph, "ReaderB", "ReadA")?;
    type_impls_trait(&mut graph, "ReaderA", "ReadB")?;

    assert!(graph.analyze().structural_trait_groups.is_empty());
    assert!(graph.discover_rewrite_candidates().is_empty());
    Ok(())
}

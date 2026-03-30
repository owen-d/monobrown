#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod support;

use std::fs;

use descendit::{
    ArtifactAggregation, ArtifactSizeWeighting, CompliancePolicy, ComplianceReport,
    ObjectiveScalarization, run_corpus_experiment,
};
use support::benchmark_cases::{BenchmarkCase, benchmark_case};

fn compliance_for(policy: &CompliancePolicy, source: &str) -> ComplianceReport {
    let dir = tempfile::tempdir().expect("create scenario tempdir");
    fs::write(dir.path().join("lib.rs"), source).expect("write scenario source");
    let report = descendit::analyze_path(dir.path()).expect("analyze scenario source");
    descendit::compute_compliance(&report, policy)
}

fn dimension_score(report: &ComplianceReport, name: &str) -> f64 {
    report
        .soft_dimensions
        .iter()
        .find(|dimension| dimension.name == name)
        .unwrap_or_else(|| panic!("missing dimension {name}"))
        .score
}

fn branchy_function(name: &str, filler_lines: usize) -> String {
    let mut source = String::new();
    source.push_str(&format!("pub fn {name}(x: i32) -> i32 {{\n"));
    source.push_str("    let mut total = 0;\n");
    source.push_str("    if x > 0 {\n");
    for line in 0..filler_lines {
        source.push_str(&format!("        total += {};\n", line + 1));
    }
    source.push_str("    } else {\n");
    source.push_str("        total -= 1;\n");
    source.push_str("    }\n");
    source.push_str("    total\n");
    source.push_str("}\n\n");
    source
}

fn corpus_result_for_case(
    case: &BenchmarkCase,
    policy: &CompliancePolicy,
) -> (
    descendit::CorpusExperimentResult,
    descendit::CorpusExperimentResult,
) {
    let dir = tempfile::tempdir().expect("create corpus scenario tempdir");

    fs::write(dir.path().join("before.rs"), case.before).expect("write before source");
    fs::write(dir.path().join("after.rs"), case.after).expect("write after source");

    let before_analysis =
        descendit::analyze_path(&dir.path().join("before.rs")).expect("analyze before");
    let after_analysis =
        descendit::analyze_path(&dir.path().join("after.rs")).expect("analyze after");

    let run = run_corpus_experiment(
        &[
            descendit::CorpusExperimentTarget {
                label: "before".into(),
                analysis: before_analysis,
                semantic: None,
            },
            descendit::CorpusExperimentTarget {
                label: "after".into(),
                analysis: after_analysis,
                semantic: None,
            },
        ],
        policy,
    );

    let before = run
        .results
        .iter()
        .find(|result| result.label == "before")
        .expect("before result")
        .clone();
    let after = run
        .results
        .iter()
        .find(|result| result.label == "after")
        .expect("after result")
        .clone();

    (before, after)
}

#[test]
fn test_split_and_merge_do_not_trivially_win_under_sensitive_policy() {
    let mut policy = CompliancePolicy::default();
    policy.directional.code_economy_log2_overhead = 1.5_f64.log2();
    policy.directional.bloat_log2_lines = 3.0_f64.log2();

    let split = benchmark_case("split_god_function");
    let split_before = compliance_for(&policy, split.before);
    let split_after = compliance_for(&policy, split.after);

    assert!(
        dimension_score(&split_after, "bloat") > dimension_score(&split_before, "bloat"),
        "splitting should improve bloat under the sensitive policy",
    );
    assert!(
        dimension_score(&split_after, "code_economy")
            < dimension_score(&split_before, "code_economy"),
        "splitting should worsen code economy under the sensitive policy",
    );

    let merge = benchmark_case("merge_helpers_into_one_function");
    let merge_before = compliance_for(&policy, merge.before);
    let merge_after = compliance_for(&policy, merge.after);

    assert!(
        dimension_score(&merge_after, "bloat") < dimension_score(&merge_before, "bloat"),
        "merging should worsen bloat under the sensitive policy",
    );
    assert!(
        dimension_score(&merge_after, "code_economy")
            > dimension_score(&merge_before, "code_economy"),
        "merging should improve code economy under the sensitive policy",
    );
}

#[test]
fn test_one_extreme_outlier_hurts_more_than_many_mild_degradations() {
    let mut outlier_src = String::new();
    for index in 0..9 {
        outlier_src.push_str(&branchy_function(&format!("good_{index}"), 6));
    }
    outlier_src.push_str(&branchy_function("extreme", 96));

    let mut diffuse_src = String::new();
    for index in 0..10 {
        diffuse_src.push_str(&branchy_function(&format!("mild_{index}"), 12));
    }

    let mut policy = CompliancePolicy::default();
    policy.aggregation.bloat_aggregation = ArtifactAggregation::MeanPlusCvarLoss {
        alpha: 0.1,
        tail_weight: 0.5,
    };
    policy.aggregation.objective_scalarization = ObjectiveScalarization::ArithmeticMeanScore;

    let outlier = compliance_for(&policy, &outlier_src);
    let diffuse = compliance_for(&policy, &diffuse_src);

    assert!(
        dimension_score(&outlier, "bloat") < dimension_score(&diffuse, "bloat"),
        "one extreme outlier should hurt bloat more than many mild degradations",
    );
}

#[test]
fn test_obvious_improvement_and_regression_keep_sign_under_robust_normalization() {
    let improve_case = benchmark_case("replace_bool_soup_with_enum");
    let regress_case = benchmark_case("add_duplication");

    let default_policy = CompliancePolicy::default();
    let mut robust_policy = CompliancePolicy::default();
    robust_policy.normalization.overrides.insert(
        descendit::NormalizationCohort::StateCardinalityType,
        descendit::CohortNormalizationStrategy::CohortMedianIqr {
            iqr_multiplier: 1.0,
            min_count: 2,
        },
    );
    robust_policy.normalization.overrides.insert(
        descendit::NormalizationCohort::StateCardinalityFunction,
        descendit::CohortNormalizationStrategy::CohortMedianIqr {
            iqr_multiplier: 1.0,
            min_count: 2,
        },
    );
    robust_policy.normalization.overrides.insert(
        descendit::NormalizationCohort::BloatFunction,
        descendit::CohortNormalizationStrategy::CohortUpperQuartile {
            multiplier: 1.0,
            min_count: 2,
        },
    );

    let (default_before_improve, default_after_improve) =
        corpus_result_for_case(improve_case, &default_policy);
    let (robust_before_improve, robust_after_improve) =
        corpus_result_for_case(improve_case, &robust_policy);
    assert!(
        default_after_improve.composite_score >= default_before_improve.composite_score,
        "obvious improvement should not reverse under default normalization",
    );
    assert!(
        robust_after_improve.composite_score >= robust_before_improve.composite_score,
        "obvious improvement should not reverse under robust normalization",
    );

    let (default_before_regress, default_after_regress) =
        corpus_result_for_case(regress_case, &default_policy);
    let (robust_before_regress, robust_after_regress) =
        corpus_result_for_case(regress_case, &robust_policy);
    assert!(
        default_after_regress.composite_score <= default_before_regress.composite_score,
        "obvious regression should not reverse under default normalization",
    );
    assert!(
        robust_after_regress.composite_score <= robust_before_regress.composite_score,
        "obvious regression should not reverse under robust normalization",
    );
}

#[test]
fn test_directional_mode_preserves_tradeoffs_and_signs() {
    let policy = CompliancePolicy::default();

    let split = benchmark_case("split_god_function");
    let split_before = compliance_for(&policy, split.before);
    let split_after = compliance_for(&policy, split.after);
    assert!(
        dimension_score(&split_after, "bloat") > dimension_score(&split_before, "bloat"),
        "directional mode should still reward splitting a god function on bloat",
    );
    assert!(
        dimension_score(&split_after, "code_economy")
            < dimension_score(&split_before, "code_economy"),
        "directional mode should still penalize the extra overhead of splitting",
    );

    let improve_case = benchmark_case("replace_bool_soup_with_enum");
    let improve_before = compliance_for(&policy, improve_case.before);
    let improve_after = compliance_for(&policy, improve_case.after);
    assert!(
        improve_after.composite_score >= improve_before.composite_score,
        "directional mode should keep the obvious enum improvement positive",
    );

    let regress_case = benchmark_case("add_duplication");
    let regress_before = compliance_for(&policy, regress_case.before);
    let regress_after = compliance_for(&policy, regress_case.after);
    assert!(
        regress_after.composite_score <= regress_before.composite_score,
        "directional mode should keep the obvious duplication regression negative",
    );
}

#[test]
fn test_hierarchical_size_weighting_dampens_small_helper_dilution() {
    let before = branchy_function("bad", 96);

    let mut after = before.clone();
    for index in 0..10 {
        after.push_str(&branchy_function(&format!("tiny_{index}"), 6));
    }

    let plain_policy = CompliancePolicy::default();
    let mut hierarchical_policy = CompliancePolicy::default();
    hierarchical_policy.aggregation.bloat_aggregation =
        ArtifactAggregation::HierarchicalFileWeightedMeanLoss {
            size_weighting: ArtifactSizeWeighting::LinearWeight,
        };

    let plain_before = compliance_for(&plain_policy, &before);
    let plain_after = compliance_for(&plain_policy, &after);
    let hierarchical_before = compliance_for(&hierarchical_policy, &before);
    let hierarchical_after = compliance_for(&hierarchical_policy, &after);

    let plain_improvement =
        dimension_score(&plain_after, "bloat") - dimension_score(&plain_before, "bloat");
    let hierarchical_improvement = dimension_score(&hierarchical_after, "bloat")
        - dimension_score(&hierarchical_before, "bloat");

    assert!(
        plain_improvement > 0.0,
        "adding tiny helpers should improve plain bloat"
    );
    assert!(
        hierarchical_improvement > 0.0,
        "hierarchical weighting should still recognize some improvement",
    );
    assert!(
        hierarchical_improvement < plain_improvement,
        "hierarchical size weighting should dampen small-helper dilution",
    );
}

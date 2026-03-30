//! State cardinality loss dimension: type and function state-space scoring.

use crate::compliance::{
    ComplianceDimension, ComplianceDimensionPipeline, CompliancePipelineObservation,
    CompliancePipelineSubjectKind, CompliancePolicy, HeatmapContext, HeatmapEntry,
};
use crate::loss::common::{
    aggregate_pipeline_observations, pipeline_cohort, reference_scale_for, resolved_calibrator,
};
use crate::metrics::TypeMetrics;
use crate::normalization::{NormalizationCohort, NormalizationContext};

pub(crate) fn state_cardinality_raw(log2_cardinality: f64) -> f64 {
    log2_cardinality.max(0.0)
}

/// Per-type and per-function score: `min(1, (K+1) / (1 + log2_card))`,
/// dimension = geometric mean.
///
/// Types always participate. Functions participate only when they are production
/// (non-test) and have `internal_state_cardinality_log2 > 0.0`.
/// Items with `log2_card <= K` score 1.0 (no penalty).
#[allow(clippy::too_many_arguments)]
pub(crate) fn state_cardinality_observations(
    types: &[TypeMetrics],
    functions: &[crate::metrics::FunctionMetrics],
    type_normalizer: crate::normalization::Normalizer,
    type_calibrator: crate::calibration::Calibrator,
    function_normalizer: crate::normalization::Normalizer,
    function_calibrator: crate::calibration::Calibrator,
    semantic: Option<&crate::semantic::SemanticOverlay>,
) -> Vec<CompliancePipelineObservation> {
    let mut observations: Vec<CompliancePipelineObservation> =
        Vec::with_capacity(types.len() + functions.len());

    for t in types {
        // Use resolved cardinality from semantic overlay when available,
        // falling back to the syn-computed value.
        let cardinality_log2 = semantic
            .and_then(|s| s.type_cardinality(&t.file, &t.module_path, &t.name))
            .unwrap_or(t.state_cardinality_log2);
        let raw = state_cardinality_raw(cardinality_log2);
        let calibrated = type_calibrator.calibrate(type_normalizer.normalize(raw));
        observations.push(CompliancePipelineObservation {
            kind: CompliancePipelineSubjectKind::Type,
            file: t.file.clone(),
            line: t.line,
            name: t.name.clone(),
            detail: format!("log2 cardinality {cardinality_log2:.1}"),
            cohort: Some(NormalizationCohort::StateCardinalityType),
            // Hierarchical weighting currently treats type size as field count.
            size_hint: t.total_fields.max(1) as f64,
            raw,
            normalized: Some(calibrated.normalized),
            calibrated_score: calibrated.score,
            scope_path: t.scope_path.clone(),
        });
    }

    for f in functions
        .iter()
        .filter(|f| !f.is_test && f.internal_state_cardinality_log2 > 0.0)
    {
        // Use resolved cardinality from semantic overlay when available,
        // falling back to the syn-computed value.
        let cardinality_log2 = semantic
            .and_then(|s| s.function_cardinality(&f.file, &f.module_path, &f.name, f.line))
            .unwrap_or(f.internal_state_cardinality_log2);
        let raw = state_cardinality_raw(cardinality_log2);
        let calibrated = function_calibrator.calibrate(function_normalizer.normalize(raw));
        observations.push(CompliancePipelineObservation {
            kind: CompliancePipelineSubjectKind::Function,
            file: f.file.clone(),
            line: f.line,
            name: f.name.clone(),
            detail: format!("fn internal log2 cardinality {cardinality_log2:.1}"),
            cohort: Some(NormalizationCohort::StateCardinalityFunction),
            // Hierarchical weighting currently treats function size as lines of code.
            size_hint: f.lines.max(1) as f64,
            raw,
            normalized: Some(calibrated.normalized),
            calibrated_score: calibrated.score,
            scope_path: f.scope_path.clone(),
        });
    }

    observations
}

pub(crate) fn state_cardinality_compliance(
    types: &[TypeMetrics],
    functions: &[crate::metrics::FunctionMetrics],
    policy: &CompliancePolicy,
    normalization_context: &NormalizationContext,
    semantic: Option<&crate::semantic::SemanticOverlay>,
) -> ComplianceDimension {
    let reference_scale = reference_scale_for(policy, NormalizationCohort::StateCardinalityType);
    let type_normalizer = policy.normalization.normalizer_for(
        normalization_context,
        NormalizationCohort::StateCardinalityType,
        reference_scale,
    );
    let function_normalizer = policy.normalization.normalizer_for(
        normalization_context,
        NormalizationCohort::StateCardinalityFunction,
        reference_scale,
    );
    let type_calibrator = resolved_calibrator(policy, NormalizationCohort::StateCardinalityType);
    let function_calibrator =
        resolved_calibrator(policy, NormalizationCohort::StateCardinalityFunction);
    let rule_label = "normalize(log2_card)";

    let observations = state_cardinality_observations(
        types,
        functions,
        type_normalizer,
        type_calibrator,
        function_normalizer,
        function_calibrator,
        semantic,
    );

    let item_count = observations.len();
    let score = aggregate_pipeline_observations(
        policy.aggregation.state_cardinality_aggregation,
        &observations,
    );

    ComplianceDimension {
        name: "state_cardinality".into(),
        score,
        item_count,
        rule: format!(
            "{} type={:?}, fn={:?}; calibrate type={:?}, fn={:?}; aggregate with {:?}",
            rule_label,
            type_normalizer,
            function_normalizer,
            type_calibrator,
            function_calibrator,
            policy.aggregation.state_cardinality_aggregation
        ),
        pipeline: ComplianceDimensionPipeline {
            cohorts: vec![
                pipeline_cohort(
                    NormalizationCohort::StateCardinalityType,
                    reference_scale,
                    type_normalizer,
                    type_calibrator,
                ),
                pipeline_cohort(
                    NormalizationCohort::StateCardinalityFunction,
                    reference_scale,
                    function_normalizer,
                    function_calibrator,
                ),
            ],
            artifact_aggregation: Some(policy.aggregation.state_cardinality_aggregation),
            aggregated_score: score,
            observations,
        },
    }
}

/// Attribute state_cardinality dimension loss to individual types and functions.
///
/// Only types/functions with non-zero calibrated loss appear in the emitted
/// heatmap, but the counterfactual aggregation still includes fully compliant
/// participants with score `1.0`.
pub(crate) fn emit_state_cardinality_heatmap(
    ctx: &HeatmapContext<'_>,
    entries: &mut Vec<HeatmapEntry>,
) {
    crate::loss::common::emit_observation_based_heatmap(
        "state_cardinality",
        ctx.policy.aggregation.state_cardinality_aggregation,
        ctx,
        entries,
    );
}

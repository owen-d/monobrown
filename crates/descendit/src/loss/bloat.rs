//! Bloat loss dimension: per-function line-count scoring.

use crate::compliance::{
    ComplianceDimension, ComplianceDimensionPipeline, CompliancePipelineObservation,
    CompliancePipelineSubjectKind, CompliancePolicy, HeatmapContext, HeatmapEntry,
};
use crate::loss::common::{
    aggregate_pipeline_observations, pipeline_cohort, reference_scale_for, resolved_calibrator,
};
use crate::normalization::{NormalizationCohort, NormalizationContext, Normalizer};

pub(crate) fn bloat_raw(lines: f64) -> f64 {
    lines.max(1.0).log2()
}

/// Bloat: per-function `score = min(1.0, K / lines)`, aggregated via geometric
/// mean across all non-test production functions.
///
/// Functions at or below `max_function_lines` get score 1.0 (no penalty).
/// Functions above the threshold decay as `K / lines`.
pub(crate) fn bloat_observations(
    production: &[&crate::metrics::FunctionMetrics],
    normalizer: Normalizer,
    calibrator: crate::calibration::Calibrator,
) -> Vec<CompliancePipelineObservation> {
    production
        .iter()
        .map(|f| {
            let raw = bloat_raw(f.lines as f64);
            let calibrated = calibrator.calibrate(normalizer.normalize(raw));
            CompliancePipelineObservation {
                kind: CompliancePipelineSubjectKind::Function,
                file: f.file.clone(),
                line: f.line,
                name: f.name.clone(),
                detail: format!("{} lines", f.lines),
                cohort: Some(NormalizationCohort::BloatFunction),
                size_hint: f.lines.max(1) as f64,
                raw,
                normalized: Some(calibrated.normalized),
                calibrated_score: calibrated.score,
                scope_path: f.scope_path.clone(),
            }
        })
        .collect()
}

pub(crate) fn bloat_compliance(
    functions: &[crate::metrics::FunctionMetrics],
    policy: &CompliancePolicy,
    normalization_context: &NormalizationContext,
) -> ComplianceDimension {
    let reference_scale = reference_scale_for(policy, NormalizationCohort::BloatFunction);
    let normalizer = policy.normalization.normalizer_for(
        normalization_context,
        NormalizationCohort::BloatFunction,
        reference_scale,
    );
    let calibrator = resolved_calibrator(policy, NormalizationCohort::BloatFunction);
    let rule_label = "normalize(log2(lines))";
    let production: Vec<_> = functions.iter().filter(|f| !f.is_test).collect();
    let observations = bloat_observations(&production, normalizer, calibrator);
    let score =
        aggregate_pipeline_observations(policy.aggregation.bloat_aggregation, &observations);
    let item_count = production.len();
    // Empty production is vacuously compliant.
    let score = if production.is_empty() { 1.0 } else { score };

    ComplianceDimension {
        name: "bloat".into(),
        score,
        item_count,
        rule: format!(
            "{} with {:?}; calibrate with {:?}; aggregate with {:?}",
            rule_label, normalizer, calibrator, policy.aggregation.bloat_aggregation
        ),
        pipeline: ComplianceDimensionPipeline {
            cohorts: vec![pipeline_cohort(
                NormalizationCohort::BloatFunction,
                reference_scale,
                normalizer,
                calibrator,
            )],
            artifact_aggregation: Some(policy.aggregation.bloat_aggregation),
            aggregated_score: score,
            observations,
        },
    }
}

/// Attribute bloat dimension loss to individual production functions.
///
/// Only functions with non-zero calibrated loss appear in the emitted heatmap,
/// but the counterfactual aggregation still includes fully compliant
/// functions with score `1.0`.
pub(crate) fn emit_bloat_heatmap(ctx: &HeatmapContext<'_>, entries: &mut Vec<HeatmapEntry>) {
    crate::loss::common::emit_observation_based_heatmap(
        "bloat",
        ctx.policy.aggregation.bloat_aggregation,
        ctx,
        entries,
    );
}

//! Code economy loss dimension: function overhead ratio scoring.

use crate::calibration::SCORE_TOLERANCE;
use crate::compliance::{
    ComplianceDimension, ComplianceDimensionPipeline, CompliancePipelineObservation,
    CompliancePipelineSubjectKind, CompliancePolicy, HeatmapContext, HeatmapEntry,
};
use crate::loss::common::{
    counterfactual_composite, pipeline_cohort, reference_scale_for, resolved_calibrator,
};
use crate::metrics::AnalysisReport;
use crate::normalization::{NormalizationCohort, NormalizationContext};

pub(crate) fn code_economy_raw(overhead_ratio: f64) -> f64 {
    overhead_ratio.max(1.0).log2()
}

/// Code economy: `score = min(1.0, threshold / actual_ratio)`.
///
/// Ratios at or below `max_function_overhead_ratio` get score 1.0 (no penalty).
/// Ratios above the threshold decay as `threshold / ratio`.
/// When there is no adjusted public API surface the score is 1.0 (vacuously compliant).
pub(crate) fn code_economy_compliance(
    report: &AnalysisReport,
    policy: &CompliancePolicy,
    normalization_context: &NormalizationContext,
) -> ComplianceDimension {
    let adjusted_public =
        report.summary.public_function_count + report.summary.macro_export_fn_count;
    let adjusted_production =
        report.summary.production_function_count + report.summary.macro_fn_count;
    let raw = if adjusted_public == 0 {
        0.0
    } else {
        code_economy_raw(report.summary.function_overhead_ratio)
    };
    let reference_scale = reference_scale_for(policy, NormalizationCohort::CodeEconomy);
    let normalizer = policy.normalization.normalizer_for(
        normalization_context,
        NormalizationCohort::CodeEconomy,
        reference_scale,
    );
    let calibrator = resolved_calibrator(policy, NormalizationCohort::CodeEconomy);
    let calibrated = calibrator.calibrate(normalizer.normalize(raw));
    let is_vacuous = adjusted_public == 0;
    let score = if is_vacuous { 1.0 } else { calibrated.score };

    let ratio = report.summary.function_overhead_ratio;
    let macro_overhead_count = report
        .summary
        .macro_fn_count
        .saturating_sub(report.summary.macro_export_fn_count);

    let observations = overhead_observations(report, adjusted_public, macro_overhead_count, ratio);

    ComplianceDimension {
        name: "code_economy".into(),
        score,
        item_count: adjusted_production,
        rule: format!(
            "normalize(log2(overhead_ratio)) with {normalizer:?}; calibrate with {calibrator:?}",
        ),
        pipeline: ComplianceDimensionPipeline {
            cohorts: vec![pipeline_cohort(
                NormalizationCohort::CodeEconomy,
                reference_scale,
                normalizer,
                calibrator,
            )],
            artifact_aggregation: None,
            aggregated_score: score,
            observations,
        },
    }
}

/// Attribute code_economy dimension loss to non-public overhead functions.
///
/// Only emits heatmap entries when the shaped code-economy score incurs
/// non-zero loss.
pub(crate) fn emit_code_economy_heatmap(
    report: &AnalysisReport,
    ctx: &HeatmapContext<'_>,
    entries: &mut Vec<HeatmapEntry>,
) {
    let policy = ctx.policy;
    let normalization_context = ctx.normalization_context;
    let adjusted_public =
        report.summary.public_function_count + report.summary.macro_export_fn_count;
    let raw = if adjusted_public == 0 {
        0.0
    } else {
        code_economy_raw(report.summary.function_overhead_ratio)
    };
    let normalizer = policy.normalization.normalizer_for(
        normalization_context,
        NormalizationCohort::CodeEconomy,
        reference_scale_for(policy, NormalizationCohort::CodeEconomy),
    );
    let calibrator = resolved_calibrator(policy, NormalizationCohort::CodeEconomy);
    let calibrated = calibrator.calibrate(normalizer.normalize(raw));

    let macro_overhead_count = report
        .summary
        .macro_fn_count
        .saturating_sub(report.summary.macro_export_fn_count);
    let overhead_fns: Vec<_> = report
        .functions
        .iter()
        .filter(|f| !f.is_test && !f.is_pub)
        .collect();
    let overhead_unit_count = overhead_fns.len() + macro_overhead_count;

    if adjusted_public == 0 || overhead_unit_count == 0 {
        return;
    }

    if !calibrated.has_loss() || (ctx.dim.score - 1.0).abs() < SCORE_TOLERANCE {
        return;
    }

    let cf_composite = counterfactual_composite(
        ctx.dim_scores,
        ctx.dim_index,
        1.0,
        policy.aggregation.objective_scalarization,
    );
    let total_responsibility = cf_composite - ctx.composite_score;
    if total_responsibility < 1e-12 {
        return;
    }

    let share = total_responsibility / overhead_unit_count as f64;

    for f in &overhead_fns {
        entries.push(HeatmapEntry {
            file: f.file.clone(),
            line: f.line,
            function_name: f.name.clone(),
            dimension: "code_economy".into(),
            responsibility: share,
            detail: "non-pub overhead fn".into(),
            scope_path: f.scope_path.clone(),
        });
    }

    if macro_overhead_count > 0 {
        entries.push(HeatmapEntry {
            file: "<codebase>".into(),
            line: 0,
            function_name: "<macro_rules>".into(),
            dimension: "code_economy".into(),
            responsibility: share * macro_overhead_count as f64,
            detail: format!("{macro_overhead_count} non-exported control-flow macro(s)"),
            scope_path: Vec::new(),
        });
    }
}

/// Build per-function pipeline observations for overhead attribution.
fn overhead_observations(
    report: &AnalysisReport,
    adjusted_public: usize,
    macro_overhead_count: usize,
    ratio: f64,
) -> Vec<CompliancePipelineObservation> {
    let mut observations: Vec<CompliancePipelineObservation> = report
        .functions
        .iter()
        .filter(|f| !f.is_test && !f.is_pub)
        .map(|f| CompliancePipelineObservation {
            kind: CompliancePipelineSubjectKind::Function,
            file: f.file.clone(),
            line: f.line,
            name: f.name.clone(),
            detail: format!(
                "non-pub overhead fn (contributes 1/{adjusted_public} to overhead ratio {ratio:.2})"
            ),
            cohort: Some(NormalizationCohort::CodeEconomy),
            size_hint: 1.0,
            raw: 1.0,
            normalized: None,
            calibrated_score: 0.0,
            scope_path: f.scope_path.clone(),
        })
        .collect();

    if macro_overhead_count > 0 {
        observations.push(CompliancePipelineObservation {
            kind: CompliancePipelineSubjectKind::Function,
            file: "<codebase>".into(),
            line: 0,
            name: "<macro_rules>".into(),
            detail: format!(
                "{macro_overhead_count} non-exported control-flow macro(s) \
                 (contributes {macro_overhead_count}/{adjusted_public} to overhead ratio {ratio:.2})"
            ),
            cohort: Some(NormalizationCohort::CodeEconomy),
            size_hint: 1.0,
            raw: 1.0,
            normalized: None,
            calibrated_score: 0.0,
            scope_path: Vec::new(),
        });
    }

    observations
}

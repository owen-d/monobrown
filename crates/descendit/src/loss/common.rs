//! Shared helpers used across multiple loss dimension implementations.

use crate::aggregation::{
    ArtifactAggregation, ArtifactAggregationObservation, ObjectiveScalarization,
    aggregate_artifact_observations, scalarize_dimension_scores,
};
use crate::calibration::Calibrator;
use crate::compliance::{
    CompliancePipelineCohort, CompliancePipelineObservation, CompliancePolicy, HeatmapContext,
    HeatmapEntry,
};
use crate::normalization::{NormalizationCohort, Normalizer};

pub(crate) fn reference_scale_for(policy: &CompliancePolicy, cohort: NormalizationCohort) -> f64 {
    match cohort {
        NormalizationCohort::CodeEconomy => policy.directional.code_economy_log2_overhead,
        NormalizationCohort::StateCardinalityType
        | NormalizationCohort::StateCardinalityFunction => {
            policy.directional.state_cardinality_log2
        }
        NormalizationCohort::BloatFunction => policy.directional.bloat_log2_lines,
        NormalizationCohort::CouplingModule => policy.directional.coupling_density_edges,
    }
}

/// Resolve the calibrator for a given cohort, with per-dimension defaults.
///
/// Each dimension's default calibrator is chosen based on its metric's
/// characteristics:
///
/// **Bloat** and **state cardinality** use `StretchedExponentialDecay` because
/// their low-end values are essentially unavoidable (a function needs *some*
/// lines; a type needs *some* state). Plain exponential decay would waste most
/// of its penalty budget on the unavoidable low end. The stretched variant
/// (Weibull shape) concentrates penalty where the metric is actionable:
///
///   - Bloat (shape=3, scale≈log2(45)): a 10-line function is nearly free;
///     the 50–70 line range gets strong pressure.
///   - State cardinality (shape=2, scale=4): 1–2 bools cost little; the
///     half-life sits at 4 bits (16 states), and 6+ bits is severely penalized.
///
///     ```text
///     bits   states      score (shape=2, scale=4)
///      0       1         1.000
///      1       2         0.971
///      2       4         0.891
///      3       8         0.749
///      4      16         0.500  ← half-life
///      5      32         0.214
///      6      64         0.052
///     ```
///
/// **Code economy** uses plain `ExponentialDecayScore` because its raw metric
/// is already `log2(overhead_ratio)`, which compresses the dynamic range. An
/// overhead ratio of 2 maps to raw=1.0 and a ratio of 8 maps to raw=3.0 — the
/// range is narrow enough that the marginal-penalty skew of stretched
/// exponential doesn't matter much. Unlike bloat/cardinality, a ratio of 1
/// (every function is public) is genuinely good, not unavoidable overhead, so
/// there's no "free low end" to protect.
pub(crate) fn resolved_calibrator(
    policy: &CompliancePolicy,
    cohort: NormalizationCohort,
) -> Calibrator {
    policy
        .calibration
        .overrides
        .get(&cohort)
        .copied()
        .unwrap_or(match cohort {
            NormalizationCohort::BloatFunction => Calibrator::StretchedExponentialDecay {
                half_life: 1.0,
                shape: 3.0,
            },
            NormalizationCohort::StateCardinalityType
            | NormalizationCohort::StateCardinalityFunction => {
                Calibrator::StretchedExponentialDecay {
                    half_life: 1.0,
                    shape: 2.0,
                }
            }
            _ => Calibrator::ExponentialDecayScore { half_life: 1.0 },
        })
}

pub(crate) fn pipeline_cohort(
    cohort: NormalizationCohort,
    reference_scale: f64,
    normalizer: Normalizer,
    calibrator: Calibrator,
) -> CompliancePipelineCohort {
    CompliancePipelineCohort {
        cohort,
        reference_scale,
        normalizer,
        calibrator,
    }
}

pub(crate) fn observation_has_loss(observation: &CompliancePipelineObservation) -> bool {
    crate::CalibratedMetric {
        raw: observation.raw,
        normalized: observation.normalized.unwrap_or(0.0),
        score: observation.calibrated_score,
    }
    .has_loss()
}

pub(crate) fn aggregate_pipeline_observations(
    policy: ArtifactAggregation,
    observations: &[CompliancePipelineObservation],
) -> f64 {
    let aggregation_observations: Vec<_> = observations
        .iter()
        .map(|observation| ArtifactAggregationObservation {
            file: &observation.file,
            score: observation.calibrated_score,
            size_hint: observation.size_hint,
        })
        .collect();
    aggregate_artifact_observations(policy, &aggregation_observations)
}

/// Compute the scalarized composite of `scores` with `scores[replace_index]`
/// replaced by `new_value`.
pub(crate) fn counterfactual_composite(
    scores: &[f64],
    replace_index: usize,
    new_value: f64,
    scalarization: ObjectiveScalarization,
) -> f64 {
    let modified: Vec<f64> = scores
        .iter()
        .enumerate()
        .map(|(i, &s)| if i == replace_index { new_value } else { s })
        .collect();
    scalarize_dimension_scores(scalarization, &modified)
}

pub(crate) struct ItemHeatmapSource {
    pub file: String,
    pub line: usize,
    pub function_name: String,
    pub detail: String,
    pub population_index: usize,
    pub scope_path: Vec<crate::metrics::ScopeSegment>,
}

pub(crate) fn emit_itemized_dimension_heatmap(
    dimension_name: &str,
    observations: &[CompliancePipelineObservation],
    items: &[ItemHeatmapSource],
    artifact_aggregation: ArtifactAggregation,
    ctx: &HeatmapContext<'_>,
    entries: &mut Vec<HeatmapEntry>,
) {
    if items.is_empty() {
        return;
    }

    // Recompute against the full scoring population, including compliant items
    // that already score 1.0, so the heatmap counterfactual matches the same
    // artifact population used by the dimension score itself.
    let recomputed_dim_score = aggregate_pipeline_observations(artifact_aggregation, observations);
    debug_assert!(
        (recomputed_dim_score - ctx.dim.score).abs() < 1e-10,
        "heatmap population must match dimension scoring for {dimension_name}: recomputed={recomputed_dim_score}, actual={}",
        ctx.dim.score,
    );

    let objective_scalarization = ctx.policy.aggregation.objective_scalarization;
    let mut modified_observations: Vec<ArtifactAggregationObservation<'_>> = observations
        .iter()
        .map(|observation| ArtifactAggregationObservation {
            file: &observation.file,
            score: observation.calibrated_score,
            size_hint: observation.size_hint,
        })
        .collect();

    for item in items {
        let original_score = modified_observations[item.population_index].score;
        modified_observations[item.population_index].score = 1.0;
        let cf_dim_score =
            aggregate_artifact_observations(artifact_aggregation, &modified_observations);
        modified_observations[item.population_index].score = original_score;
        let cf_composite = counterfactual_composite(
            ctx.dim_scores,
            ctx.dim_index,
            cf_dim_score,
            objective_scalarization,
        );
        let responsibility = cf_composite - ctx.composite_score;
        if responsibility < 1e-12 {
            continue;
        }

        entries.push(HeatmapEntry {
            file: item.file.clone(),
            line: item.line,
            function_name: item.function_name.clone(),
            dimension: dimension_name.into(),
            responsibility,
            detail: item.detail.clone(),
            scope_path: item.scope_path.clone(),
        });
    }
}

/// Shared pattern for observation-based heatmap emission.
///
/// Filters observations for those with loss, maps to `ItemHeatmapSource`,
/// and delegates to `emit_itemized_dimension_heatmap`.
pub(crate) fn emit_observation_based_heatmap(
    dimension_name: &str,
    artifact_aggregation: ArtifactAggregation,
    ctx: &HeatmapContext<'_>,
    entries: &mut Vec<HeatmapEntry>,
) {
    let items: Vec<ItemHeatmapSource> = ctx
        .dim
        .pipeline
        .observations
        .iter()
        .enumerate()
        .filter_map(|(population_index, observation)| {
            if !observation_has_loss(observation) {
                return None;
            }
            Some(ItemHeatmapSource {
                file: observation.file.clone(),
                line: observation.line,
                function_name: observation.name.clone(),
                detail: observation.detail.clone(),
                population_index,
                scope_path: observation.scope_path.clone(),
            })
        })
        .collect();

    emit_itemized_dimension_heatmap(
        dimension_name,
        &ctx.dim.pipeline.observations,
        &items,
        artifact_aggregation,
        ctx,
        entries,
    );
}

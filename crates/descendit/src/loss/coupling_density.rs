//! Coupling density loss dimension: per-module call graph edge scoring.
//!
//! When the full `SemanticOverlay` is available, each module gets an observation
//! with `raw = outgoing_edge_count`, normalized and calibrated through the
//! standard pipeline. When only the `SemanticSummary` is available (no overlay),
//! falls back to a single codebase-level observation with `score = 1 - density`.

use crate::calibration::Calibrator;
use crate::compliance::{
    ComplianceDimension, ComplianceDimensionPipeline, CompliancePipelineObservation,
    CompliancePipelineSubjectKind, CompliancePolicy, HeatmapContext, HeatmapEntry,
};
use crate::loss::common::{
    aggregate_pipeline_observations, pipeline_cohort, reference_scale_for, resolved_calibrator,
};
use crate::normalization::{NormalizationCohort, NormalizationContext, Normalizer};

/// Raw metric for a module's coupling: simply the outgoing edge count.
pub(crate) fn coupling_density_raw(edge_count: usize) -> f64 {
    edge_count as f64
}

/// Build coupling density observations from the semantic overlay.
///
/// When per-function data is available (`function_outgoing_edges` is non-empty),
/// emits one observation per function with `kind: Function`. Modules that only
/// appear as callees (0 outgoing edges, no functions) still get `kind: Module`
/// observations with `raw: 0` so they participate in normalization.
///
/// When per-function data is empty (legacy data), falls back to per-module
/// observations.
pub(crate) fn coupling_density_observations(
    coupling: &crate::semantic::CouplingData,
    normalizer: Normalizer,
    calibrator: Calibrator,
) -> Vec<CompliancePipelineObservation> {
    if coupling.function_outgoing_edges.is_empty() {
        return coupling_density_observations_per_module(coupling, normalizer, calibrator);
    }

    let mut observations = Vec::new();

    // Track which modules have at least one function observation.
    let mut modules_with_functions: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for ((module, function, line), &edge_count) in &coupling.function_outgoing_edges {
        modules_with_functions.insert(module.clone());
        let raw = coupling_density_raw(edge_count);
        let calibrated = calibrator.calibrate(normalizer.normalize(raw));
        let file = coupling
            .function_files
            .get(&(module.clone(), function.clone(), *line))
            .cloned()
            .unwrap_or_else(|| "<codebase>".into());
        observations.push(CompliancePipelineObservation {
            kind: CompliancePipelineSubjectKind::Function,
            file,
            line: *line,
            name: format!("{module}::{function}"),
            detail: format!("{edge_count} outgoing cross-module edge(s)"),
            cohort: Some(NormalizationCohort::CouplingModule),
            size_hint: 1.0,
            raw,
            normalized: Some(calibrated.normalized),
            calibrated_score: calibrated.score,
            scope_path: vec![
                crate::metrics::ScopeSegment::Module(module.clone()),
                crate::metrics::ScopeSegment::Function(function.clone()),
            ],
        });
    }

    // Modules that only appear as callees (0 outgoing edges, no functions)
    // still need observations so they participate in normalization.
    for module in &coupling.all_modules {
        if modules_with_functions.contains(module) {
            continue;
        }
        let raw = coupling_density_raw(0);
        let calibrated = calibrator.calibrate(normalizer.normalize(raw));
        let file = coupling
            .module_files
            .get(module)
            .cloned()
            .unwrap_or_else(|| "<codebase>".into());
        observations.push(CompliancePipelineObservation {
            kind: CompliancePipelineSubjectKind::Module,
            file,
            line: 0,
            name: module.clone(),
            detail: "0 outgoing cross-module edge(s)".into(),
            cohort: Some(NormalizationCohort::CouplingModule),
            size_hint: 1.0,
            raw,
            normalized: Some(calibrated.normalized),
            calibrated_score: calibrated.score,
            scope_path: vec![crate::metrics::ScopeSegment::Module(module.clone())],
        });
    }

    observations
}

/// Per-module observations (legacy fallback when no per-function data).
fn coupling_density_observations_per_module(
    coupling: &crate::semantic::CouplingData,
    normalizer: Normalizer,
    calibrator: Calibrator,
) -> Vec<CompliancePipelineObservation> {
    coupling
        .all_modules
        .iter()
        .map(|module| {
            let edge_count = coupling
                .module_outgoing_edges
                .get(module)
                .copied()
                .unwrap_or(0);
            let raw = coupling_density_raw(edge_count);
            let calibrated = calibrator.calibrate(normalizer.normalize(raw));
            let file = coupling
                .module_files
                .get(module)
                .cloned()
                .unwrap_or_else(|| "<codebase>".into());
            CompliancePipelineObservation {
                kind: CompliancePipelineSubjectKind::Module,
                file,
                line: 0,
                name: module.clone(),
                detail: format!("{edge_count} outgoing cross-module edge(s)"),
                cohort: Some(NormalizationCohort::CouplingModule),
                size_hint: 1.0,
                raw,
                normalized: Some(calibrated.normalized),
                calibrated_score: calibrated.score,
                scope_path: vec![crate::metrics::ScopeSegment::Module(module.clone())],
            }
        })
        .collect()
}

/// Coupling density compliance dimension.
///
/// When the full `SemanticOverlay` is available: builds per-module observations,
/// aggregates via the pipeline, and records `artifact_aggregation`.
///
/// When only `SemanticSummary` is available (no overlay): emits a single
/// codebase-level observation with `score = 1 - density` (backward compat).
///
/// When no coupling data at all: score = 1.0 (vacuous).
pub(crate) fn coupling_density_compliance(
    semantic: Option<&crate::semantic::SemanticOverlay>,
    semantic_summary: Option<&crate::metrics::SemanticSummary>,
    policy: &CompliancePolicy,
    normalization_context: &NormalizationContext,
) -> ComplianceDimension {
    // Full overlay path: per-module observations through the pipeline.
    if let Some(s) = semantic {
        let reference_scale = reference_scale_for(policy, NormalizationCohort::CouplingModule);
        let normalizer = policy.normalization.normalizer_for(
            normalization_context,
            NormalizationCohort::CouplingModule,
            reference_scale,
        );
        let calibrator = resolved_calibrator(policy, NormalizationCohort::CouplingModule);

        let observations = coupling_density_observations(&s.coupling, normalizer, calibrator);
        let item_count = observations.len();
        let artifact_aggregation = policy.aggregation.coupling_density_aggregation;
        let score = if observations.is_empty() {
            1.0
        } else {
            aggregate_pipeline_observations(artifact_aggregation, &observations)
        };

        return ComplianceDimension {
            name: "coupling_density".into(),
            score,
            item_count,
            rule: format!(
                "normalize(edges) with {normalizer:?}; calibrate with {calibrator:?}; aggregate with {artifact_aggregation:?}",
            ),
            pipeline: ComplianceDimensionPipeline {
                cohorts: vec![pipeline_cohort(
                    NormalizationCohort::CouplingModule,
                    reference_scale,
                    normalizer,
                    calibrator,
                )],
                artifact_aggregation: Some(artifact_aggregation),
                aggregated_score: score,
                observations,
            },
        };
    }

    // Fallback: SemanticSummary or no data at all.
    coupling_density_fallback(semantic_summary)
}

/// Fallback path when no full semantic overlay is available.
fn coupling_density_fallback(
    semantic_summary: Option<&crate::metrics::SemanticSummary>,
) -> ComplianceDimension {
    let (density, detail) = if let Some(ss) = semantic_summary {
        (
            ss.coupling_density,
            format!(
                "density {:.4}, {} modules, {} cross-module edges (from saved semantic summary)",
                ss.coupling_density, ss.coupling_module_count, ss.coupling_edge_count
            ),
        )
    } else {
        (0.0, "no semantic data; vacuous score".into())
    };
    let score = 1.0 - density;

    ComplianceDimension {
        name: "coupling_density".into(),
        score,
        item_count: 0,
        rule: "score = 1 - density".into(),
        pipeline: ComplianceDimensionPipeline {
            cohorts: Vec::new(),
            artifact_aggregation: None,
            aggregated_score: score,
            observations: vec![CompliancePipelineObservation {
                kind: CompliancePipelineSubjectKind::Codebase,
                file: "<codebase>".into(),
                line: 0,
                name: "coupling_density".into(),
                detail,
                cohort: None,
                size_hint: 1.0,
                raw: density,
                normalized: None,
                calibrated_score: score,
                scope_path: Vec::new(),
            }],
        },
    }
}

/// Attribute coupling_density dimension loss to individual modules.
///
/// Follows the bloat/state_cardinality pattern: builds `ItemHeatmapSource` from
/// observations with loss, calls `emit_itemized_dimension_heatmap`.
pub(crate) fn emit_coupling_density_heatmap(
    ctx: &HeatmapContext<'_>,
    entries: &mut Vec<HeatmapEntry>,
) {
    let artifact_aggregation = match ctx.dim.pipeline.artifact_aggregation {
        Some(agg) => agg,
        // No artifact aggregation means fallback path (single codebase observation).
        // Use the old counterfactual-share approach for backward compat.
        None => return emit_coupling_density_heatmap_legacy(ctx, entries),
    };

    crate::loss::common::emit_observation_based_heatmap(
        "coupling_density",
        artifact_aggregation,
        ctx,
        entries,
    );
}

/// Legacy heatmap for the summary-only fallback path (no per-module observations).
fn emit_coupling_density_heatmap_legacy(ctx: &HeatmapContext<'_>, entries: &mut Vec<HeatmapEntry>) {
    use crate::calibration::SCORE_TOLERANCE;
    use crate::loss::common::counterfactual_composite;

    if (ctx.dim.score - 1.0).abs() < SCORE_TOLERANCE {
        return;
    }

    let cf_composite = counterfactual_composite(
        ctx.dim_scores,
        ctx.dim_index,
        1.0,
        ctx.policy.aggregation.objective_scalarization,
    );
    let total_responsibility = cf_composite - ctx.composite_score;
    if total_responsibility < 1e-12 {
        return;
    }

    // Single codebase-level observation in the fallback path.
    entries.push(HeatmapEntry {
        file: "<codebase>".into(),
        line: 0,
        function_name: "coupling_density".into(),
        dimension: "coupling_density".into(),
        responsibility: total_responsibility,
        detail: ctx.dim.pipeline.observations[0].detail.clone(),
        scope_path: Vec::new(),
    });
}

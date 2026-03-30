//! Duplication loss dimension: structural duplication scoring.

use std::collections::HashMap;

use crate::calibration::SCORE_TOLERANCE;
use crate::compliance::{
    ComplianceDimension, ComplianceDimensionPipeline, CompliancePipelineObservation,
    CompliancePipelineSubjectKind, HeatmapContext, HeatmapEntry,
};
use crate::loss::common::counterfactual_composite;
use crate::metrics::AnalysisReport;

/// Key for deduplicating function locations across duplicate groups/pairs.
type FnKey = (String, String, usize);

/// Duplication: `score = 1 - duplication_score`, with per-function observations
/// for each duplicated function.
pub(crate) fn duplication_compliance(report: &AnalysisReport) -> ComplianceDimension {
    let dup_score = report.duplication.duplication_score;
    let score = 1.0 - dup_score;
    let total_fn_count = report.duplication.functions_fingerprinted;

    let (detail_parts, scope_map) = collect_duplication_details(report);

    let mut observations: Vec<CompliancePipelineObservation> = detail_parts
        .into_iter()
        .map(|((name, file, line), parts)| {
            let key: FnKey = (name.clone(), file.clone(), line);
            let scope_path = scope_map.get(&key).cloned().unwrap_or_default();
            CompliancePipelineObservation {
                kind: CompliancePipelineSubjectKind::Function,
                file,
                line,
                name,
                detail: parts.join("; "),
                cohort: None,
                size_hint: 1.0,
                raw: 1.0,
                normalized: None,
                calibrated_score: 0.0,
                scope_path,
            }
        })
        .collect();
    observations.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.name.cmp(&b.name))
    });

    ComplianceDimension {
        name: "duplication".into(),
        score,
        item_count: total_fn_count,
        rule: "score = 1 - duplication_score".into(),
        pipeline: ComplianceDimensionPipeline {
            cohorts: Vec::new(),
            // Aggregation is None: the dimension score is computed directly as
            // 1 - duplication_score rather than re-derived from observations.
            // We only emit observations for duplicated functions (score 0.0);
            // non-duplicated functions implicitly score 1.0 but are not
            // enumerated in the DuplicationReport.
            artifact_aggregation: None,
            aggregated_score: score,
            observations,
        },
    }
}

/// Collect per-function detail strings and scope paths from the duplication report.
///
/// Returns `(detail_parts, scope_map)` where both are keyed by `(name, file, line)`.
fn collect_duplication_details(
    report: &AnalysisReport,
) -> (
    HashMap<FnKey, Vec<String>>,
    HashMap<FnKey, Vec<crate::metrics::ScopeSegment>>,
) {
    let mut detail_parts: HashMap<FnKey, Vec<String>> = HashMap::new();
    let mut scope_map: HashMap<FnKey, Vec<crate::metrics::ScopeSegment>> = HashMap::new();

    for group in &report.duplication.exact_duplicates {
        for loc in &group.functions {
            let key: FnKey = (loc.name.clone(), loc.file.clone(), loc.line);
            let other_names: Vec<&str> = group
                .functions
                .iter()
                .filter(|other| {
                    other.name != loc.name || other.file != loc.file || other.line != loc.line
                })
                .map(|other| other.name.as_str())
                .collect();
            let detail = format!(
                "exact duplicate of {} (shape_len {})",
                other_names.join(", "),
                group.shape_length,
            );
            detail_parts.entry(key.clone()).or_default().push(detail);
            scope_map
                .entry(key)
                .or_insert_with(|| loc.scope_path.clone());
        }
    }

    for pair in &report.duplication.near_duplicates {
        for (this, other) in [(&pair.a, &pair.b), (&pair.b, &pair.a)] {
            let key: FnKey = (this.name.clone(), this.file.clone(), this.line);
            let detail = format!(
                "near duplicate of {} (similarity {:.2})",
                other.name, pair.similarity,
            );
            detail_parts.entry(key.clone()).or_default().push(detail);
            scope_map
                .entry(key)
                .or_insert_with(|| this.scope_path.clone());
        }
    }

    (detail_parts, scope_map)
}

/// Attribute duplication dimension loss to functions in duplicate groups.
///
/// Uses equal-share attribution rather than `emit_itemized_dimension_heatmap`
/// because the per-function observations only cover duplicated functions, not
/// the full population. Counterfactual heatmap would require observations for
/// all `functions_fingerprinted` functions (including non-duplicated ones at
/// score 1.0), which the `DuplicationReport` does not enumerate.
pub(crate) fn emit_duplication_heatmap(ctx: &HeatmapContext<'_>, entries: &mut Vec<HeatmapEntry>) {
    // Score of 1.0 means fully compliant — nothing to attribute.
    if (ctx.dim.score - 1.0).abs() < SCORE_TOLERANCE {
        return;
    }

    let cf_composite = counterfactual_composite(
        ctx.dim_scores,
        ctx.dim_index,
        1.0,
        ctx.policy.aggregation.objective_scalarization,
    );
    let responsibility_per_fn = cf_composite - ctx.composite_score;

    let violators = &ctx.dim.pipeline.observations;
    let violator_count = violators.len();
    if violator_count == 0 {
        return;
    }

    // Each violating function gets an equal share of the counterfactual improvement.
    let share = responsibility_per_fn / violator_count as f64;

    for obs in violators {
        entries.push(HeatmapEntry {
            file: obs.file.clone(),
            line: obs.line,
            function_name: obs.name.clone(),
            dimension: "duplication".into(),
            responsibility: share,
            detail: obs.detail.clone(),
            scope_path: obs.scope_path.clone(),
        });
    }
}

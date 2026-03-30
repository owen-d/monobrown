//! Helpers for corpus-level scoring experiments.
//!
//! A corpus run builds a shared normalization context across many analysis
//! reports, then scores each target against that common context.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::loss::common::{reference_scale_for, resolved_calibrator};
use crate::{
    AnalysisReport, Calibrator, CohortStats, ComplianceDimensionPipeline, CompliancePolicy,
    ComplianceReport, HeatmapEntry, NormalizationCohort, NormalizationContext,
    NormalizationContextBuilder, Normalizer, SemanticOverlay, compute_compliance_with_context,
};

/// One named input to a corpus-level compliance run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusExperimentTarget {
    pub label: String,
    pub analysis: AnalysisReport,
    pub semantic: Option<SemanticOverlay>,
}

/// One named result from a corpus-level compliance run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusExperimentResult {
    pub label: String,
    pub composite_score: f64,
    pub dimensions: Vec<ExperimentDimensionSummary>,
    pub heatmap_summary: ExperimentHeatmapSummary,
    pub compliance: ComplianceReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentDimensionSummary {
    pub name: String,
    pub score: f64,
    pub loss: f64,
    pub item_count: usize,
    pub rule: String,
    pub pipeline: ComplianceDimensionPipeline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentHeatmapSummary {
    pub entry_count: usize,
    pub dimension_totals: Vec<ExperimentHeatmapDimensionSummary>,
    pub top_entries: Vec<HeatmapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentHeatmapDimensionSummary {
    pub dimension: String,
    pub total_responsibility: f64,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentContextStat {
    pub cohort: NormalizationCohort,
    pub stats: Option<CohortStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResolvedCohort {
    pub cohort: NormalizationCohort,
    pub reference_scale: f64,
    pub normalizer: Normalizer,
    pub calibrator: Calibrator,
}

/// Full result of scoring a set of reports against a shared normalization context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusExperimentRun {
    pub normalization_context: NormalizationContext,
    pub context_stats: Vec<ExperimentContextStat>,
    pub resolved_cohorts: Vec<ExperimentResolvedCohort>,
    pub results: Vec<CorpusExperimentResult>,
}

/// Build a shared normalization context across the corpus, then score each
/// target against that same context.
pub fn run_corpus_experiment(
    targets: &[CorpusExperimentTarget],
    policy: &CompliancePolicy,
) -> CorpusExperimentRun {
    let mut builder = NormalizationContextBuilder::default();
    for target in targets {
        builder.observe_report(&target.analysis);
    }
    let normalization_context = builder.build();
    let context_stats = NormalizationCohort::all()
        .iter()
        .map(|&cohort| ExperimentContextStat {
            cohort,
            stats: normalization_context.stats_for(cohort).copied(),
        })
        .collect();
    let resolved_cohorts = NormalizationCohort::all()
        .iter()
        .map(|&cohort| ExperimentResolvedCohort {
            cohort,
            reference_scale: reference_scale_for(policy, cohort),
            normalizer: policy.normalization.normalizer_for(
                &normalization_context,
                cohort,
                reference_scale_for(policy, cohort),
            ),
            calibrator: resolved_calibrator(policy, cohort),
        })
        .collect();

    let results = targets
        .iter()
        .map(|target| {
            let compliance = compute_compliance_with_context(
                &target.analysis,
                policy,
                &normalization_context,
                target.semantic.as_ref(),
            );
            CorpusExperimentResult {
                label: target.label.clone(),
                composite_score: compliance.composite_score,
                dimensions: compliance
                    .soft_dimensions
                    .iter()
                    .map(|dimension| ExperimentDimensionSummary {
                        name: dimension.name.clone(),
                        score: dimension.score,
                        loss: 1.0 - dimension.score,
                        item_count: dimension.item_count,
                        rule: dimension.rule.clone(),
                        pipeline: dimension.pipeline.clone(),
                    })
                    .collect(),
                heatmap_summary: summarize_heatmap(&compliance.heatmap, 10),
                compliance,
            }
        })
        .collect();

    CorpusExperimentRun {
        normalization_context,
        context_stats,
        resolved_cohorts,
        results,
    }
}

pub fn summarize_heatmap(entries: &[HeatmapEntry], top_n: usize) -> ExperimentHeatmapSummary {
    let mut totals: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for entry in entries {
        let aggregate = totals.entry(entry.dimension.clone()).or_insert((0.0, 0));
        aggregate.0 += entry.responsibility;
        aggregate.1 += 1;
    }

    let mut dimension_totals: Vec<ExperimentHeatmapDimensionSummary> = totals
        .into_iter()
        .map(
            |(dimension, (total_responsibility, entry_count))| ExperimentHeatmapDimensionSummary {
                dimension,
                total_responsibility,
                entry_count,
            },
        )
        .collect();
    dimension_totals.sort_by(|a, b| {
        b.total_responsibility
            .partial_cmp(&a.total_responsibility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.dimension.cmp(&b.dimension))
    });

    ExperimentHeatmapSummary {
        entry_count: entries.len(),
        dimension_totals,
        top_entries: entries.iter().take(top_n).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComplianceDimension, EntropyMetrics, FunctionMetrics, Summary};

    fn empty_report() -> AnalysisReport {
        AnalysisReport {
            analysis_root: None,
            files_analyzed: 0,
            total_lines: 0,
            functions: Vec::new(),
            types: Vec::new(),
            entropy: EntropyMetrics {
                total_tokens: 0,
                vocabulary_size: 0,
                entropy_bits: 0.0,
                normalized_entropy: 0.0,
                per_file: Vec::new(),
            },
            duplication: crate::DuplicationReport {
                functions_fingerprinted: 0,
                exact_duplicates: Vec::new(),
                near_duplicates: Vec::new(),
                duplication_score: 0.0,
            },
            summary: Summary::default(),
            semantic: None,
        }
    }

    fn bloaty_function(name: &str, lines: usize, cyclomatic: usize) -> FunctionMetrics {
        FunctionMetrics {
            name: name.into(),
            file: "lib.rs".into(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines,
            params: 0,
            nesting_depth: 1,
            cyclomatic,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }
    }

    fn find_dimension<'a>(report: &'a ComplianceReport, name: &str) -> &'a ComplianceDimension {
        report
            .soft_dimensions
            .iter()
            .find(|dimension| dimension.name == name)
            .unwrap_or_else(|| panic!("missing dimension {name}"))
    }

    #[test]
    fn test_run_corpus_experiment_builds_shared_context_and_scores_targets() {
        let mut first = empty_report();
        first.functions = vec![bloaty_function("small", 20, 2)];
        first.summary.production_function_count = 1;
        first.summary.public_function_count = 1;
        first.summary.function_overhead_ratio = 1.0;

        let mut second = empty_report();
        second.functions = vec![bloaty_function("large", 60, 2)];
        second.summary.production_function_count = 1;
        second.summary.public_function_count = 1;
        second.summary.function_overhead_ratio = 1.0;

        let policy = CompliancePolicy::default();
        let run = run_corpus_experiment(
            &[
                CorpusExperimentTarget {
                    label: "first".into(),
                    analysis: first,
                    semantic: None,
                },
                CorpusExperimentTarget {
                    label: "second".into(),
                    analysis: second,
                    semantic: None,
                },
            ],
            &policy,
        );

        assert_eq!(run.results.len(), 2);
        let Some(bloat_stats) = run
            .normalization_context
            .stats_for(crate::NormalizationCohort::BloatFunction)
        else {
            panic!("bloat stats should exist");
        };
        assert_eq!(bloat_stats.count, 2);
        assert_eq!(run.context_stats.len(), 5);
        assert_eq!(run.resolved_cohorts.len(), 5);
        assert_eq!(run.results[0].dimensions.len(), 5);
        let Some(bloat_dimension) = run.results[0]
            .dimensions
            .iter()
            .find(|dimension| dimension.name == "bloat")
        else {
            panic!("bloat dimension should exist");
        };
        assert_eq!(bloat_dimension.pipeline.observations.len(), 1);
        assert_eq!(
            run.results[0].composite_score,
            run.results[0].compliance.composite_score
        );
        assert_eq!(
            run.results[0].heatmap_summary.entry_count,
            run.results[0].compliance.heatmap.len()
        );
        assert!(
            find_dimension(&run.results[0].compliance, "bloat").score
                >= find_dimension(&run.results[1].compliance, "bloat").score
        );
    }
}

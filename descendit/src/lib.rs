//! descendit — Deterministic structural metrics for Rust source code.
//!
//! Parses Rust source files using `syn` and extracts quantifiable metrics
//! at the function, type, and codebase levels. All analysis is pure and
//! deterministic: the same source always produces the same report.
//!
//! # Usage
//!
//! ```no_run
//! use std::path::Path;
//! use descendit::analyze_path;
//!
//! let report = analyze_path(Path::new("src/")).unwrap();
//! println!("functions: {}", report.summary.function_count);
//! println!("max complexity: {}", report.summary.max_cyclomatic);
//! ```

pub mod aggregation;
pub mod analyze;
pub mod calibration;
pub mod compliance;
pub mod diff;
pub mod duplication;
pub mod experiment;
pub mod loss;
pub mod metrics;
pub mod normalization;
pub mod rollup;
pub mod semantic;
pub use aggregation::{
    ArtifactAggregation, ArtifactAggregationObservation, ArtifactSizeWeighting,
    ObjectiveScalarization, aggregate_artifact_observations, aggregate_artifact_scores,
    scalarize_dimension_scores,
};
pub use analyze::analyze_path;
pub use calibration::{CalibratedMetric, CalibrationPolicy, Calibrator, SCORE_TOLERANCE};
pub use compliance::{
    ComplianceAggregationPolicy, ComplianceDeltaError, ComplianceDimension,
    ComplianceDimensionPipeline, CompliancePipelineCohort, CompliancePipelineObservation,
    CompliancePipelineSubjectKind, CompliancePolicy, ComplianceReport, DirectionalScales,
    HeatmapContext, HeatmapEntry, LossDimension, LossFunction, LossMetadata, ScoringInfo,
    compliance_delta_to_loss_vector, compliance_to_loss_vector, compute_compliance,
    compute_compliance_with_context, compute_compliance_with_semantic,
};
pub use diff::{
    Assessment, DiffReport, Direction, LossEntry, LossValueOut, LossVectorOut, MetricDelta,
    RawMetricEntry, RawMetricValueOut, RawMetricVectorOut, diff_summaries,
};
pub use duplication::{
    DuplicateGroup, DuplicationReport, FunctionFingerprint, FunctionLocation, NearDuplicatePair,
    ShapeToken,
};
pub use experiment::{
    CorpusExperimentResult, CorpusExperimentRun, CorpusExperimentTarget, ExperimentContextStat,
    ExperimentDimensionSummary, ExperimentHeatmapDimensionSummary, ExperimentHeatmapSummary,
    ExperimentResolvedCohort, run_corpus_experiment, summarize_heatmap,
};
pub use metrics::{
    AnalysisReport, EntropyMetrics, FileEntropy, FunctionMetrics, ScopeSegment, SemanticSummary,
    Summary, TypeKind, TypeMetrics, module_path_from_scope,
};
pub use normalization::{
    CohortNormalizationStrategy, CohortStats, NormalizationCohort, NormalizationContext,
    NormalizationContextBuilder, NormalizationPolicy, NormalizedMetric, Normalizer, OnlineStats,
};
pub use rollup::{HeatmapTreeNode, build_heatmap_tree};
pub use semantic::{
    CallEdge, CouplingData, ResolvedFunctionCardinality, ResolvedTypeCardinality, SemanticData,
    SemanticOverlay,
};

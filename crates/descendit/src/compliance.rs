//! Structural compliance scoring.
//!
//! Converts raw metrics into normalized [0, 1] compliance scores where:
//! - 1.0 = perfect compliance (all code follows the rule)
//! - 0.0 = zero compliance (no code follows the rule)
//!
//! Each dimension maps raw metrics through an explicit pipeline:
//! raw metric -> normalization -> bounded score -> artifact aggregation.
//! The final composite then scalarizes dimension scores according to the active
//! objective policy. Defaults preserve the earlier geometric-mean behavior.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::aggregation::{ArtifactAggregation, ObjectiveScalarization, scalarize_dimension_scores};
use crate::calibration::CalibrationPolicy;
use crate::diff::{LossEntry, LossValueOut, LossVectorOut};
use crate::metrics::AnalysisReport;
use crate::normalization::{
    NormalizationCohort, NormalizationContext, NormalizationPolicy, Normalizer,
};

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// Reference scales for directional scoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectionalScales {
    /// One normalized unit corresponds to this many doublings of overhead ratio.
    #[serde(default = "default_code_economy_log2_scale")]
    pub code_economy_log2_overhead: f64,
    /// One normalized unit corresponds to this many bits of state-space growth.
    #[serde(default = "default_state_cardinality_log2_scale")]
    pub state_cardinality_log2: f64,
    /// One normalized unit corresponds to this many doublings of function lines.
    #[serde(default = "default_bloat_log2_lines_scale")]
    pub bloat_log2_lines: f64,
    /// One normalized unit corresponds to this many outgoing cross-module edges.
    #[serde(default = "default_coupling_density_edges")]
    pub coupling_density_edges: f64,
}

impl Default for DirectionalScales {
    fn default() -> Self {
        Self {
            code_economy_log2_overhead: default_code_economy_log2_scale(),
            state_cardinality_log2: default_state_cardinality_log2_scale(),
            bloat_log2_lines: default_bloat_log2_lines_scale(),
            coupling_density_edges: default_coupling_density_edges(),
        }
    }
}

/// Aggregation/scalarization policy for soft compliance dimensions.
///
/// `duplication` and `code_economy` are global dimensions, so only per-item
/// dimensions expose artifact-aggregation policy here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ComplianceAggregationPolicy {
    /// How state-cardinality item scores are aggregated into one dimension score.
    #[serde(default)]
    pub state_cardinality_aggregation: ArtifactAggregation,
    /// How bloat item scores are aggregated into one dimension score.
    #[serde(default)]
    pub bloat_aggregation: ArtifactAggregation,
    /// How coupling-density module scores are aggregated into one dimension score.
    #[serde(default)]
    pub coupling_density_aggregation: ArtifactAggregation,
    /// How dimension scores are scalarized into the final composite.
    #[serde(default)]
    pub objective_scalarization: ObjectiveScalarization,
}

/// Full soft-scoring policy.
///
/// Serialized as nested groups:
/// - `directional`
/// - `normalization`
/// - `calibration`
/// - `aggregation`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CompliancePolicy {
    #[serde(default)]
    pub directional: DirectionalScales,
    #[serde(default)]
    pub normalization: NormalizationPolicy,
    #[serde(default)]
    pub calibration: CalibrationPolicy,
    #[serde(default)]
    pub aggregation: ComplianceAggregationPolicy,
}

fn default_max_function_overhead_ratio() -> f64 {
    5.0
}

fn default_max_function_lines() -> f64 {
    45.0
}

fn default_code_economy_log2_scale() -> f64 {
    default_max_function_overhead_ratio().log2()
}

fn default_state_cardinality_log2_scale() -> f64 {
    4.0
}

fn default_bloat_log2_lines_scale() -> f64 {
    default_max_function_lines().log2()
}

fn default_coupling_density_edges() -> f64 {
    3.0
}

// ---------------------------------------------------------------------------
// Loss function catalog
// ---------------------------------------------------------------------------

/// Describes the scoring formula for a loss function.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ScoringInfo {
    pub formula: &'static str,
    pub notes: &'static str,
}

/// Descriptive metadata for a registered loss dimension.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LossMetadata {
    pub name: &'static str,
    pub description: &'static str,
    pub calculation: &'static str,
    pub aggregation: &'static str,
    pub scoring: ScoringInfo,
    /// Whether this dimension participates in the composite score.
    /// Diagnostic-only dimensions are still computed and reported but
    /// excluded from the geometric mean.
    pub composite: bool,
}

/// All known loss functions in the descendit system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LossFunction {
    Duplication,
    CodeEconomy,
    StateCardinality,
    Bloat,
    CouplingDensity,
}

impl LossFunction {
    pub fn all() -> &'static [LossFunction] {
        &[
            LossFunction::Duplication,
            LossFunction::CodeEconomy,
            LossFunction::StateCardinality,
            LossFunction::Bloat,
            LossFunction::CouplingDensity,
        ]
    }

    /// Whether this dimension participates in the composite score.
    pub fn is_composite(&self) -> bool {
        self.metadata().composite
    }

    pub fn name(&self) -> &'static str {
        self.metadata().name
    }

    pub fn description(&self) -> &'static str {
        self.metadata().description
    }

    pub fn scoring_info(&self) -> ScoringInfo {
        self.metadata().scoring
    }

    pub fn calculation(&self) -> &'static str {
        self.metadata().calculation
    }

    pub fn aggregation(&self) -> &'static str {
        self.metadata().aggregation
    }

    pub fn metadata(&self) -> &'static LossMetadata {
        self.definition().meta()
    }

    fn definition(self) -> &'static dyn LossDimension {
        match self {
            Self::Duplication => &DuplicationDimension,
            Self::CodeEconomy => &CodeEconomyDimension,
            Self::StateCardinality => &StateCardinalityDimension,
            Self::Bloat => &BloatDimension,
            Self::CouplingDensity => &CouplingDensityDimension,
        }
    }
}

/// Shared context for heatmap emission, bundling the per-dimension scoring
/// state and policy references that every heatmap emitter needs.
pub struct HeatmapContext<'a> {
    pub(crate) dim: &'a ComplianceDimension,
    pub(crate) dim_index: usize,
    pub(crate) dim_scores: &'a [f64],
    pub(crate) composite_score: f64,
    pub(crate) policy: &'a CompliancePolicy,
    pub(crate) normalization_context: &'a NormalizationContext,
}

/// Behavior contract for a registered loss dimension.
pub trait LossDimension: Sync {
    fn meta(&self) -> &'static LossMetadata;
    fn compute(
        &self,
        report: &AnalysisReport,
        policy: &CompliancePolicy,
        normalization_context: &NormalizationContext,
        semantic: Option<&crate::semantic::SemanticOverlay>,
    ) -> ComplianceDimension;
    fn emit_heatmap(
        &self,
        report: &AnalysisReport,
        ctx: &HeatmapContext<'_>,
        semantic: Option<&crate::semantic::SemanticOverlay>,
        entries: &mut Vec<HeatmapEntry>,
    );
}

struct DuplicationDimension;
struct CodeEconomyDimension;
struct StateCardinalityDimension;
struct BloatDimension;
struct CouplingDensityDimension;

const DUPLICATION_METADATA: LossMetadata = LossMetadata {
    name: "duplication",
    description: "Structural duplication across functions. Duplicate code means duplicate bugs and duplicate maintenance burden.",
    calculation: "Fingerprints production (non-test) function bodies into shape tokens (stripping identifiers/literals). Functions with shape length >= 5 are compared. duplication_score = fraction of production functions in a duplicate group.",
    aggregation: "Global. 0 duplicates -> score 1.0. All duplicated -> score 0.0.",
    scoring: ScoringInfo {
        formula: "duplication_score",
        notes: "0 duplication -> loss 0.0",
    },
    composite: true,
};

const CODE_ECONOMY_METADATA: LossMetadata = LossMetadata {
    name: "code_economy",
    description: "Function overhead ratio: non-test functions / public functions. A high ratio means excessive internal machinery relative to API surface.",
    calculation: "overhead_ratio = adjusted_production / adjusted_public, where control-flow macros count as production functions and #[macro_export] macros count as public API surface. Default normalization uses the configured threshold and default calibration uses score = min(1, threshold/ratio). Alternative normalization/calibration policies can change that shaping.",
    aggregation: "Global. No adjusted public API surface -> vacuously 1.0.",
    scoring: ScoringInfo {
        formula: "default loss = 1 - min(1, threshold/overhead_ratio)",
        notes: "default threshold = max_function_overhead_ratio; policy may override normalization/calibration",
    },
    composite: true,
};

const STATE_CARDINALITY_METADATA: LossMetadata = LossMetadata {
    name: "state_cardinality",
    description: "State space size (log2) for types and functions. Types with many boolean/optional fields and functions with many mutable boolean/option/result bindings create exponential state spaces.",
    calculation: "Recursive type analysis: bool -> 2, Option<T> -> 1+T, Result<T,E> -> T+E, scalars/named -> 1. Structs multiply field cardinalities, enums use the worst intra-variant product cardinality. Default normalization uses K = max_state_cardinality_log2 and default calibration yields per-type/fn score = min(1, (K+1)/(1+log2_card)). Alternative normalization/calibration policies can change that shaping.",
    aggregation: "Default: geometric mean across types and functions with mutable state. Aggregation policy is configurable.",
    scoring: ScoringInfo {
        formula: "default loss = 1 - geomean(min(1, (K+1) / (1 + log2_card))) per type and fn",
        notes: "default K = max_state_cardinality_log2; policy may override normalization/calibration",
    },
    composite: true,
};

const BLOAT_METADATA: LossMetadata = LossMetadata {
    name: "bloat",
    description: "Function line-count bloat: production functions whose line count exceeds the threshold carry too much logic per function. All non-test production functions participate.",
    calculation: "All non-test production functions participate. Default normalization uses K = max_function_lines and default calibration yields per-function score = min(1, K / lines). Alternative normalization/calibration policies can change that shaping.",
    aggregation: "Default: geometric mean across production functions. Aggregation policy is configurable.",
    scoring: ScoringInfo {
        formula: "default loss = 1 - geomean(min(1, K / lines)) per fn",
        notes: "default K = max_function_lines (35); policy may override normalization/calibration",
    },
    composite: true,
};

const COUPLING_DENSITY_METADATA: LossMetadata = LossMetadata {
    name: "coupling_density",
    description: "Per-module outgoing cross-module edge count. Modules with many outgoing dependencies carry coupling loss.",
    calculation: "Per-module outgoing edge count, normalized and calibrated through the standard pipeline. Default normalization uses K = max_coupling_edges and default calibration yields per-module score = min(1, K / edges). Alternative normalization/calibration policies can change that shaping.",
    aggregation: "Default: geometric mean across modules (same as bloat/state_cardinality). Fallback: score = 1 - density when only SemanticSummary is available (no overlay).",
    scoring: ScoringInfo {
        formula: "default loss = 1 - geomean(min(1, K / edges)) per module",
        notes: "Requires SemanticOverlay for per-module pipeline; falls back to 1 - density with SemanticSummary only. Defaults to 1.0 (no penalty) when no semantic data is available.",
    },
    composite: true,
};

impl LossDimension for DuplicationDimension {
    fn meta(&self) -> &'static LossMetadata {
        &DUPLICATION_METADATA
    }

    fn compute(
        &self,
        report: &AnalysisReport,
        _policy: &CompliancePolicy,
        _normalization_context: &NormalizationContext,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
    ) -> ComplianceDimension {
        crate::loss::duplication::duplication_compliance(report)
    }

    fn emit_heatmap(
        &self,
        _report: &AnalysisReport,
        ctx: &HeatmapContext<'_>,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
        entries: &mut Vec<HeatmapEntry>,
    ) {
        crate::loss::duplication::emit_duplication_heatmap(ctx, entries);
    }
}

impl LossDimension for CodeEconomyDimension {
    fn meta(&self) -> &'static LossMetadata {
        &CODE_ECONOMY_METADATA
    }

    fn compute(
        &self,
        report: &AnalysisReport,
        policy: &CompliancePolicy,
        normalization_context: &NormalizationContext,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
    ) -> ComplianceDimension {
        crate::loss::code_economy::code_economy_compliance(report, policy, normalization_context)
    }

    fn emit_heatmap(
        &self,
        report: &AnalysisReport,
        ctx: &HeatmapContext<'_>,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
        entries: &mut Vec<HeatmapEntry>,
    ) {
        crate::loss::code_economy::emit_code_economy_heatmap(report, ctx, entries);
    }
}

impl LossDimension for StateCardinalityDimension {
    fn meta(&self) -> &'static LossMetadata {
        &STATE_CARDINALITY_METADATA
    }

    fn compute(
        &self,
        report: &AnalysisReport,
        policy: &CompliancePolicy,
        normalization_context: &NormalizationContext,
        semantic: Option<&crate::semantic::SemanticOverlay>,
    ) -> ComplianceDimension {
        crate::loss::state_cardinality::state_cardinality_compliance(
            &report.types,
            &report.functions,
            policy,
            normalization_context,
            semantic,
        )
    }

    fn emit_heatmap(
        &self,
        _report: &AnalysisReport,
        ctx: &HeatmapContext<'_>,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
        entries: &mut Vec<HeatmapEntry>,
    ) {
        crate::loss::state_cardinality::emit_state_cardinality_heatmap(ctx, entries);
    }
}

impl LossDimension for BloatDimension {
    fn meta(&self) -> &'static LossMetadata {
        &BLOAT_METADATA
    }

    fn compute(
        &self,
        report: &AnalysisReport,
        policy: &CompliancePolicy,
        normalization_context: &NormalizationContext,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
    ) -> ComplianceDimension {
        crate::loss::bloat::bloat_compliance(&report.functions, policy, normalization_context)
    }

    fn emit_heatmap(
        &self,
        _report: &AnalysisReport,
        ctx: &HeatmapContext<'_>,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
        entries: &mut Vec<HeatmapEntry>,
    ) {
        crate::loss::bloat::emit_bloat_heatmap(ctx, entries);
    }
}

impl LossDimension for CouplingDensityDimension {
    fn meta(&self) -> &'static LossMetadata {
        &COUPLING_DENSITY_METADATA
    }

    fn compute(
        &self,
        report: &AnalysisReport,
        policy: &CompliancePolicy,
        normalization_context: &NormalizationContext,
        semantic: Option<&crate::semantic::SemanticOverlay>,
    ) -> ComplianceDimension {
        crate::loss::coupling_density::coupling_density_compliance(
            semantic,
            report.semantic.as_ref(),
            policy,
            normalization_context,
        )
    }

    fn emit_heatmap(
        &self,
        _report: &AnalysisReport,
        ctx: &HeatmapContext<'_>,
        _semantic: Option<&crate::semantic::SemanticOverlay>,
        entries: &mut Vec<HeatmapEntry>,
    ) {
        crate::loss::coupling_density::emit_coupling_density_heatmap(ctx, entries);
    }
}

// ---------------------------------------------------------------------------
// Heatmap entry
// ---------------------------------------------------------------------------

/// A single entry in the loss heatmap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapEntry {
    pub file: String,
    pub line: usize,
    pub function_name: String,
    pub dimension: String,
    pub responsibility: f64,
    /// E.g., "85 lines" or "cc 12".
    pub detail: String,
    /// Full scope context for hierarchical rollup (modules, types, functions).
    #[serde(default)]
    pub scope_path: Vec<crate::metrics::ScopeSegment>,
}

/// High-level kind of artifact represented in a dimension pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompliancePipelineSubjectKind {
    Codebase,
    Type,
    Function,
    Module,
}

/// One cohort resolution used while scoring a dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompliancePipelineCohort {
    pub cohort: NormalizationCohort,
    pub reference_scale: f64,
    pub normalizer: Normalizer,
    pub calibrator: crate::Calibrator,
}

/// One scored observation in a dimension pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompliancePipelineObservation {
    pub kind: CompliancePipelineSubjectKind,
    pub file: String,
    pub line: usize,
    pub name: String,
    pub detail: String,
    pub cohort: Option<NormalizationCohort>,
    /// Hierarchical aggregation size hint.
    ///
    /// Units are dimension-specific. `state_cardinality` currently uses field
    /// count for types and line count for functions, which is heuristic rather
    /// than a claim that those are perfectly commensurable.
    pub size_hint: f64,
    pub raw: f64,
    pub normalized: Option<f64>,
    pub calibrated_score: f64,
    /// Full scope context for hierarchical rollup (modules, types, functions).
    #[serde(default)]
    pub scope_path: Vec<crate::metrics::ScopeSegment>,
}

/// Canonical trace of how a soft dimension was produced.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComplianceDimensionPipeline {
    pub cohorts: Vec<CompliancePipelineCohort>,
    pub artifact_aggregation: Option<ArtifactAggregation>,
    pub aggregated_score: f64,
    pub observations: Vec<CompliancePipelineObservation>,
}

// ---------------------------------------------------------------------------
// Compliance dimension and report
// ---------------------------------------------------------------------------

#[allow(clippy::trivially_copy_pass_by_ref)] // serde serialize_with requires &T
fn serialize_as_loss<S: serde::Serializer>(score: &f64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(1.0 - score)
}

fn deserialize_from_loss<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let loss = f64::deserialize(d)?;
    Ok(1.0 - loss)
}

/// Individual compliance dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceDimension {
    /// Name of this compliance rule.
    pub name: String,
    /// Continuous score in [0.0, 1.0]. Serialized as loss (1 - score).
    #[serde(
        rename = "loss",
        serialize_with = "serialize_as_loss",
        deserialize_with = "deserialize_from_loss"
    )]
    pub score: f64,
    /// Number of items scored (0 for global dimensions, N for per-item dimensions).
    pub item_count: usize,
    /// Brief description of the rule.
    pub rule: String,
    /// Canonical scoring trace for `raw -> normalized -> calibrated -> aggregated`.
    #[serde(default)]
    pub pipeline: ComplianceDimensionPipeline,
}

/// Full compliance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    /// Continuous soft dimension scores.
    pub soft_dimensions: Vec<ComplianceDimension>,
    /// Scalarized soft dimension score. Serialized as loss (1 - score).
    #[serde(
        rename = "composite_loss",
        serialize_with = "serialize_as_loss",
        deserialize_with = "deserialize_from_loss"
    )]
    pub composite_score: f64,
    /// Loss heatmap entries (initially empty, filled by heatmap phase).
    pub heatmap: Vec<HeatmapEntry>,
    /// The scoring policy used.
    pub policy: CompliancePolicy,
}

/// Error returned when compliance deltas are computed from incompatible reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComplianceDeltaError {
    MissingBeforeDimension(String),
    MissingAfterDimension(String),
}

impl fmt::Display for ComplianceDeltaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBeforeDimension(name) => {
                write!(f, "missing dimension '{name}' in before compliance report")
            }
            Self::MissingAfterDimension(name) => {
                write!(f, "missing dimension '{name}' in after compliance report")
            }
        }
    }
}

impl std::error::Error for ComplianceDeltaError {}

// ---------------------------------------------------------------------------
// Main compliance computation
// ---------------------------------------------------------------------------

/// Compute compliance scores from an analysis report and scoring policy.
///
/// Soft dimensions produce continuous [0, 1] scores. The composite score uses
/// the configured objective scalarization policy. Normalization defaults to
/// report-local cohort statistics.
pub fn compute_compliance(report: &AnalysisReport, policy: &CompliancePolicy) -> ComplianceReport {
    compute_compliance_full(report, policy, None)
}

/// Compute compliance with an explicit semantic overlay.
pub fn compute_compliance_with_semantic(
    report: &AnalysisReport,
    policy: &CompliancePolicy,
    semantic: Option<&crate::semantic::SemanticOverlay>,
) -> ComplianceReport {
    compute_compliance_full(report, policy, semantic)
}

fn compute_compliance_full(
    report: &AnalysisReport,
    policy: &CompliancePolicy,
    semantic: Option<&crate::semantic::SemanticOverlay>,
) -> ComplianceReport {
    let normalization_context = NormalizationContext::from_report(report);
    compute_compliance_with_context(report, policy, &normalization_context, semantic)
}

/// Compute compliance scores from an analysis report, scoring policy, and
/// explicit normalization context.
///
/// This is the entry point for corpus-relative or online normalization: build a
/// shared `NormalizationContext` across many reports, then score each report
/// against that context.
pub fn compute_compliance_with_context(
    report: &AnalysisReport,
    policy: &CompliancePolicy,
    normalization_context: &NormalizationContext,
    semantic: Option<&crate::semantic::SemanticOverlay>,
) -> ComplianceReport {
    let soft_dimensions: Vec<ComplianceDimension> = LossFunction::all()
        .iter()
        .map(|dimension| {
            dimension
                .definition()
                .compute(report, policy, normalization_context, semantic)
        })
        .collect();

    let composite_scores: Vec<f64> = LossFunction::all()
        .iter()
        .zip(soft_dimensions.iter())
        .filter(|(lf, _)| lf.is_composite())
        .map(|(_, d)| d.score)
        .collect();
    let composite_score = scalarize_dimension_scores(
        policy.aggregation.objective_scalarization,
        &composite_scores,
    );

    let heatmap = compute_heatmap(
        report,
        &soft_dimensions,
        composite_score,
        policy,
        normalization_context,
        semantic,
    );

    ComplianceReport {
        soft_dimensions,
        composite_score,
        heatmap,
        policy: policy.clone(),
    }
}

/// Convert a compliance report into a loss vector.
///
/// Soft dimensions become Number entries with value = 1.0 - score (lower = better).
pub fn compliance_to_loss_vector(report: &ComplianceReport) -> LossVectorOut {
    let mut entries: Vec<LossEntry> = Vec::new();

    // Soft dimension entries.
    for d in &report.soft_dimensions {
        let loss = 1.0 - d.score;
        let notes = if d.item_count > 0 {
            format!("{} items, loss: {loss:.4}", d.item_count)
        } else {
            format!("loss: {loss:.4}")
        };
        entries.push(LossEntry {
            name: format!("compliance_{}", d.name),
            value: LossValueOut::Number(loss),
            notes: Some(notes),
        });
    }

    // Composite entry.
    let composite_loss = 1.0 - report.composite_score;
    entries.push(LossEntry {
        name: "compliance_composite".into(),
        value: LossValueOut::Number(composite_loss),
        notes: Some(format!("composite loss: {composite_loss:.4}")),
    });

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    LossVectorOut { entries }
}

/// Convert two compliance reports into a delta loss vector.
///
/// Values are `after_loss - before_loss`, so positive deltas are regressions
/// and negative deltas are improvements.
pub fn compliance_delta_to_loss_vector(
    before: &ComplianceReport,
    after: &ComplianceReport,
) -> Result<LossVectorOut, ComplianceDeltaError> {
    let mut entries: Vec<LossEntry> = LossFunction::all()
        .iter()
        .map(|dimension| -> Result<LossEntry, ComplianceDeltaError> {
            let before_dim = before
                .soft_dimensions
                .iter()
                .find(|d| d.name == dimension.name())
                .ok_or_else(|| {
                    ComplianceDeltaError::MissingBeforeDimension(dimension.name().to_string())
                })?;
            let after_dim = after
                .soft_dimensions
                .iter()
                .find(|d| d.name == dimension.name())
                .ok_or_else(|| {
                    ComplianceDeltaError::MissingAfterDimension(dimension.name().to_string())
                })?;
            let before_loss = 1.0 - before_dim.score;
            let after_loss = 1.0 - after_dim.score;

            Ok(LossEntry {
                name: format!("compliance_{}", dimension.name()),
                value: LossValueOut::Number(after_loss - before_loss),
                notes: Some(format!("loss: {before_loss:.4} -> {after_loss:.4}")),
            })
        })
        .collect::<Result<_, _>>()?;

    let before_composite_loss = 1.0 - before.composite_score;
    let after_composite_loss = 1.0 - after.composite_score;
    entries.push(LossEntry {
        name: "compliance_composite".into(),
        value: LossValueOut::Number(after_composite_loss - before_composite_loss),
        notes: Some(format!(
            "loss: {before_composite_loss:.4} -> {after_composite_loss:.4}"
        )),
    });

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(LossVectorOut { entries })
}

// ---------------------------------------------------------------------------
// Heatmap computation
// ---------------------------------------------------------------------------

/// Build the loss heatmap: per-function attribution of compliance loss.
///
/// Each entry identifies a specific function (or codebase-level dimension) and
/// its `responsibility` — the fraction of total composite loss attributable to
/// fixing that single item.
fn compute_heatmap(
    report: &AnalysisReport,
    soft_dimensions: &[ComplianceDimension],
    composite_score: f64,
    policy: &CompliancePolicy,
    normalization_context: &NormalizationContext,
    semantic: Option<&crate::semantic::SemanticOverlay>,
) -> Vec<HeatmapEntry> {
    // Counterfactual computation uses only composite dimensions.
    let composite_dim_scores: Vec<f64> = LossFunction::all()
        .iter()
        .zip(soft_dimensions.iter())
        .filter(|(lf, _)| lf.is_composite())
        .map(|(_, d)| d.score)
        .collect();
    let mut entries: Vec<HeatmapEntry> = Vec::new();

    let mut composite_dim_index = 0usize;
    for (kind, dim) in LossFunction::all().iter().zip(soft_dimensions.iter()) {
        if !kind.is_composite() {
            continue;
        }
        let ctx = HeatmapContext {
            dim,
            dim_index: composite_dim_index,
            dim_scores: &composite_dim_scores,
            composite_score,
            policy,
            normalization_context,
        };
        kind.definition()
            .emit_heatmap(report, &ctx, semantic, &mut entries);
        composite_dim_index += 1;
    }
    sort_heatmap(&mut entries);

    entries
}

/// Sort heatmap by responsibility descending, then by dimension/file/line/name
/// for determinism.
fn sort_heatmap(entries: &mut [HeatmapEntry]) {
    entries.sort_by(|a, b| {
        b.responsibility
            .partial_cmp(&a.responsibility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.dimension.cmp(&b.dimension))
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
            .then(a.function_name.cmp(&b.function_name))
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::aggregate_artifact_scores;
    use crate::duplication::DuplicationReport;
    use crate::loss::common::counterfactual_composite;
    use crate::metrics::{AnalysisReport, EntropyMetrics, FunctionMetrics, Summary, TypeMetrics};

    /// Build an empty analysis report with no functions or types.
    fn empty_report() -> AnalysisReport {
        AnalysisReport {
            analysis_root: None,
            files_analyzed: 0,
            total_lines: 0,
            functions: Vec::new(),
            types: Vec::new(),
            entropy: empty_entropy(),
            duplication: empty_duplication(),
            summary: Summary::default(),
            semantic: None,
        }
    }

    fn empty_entropy() -> EntropyMetrics {
        EntropyMetrics {
            total_tokens: 0,
            vocabulary_size: 0,
            entropy_bits: 0.0,
            normalized_entropy: 0.0,
            per_file: Vec::new(),
        }
    }

    fn empty_duplication() -> DuplicationReport {
        DuplicationReport {
            functions_fingerprinted: 0,
            exact_duplicates: Vec::new(),
            near_duplicates: Vec::new(),
            duplication_score: 0.0,
        }
    }

    /// Build a compliant function: short, simple, well-asserted.
    /// lines = 2, well below K = 35, bloat score = 1.0.
    fn compliant_function(name: &str) -> FunctionMetrics {
        FunctionMetrics {
            name: name.to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 2,
            params: 2,
            nesting_depth: 1,
            cyclomatic: 2,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 3,
            meaningful_assertions: 3,
            is_test: false,
            is_pub: true,
        }
    }

    /// Build a violating function: long, complex, no assertions, many params.
    fn violating_function(name: &str) -> FunctionMetrics {
        FunctionMetrics {
            name: name.to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 200,
            params: 10,
            nesting_depth: 8,
            cyclomatic: 20,
            mutable_bindings: 10,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: false,
        }
    }

    /// Build a type with zero badness (log2=0, cardinality=1).
    fn compliant_type(name: &str) -> TypeMetrics {
        TypeMetrics {
            name: name.to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 0,
            option_fields: 0,
            total_fields: 2,
            state_cardinality: 1,
            state_cardinality_log2: 0.0,
        }
    }

    /// Build a type that violates the state cardinality threshold.
    fn violating_type(name: &str) -> TypeMetrics {
        TypeMetrics {
            name: name.to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 8,
            option_fields: 4,
            total_fields: 12,
            state_cardinality: 4096,
            state_cardinality_log2: 12.0,
        }
    }

    /// Build a test function with assertions (simulates a well-tested codebase).
    fn test_function(name: &str, assertions: usize) -> FunctionMetrics {
        FunctionMetrics {
            name: name.to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 10,
            params: 0,
            nesting_depth: 1,
            cyclomatic: 1,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions,
            meaningful_assertions: assertions,
            is_test: true,
            is_pub: false,
        }
    }

    /// Helper: find a soft dimension by name.
    fn find_soft_dim<'a>(report: &'a ComplianceReport, name: &str) -> &'a ComplianceDimension {
        report
            .soft_dimensions
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("soft dimension '{name}' not found"))
    }

    #[test]
    fn test_perfect_compliance() {
        let mut report = empty_report();
        // 3 production functions (lines=2 each).
        // 1 test function with 6 assertions => test_density = 6/6 = 1.0.
        report.functions = vec![
            compliant_function("a"),
            compliant_function("b"),
            compliant_function("c"),
            test_function("test_a", 6),
        ];
        report.types = vec![compliant_type("Foo"), compliant_type("Bar")];
        report.summary = Summary {
            test_function_count: 1,
            production_function_count: 3,
            public_function_count: 3,
            function_overhead_ratio: 1.0,
            test_density: 1.0,
            total_production_cyclomatic: 6,
            production_lines: 6,
            ..Summary::default()
        };

        let result = compute_compliance(&report, &CompliancePolicy::default());

        // Directional mode: even small functions get slight pressure.
        // lines=2 => bloat raw=log2(2)=1.0, normalized=1.0/log2(45)~0.182,
        // score=exp(-ln2*0.182^3)~0.996. Very close to 1.0 but not exact.
        for dim in &result.soft_dimensions {
            assert!(
                dim.score > 0.99,
                "soft dimension {} should be near 1.0, got {}",
                dim.name,
                dim.score,
            );
        }
        // Composite is close to 1.0 for nearly-perfect code.
        assert!(
            result.composite_score > 0.99,
            "composite should be near 1.0, got {}",
            result.composite_score,
        );
    }

    #[test]
    fn test_zero_compliance() {
        let mut report = empty_report();
        report.functions = vec![violating_function("a"), violating_function("b")];
        report.types = vec![violating_type("Bad")];
        report.duplication.duplication_score = 1.0;
        report.duplication.functions_fingerprinted = 2;
        // Production functions exist but no tests => test_density = 0.
        report.summary.production_function_count = 2;
        report.summary.test_density = 0.0;
        // Extremely high overhead ratio => code_economy score approaches 0.
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = f64::INFINITY;

        let result = compute_compliance(&report, &CompliancePolicy::default());

        // duplication: 1 - 1.0 = 0.0
        let duplication = find_soft_dim(&result, "duplication");
        assert!(
            duplication.score < f64::EPSILON,
            "duplication should be 0.0, got {}",
            duplication.score,
        );

        // code_economy: log2(inf) => inf, score approaches 0.0
        let economy = find_soft_dim(&result, "code_economy");
        assert!(
            economy.score < f64::EPSILON,
            "code_economy should be ~0.0, got {}",
            economy.score,
        );

        // state_cardinality: log2=12, raw=12.0, normalized=12/4=3.0,
        // score = exp(-ln2*3^2) = exp(-ln2*9)
        let state = find_soft_dim(&result, "state_cardinality");
        let expected_state = (-std::f64::consts::LN_2 * 9.0).exp();
        assert!(
            (state.score - expected_state).abs() < 1e-10,
            "state_cardinality should be {expected_state}, got {}",
            state.score,
        );

        // composite: 0.0 (duplication and code_economy are 0)
        assert!(
            result.composite_score < f64::EPSILON,
            "composite should be 0.0, got {}",
            result.composite_score,
        );
    }

    #[test]
    fn test_mixed_compliance() {
        let mut report = empty_report();
        report.functions = vec![compliant_function("good"), violating_function("bad")];
        report.types = vec![
            TypeMetrics {
                name: "Ok".to_string(),
                file: "test.rs".to_string(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 1,
                kind: crate::metrics::TypeKind::Struct,
                bool_fields: 1,
                option_fields: 1,
                total_fields: 4,
                state_cardinality: 4,
                state_cardinality_log2: 2.0,
            },
            violating_type("NotOk"),
        ];

        let result = compute_compliance(&report, &CompliancePolicy::default());

        // state_cardinality: type log2=2.0, raw=2.0, norm=2/4=0.5, score=exp(-ln2*0.25)
        //                    type log2=12.0, raw=12.0, norm=3.0, score=exp(-ln2*9)
        //                    geomean = sqrt(score_2 * score_12)
        let state = find_soft_dim(&result, "state_cardinality");
        let score_2 = (-std::f64::consts::LN_2 * 0.25_f64).exp();
        let score_12 = (-std::f64::consts::LN_2 * 9.0_f64).exp();
        let expected = (score_2 * score_12).sqrt();
        assert!(
            (state.score - expected).abs() < 1e-6,
            "state_cardinality should be ~{expected:.4}, got {}",
            state.score,
        );

        // Composite should be between 0 and 1 exclusive.
        assert!(
            result.composite_score > 0.0 && result.composite_score < 1.0,
            "composite should be in (0, 1), got {}",
            result.composite_score,
        );
    }

    #[test]
    fn test_empty_codebase() {
        let report = empty_report();
        let result = compute_compliance(&report, &CompliancePolicy::default());

        // Vacuously true: all soft dimension scores should be 1.0.
        for dim in &result.soft_dimensions {
            assert!(
                (dim.score - 1.0).abs() < f64::EPSILON,
                "empty codebase: soft dimension {} should be 1.0, got {}",
                dim.name,
                dim.score,
            );
        }
        assert!(
            (result.composite_score - 1.0).abs() < f64::EPSILON,
            "empty codebase composite should be 1.0, got {}",
            result.composite_score,
        );
    }

    #[test]
    fn test_geometric_mean_penalizes_outliers() {
        // One bad dimension (duplication) severely drags the composite below
        // the arithmetic mean. duplication_score=1.0 gives score 1-1.0=0.0.
        let mut report = empty_report();
        report.functions = vec![compliant_function("ok")];
        report.types = vec![compliant_type("Good")];
        report.summary = Summary {
            production_function_count: 1,
            public_function_count: 1,
            function_overhead_ratio: 1.0,
            ..Summary::default()
        };
        report.duplication.duplication_score = 0.8;
        report.duplication.functions_fingerprinted = 1;

        let result = compute_compliance(&report, &CompliancePolicy::default());

        // duplication: 1 - 0.8 = 0.2
        let dup = find_soft_dim(&result, "duplication");
        assert!(
            (dup.score - 0.2).abs() < f64::EPSILON,
            "duplication should be 0.2, got {}",
            dup.score,
        );

        // Verify that the composite (geometric mean) is much lower than arithmetic mean.
        let scores: Vec<f64> = result.soft_dimensions.iter().map(|d| d.score).collect();
        let arith_mean: f64 = scores.iter().sum::<f64>() / scores.len() as f64;
        assert!(
            result.composite_score < arith_mean,
            "geometric mean ({}) should be less than arithmetic mean ({})",
            result.composite_score,
            arith_mean,
        );

        // The composite should be positive (no dimension is exactly zero here).
        assert!(
            result.composite_score > 0.0,
            "composite should be positive, got {}",
            result.composite_score,
        );
    }

    #[test]
    fn test_compliance_to_loss_vector() {
        let mut report = empty_report();
        report.functions = vec![compliant_function("a")];
        report.types = vec![compliant_type("T")];

        let compliance = compute_compliance(&report, &CompliancePolicy::default());
        let loss = compliance_to_loss_vector(&compliance);

        // 5 soft dimensions + 1 composite = 6 entries.
        assert_eq!(
            loss.entries.len(),
            6,
            "expected 6 loss entries, got {}; names: {:?}",
            loss.entries.len(),
            loss.entries.iter().map(|e| &e.name).collect::<Vec<_>>(),
        );

        // Entries must be sorted by name.
        let names: Vec<&str> = loss.entries.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "loss vector entries must be sorted by name");

        // All soft dimension losses should be very small for near-compliant code.
        // Directional mode applies slight pressure even to short functions.
        for entry in &loss.entries {
            if let LossValueOut::Number(v) = entry.value {
                assert!(
                    v < 0.01,
                    "entry {} should have near-zero loss, got {}",
                    entry.name,
                    v,
                );
            }
        }
    }

    #[test]
    fn test_compliance_delta_to_loss_vector() {
        let mut before_report = empty_report();
        before_report.functions = vec![compliant_function("a")];
        before_report.types = vec![compliant_type("T")];
        before_report.summary = Summary {
            production_function_count: 1,
            public_function_count: 1,
            function_overhead_ratio: 1.0,
            ..Summary::default()
        };

        let mut after_report = before_report.clone();
        after_report.functions = vec![violating_function("a")];
        after_report.summary.production_function_count = 10;
        after_report.summary.public_function_count = 1;
        after_report.summary.function_overhead_ratio = 10.0;

        let policy = CompliancePolicy::default();
        let before = compute_compliance(&before_report, &policy);
        let after = compute_compliance(&after_report, &policy);
        let delta = compliance_delta_to_loss_vector(&before, &after)
            .unwrap_or_else(|err| panic!("delta conversion should succeed: {err}"));

        let composite = delta
            .entries
            .iter()
            .find(|entry| entry.name == "compliance_composite")
            .unwrap_or_else(|| panic!("missing compliance_composite"));
        let LossValueOut::Number(composite_delta) = composite.value else {
            panic!("compliance_composite should be numeric");
        };
        assert!(
            composite_delta > 0.0,
            "expected regression to increase composite loss, got {composite_delta}",
        );
    }

    #[test]
    fn test_loss_function_registry_is_consistent() {
        let names: Vec<&str> = LossFunction::all().iter().map(LossFunction::name).collect();
        let mut sorted_names = names.clone();
        sorted_names.sort();
        sorted_names.dedup();

        assert_eq!(
            names.len(),
            sorted_names.len(),
            "loss function names must be unique",
        );

        let report = compute_compliance(&empty_report(), &CompliancePolicy::default());
        assert_eq!(
            report.soft_dimensions.len(),
            LossFunction::all().len(),
            "registry and compliance output should stay aligned",
        );
    }

    #[test]
    fn test_compliance_delta_to_loss_vector_reports_missing_dimension() {
        let report = compute_compliance(&empty_report(), &CompliancePolicy::default());
        let mut missing_after = report.clone();
        missing_after
            .soft_dimensions
            .retain(|dimension| dimension.name != "bloat");

        let Err(err) = compliance_delta_to_loss_vector(&report, &missing_after) else {
            panic!("missing dimension should return an error");
        };

        assert_eq!(
            err,
            ComplianceDeltaError::MissingAfterDimension("bloat".to_string())
        );
    }

    #[test]
    fn test_code_economy_good_ratio() {
        // 4 public + 4 private = 8 non-test functions, ratio = 8/4 = 2.0.
        // raw = log2(2) = 1.0, normalized = 1.0/log2(5), score = exp(-ln2*norm)
        let mut report = empty_report();
        report.functions = vec![
            compliant_function("pub_a"),
            compliant_function("pub_b"),
            compliant_function("pub_c"),
            compliant_function("pub_d"),
        ];
        report.summary.public_function_count = 4;
        report.summary.production_function_count = 8;
        report.summary.function_overhead_ratio = 2.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let economy = find_soft_dim(&result, "code_economy");
        let expected = (-std::f64::consts::LN_2 * 1.0 / 5.0_f64.log2()).exp();
        assert!(
            (economy.score - expected).abs() < 1e-6,
            "expected score ~{expected:.6}, got {}",
            economy.score,
        );
    }

    #[test]
    fn test_code_economy_bad_ratio() {
        // 2 public + 14 private = 16 non-test functions, ratio = 16/2 = 8.0.
        // raw = log2(8) = 3.0, normalized = 3.0/log2(5), score = exp(-ln2*norm)
        let mut report = empty_report();
        report.functions = vec![compliant_function("pub_a"), compliant_function("pub_b")];
        report.summary.public_function_count = 2;
        report.summary.production_function_count = 16;
        report.summary.function_overhead_ratio = 8.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let economy = find_soft_dim(&result, "code_economy");
        let expected = (-std::f64::consts::LN_2 * 3.0 / 5.0_f64.log2()).exp();
        assert!(
            (economy.score - expected).abs() < 1e-6,
            "expected score ~{expected:.6}, got {}",
            economy.score,
        );
    }

    #[test]
    fn test_code_economy_no_public_functions() {
        // No public functions => vacuously compliant (score 1.0).
        // No functions in report.functions => no per-function observations.
        let mut report = empty_report();
        report.summary.public_function_count = 0;
        report.summary.production_function_count = 5;
        report.summary.function_overhead_ratio = 0.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let economy = find_soft_dim(&result, "code_economy");
        assert!(
            (economy.score - 1.0).abs() < f64::EPSILON,
            "expected score 1.0 (vacuous), got {}",
            economy.score,
        );
        // Per-function observations: no overhead fns in report.functions.
        assert_eq!(economy.pipeline.observations.len(), 0);
    }

    #[test]
    fn test_code_economy_macro_exports_count_as_public_surface() {
        let mut report = empty_report();
        report.summary.public_function_count = 0;
        report.summary.production_function_count = 0;
        report.summary.macro_fn_count = 10;
        report.summary.macro_export_fn_count = 1;
        report.summary.function_overhead_ratio = 10.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let economy = find_soft_dim(&result, "code_economy");
        // raw = log2(10), normalized = log2(10)/log2(5), score = exp(-ln2*norm)
        let expected = (-std::f64::consts::LN_2 * 10.0_f64.log2() / 5.0_f64.log2()).exp();
        assert!(
            (economy.score - expected).abs() < 1e-6,
            "expected macro-export-backed score ~{expected:.6}, got {}",
            economy.score,
        );
    }

    #[test]
    fn test_bloat_compliance() {
        // Single production function: lines=70.
        // raw = log2(70), normalized = log2(70)/log2(45),
        // score = exp(-ln2 * normalized^3) (StretchedExponentialDecay shape=3)
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "moderate".to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 70,
            params: 1,
            nesting_depth: 2,
            cyclomatic: 10,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let sd = find_soft_dim(&result, "bloat");
        let normalized = 70.0_f64.log2() / 45.0_f64.log2();
        let expected = (-std::f64::consts::LN_2 * normalized.powi(3)).exp();
        assert!(
            (sd.score - expected).abs() < 1e-10,
            "expected bloat score ~{expected:.6}, got {}",
            sd.score,
        );
    }

    #[test]
    fn test_bloat_moderate_function_has_moderate_penalty() {
        // Single production function: lines=30.
        // Directional mode applies continuous pressure.
        // raw = log2(30), normalized = log2(30)/log2(45),
        // score = exp(-ln2 * normalized^3)
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "short".to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 30,
            params: 1,
            nesting_depth: 2,
            cyclomatic: 5,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let sd = find_soft_dim(&result, "bloat");
        let normalized = 30.0_f64.log2() / 45.0_f64.log2();
        let expected = (-std::f64::consts::LN_2 * normalized.powi(3)).exp();
        assert!(
            (sd.score - expected).abs() < 1e-10,
            "expected bloat score ~{expected:.6}, got {}",
            sd.score,
        );
        // 30 lines is below the half-life scale (45), so score should be > 0.5
        assert!(sd.score > 0.5, "30-line function score should be > 0.5");
    }

    #[test]
    fn test_directional_mode_applies_bloat_pressure_below_compliance_threshold() {
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "small_fn".to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 16,
            params: 1,
            nesting_depth: 2,
            cyclomatic: 2,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());

        let bloat = find_soft_dim(&result, "bloat");
        assert!(
            bloat.score < 1.0,
            "directional scoring should apply pressure to a 16-line function",
        );
        assert!(
            result
                .heatmap
                .iter()
                .any(|entry| entry.dimension == "bloat" && entry.function_name == "small_fn"),
            "directional scoring should expose attribution for sub-threshold pressure",
        );
    }

    #[test]
    fn test_bloat_high() {
        // Single production function: lines=100.
        // raw = log2(100), normalized = log2(100)/log2(45),
        // score = exp(-ln2 * normalized^3)
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "bloated".to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 100,
            params: 1,
            nesting_depth: 1,
            cyclomatic: 2,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let sd = find_soft_dim(&result, "bloat");
        let normalized = 100.0_f64.log2() / 45.0_f64.log2();
        let expected = (-std::f64::consts::LN_2 * normalized.powi(3)).exp();
        assert!(
            (sd.score - expected).abs() < 1e-10,
            "expected bloat score ~{expected:.6}, got {}",
            sd.score,
        );
    }

    #[test]
    fn test_bloat_includes_cc1_functions() {
        // A cc=1 function now participates in bloat scoring using log2(lines).
        // lines=100, raw=log2(100), normalized=log2(100)/log2(45),
        // score = exp(-ln2 * normalized^3)
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "data_only".to_string(),
            file: "test.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 100,
            params: 1,
            nesting_depth: 1,
            cyclomatic: 1,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let sd = find_soft_dim(&result, "bloat");
        let normalized = 100.0_f64.log2() / 45.0_f64.log2();
        let expected = (-std::f64::consts::LN_2 * normalized.powi(3)).exp();
        assert!(
            (sd.score - expected).abs() < 1e-10,
            "cc=1 function with 100 lines should score ~{expected:.6}, got {}",
            sd.score,
        );
    }

    #[test]
    fn test_bloat_empty() {
        let report = empty_report();
        let result = compute_compliance(&report, &CompliancePolicy::default());
        let sd = find_soft_dim(&result, "bloat");
        assert!(
            (sd.score - 1.0).abs() < f64::EPSILON,
            "expected bloat score 1.0 (vacuous), got {}",
            sd.score,
        );
    }

    // -----------------------------------------------------------------------
    // Heatmap tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_heatmap_empty_when_no_code() {
        // No functions, no types, no lines => all vacuous 1.0 => empty heatmap.
        let report = empty_report();
        let result = compute_compliance(&report, &CompliancePolicy::default());

        assert!(
            result.heatmap.is_empty(),
            "empty codebase should have empty heatmap, got {} entries",
            result.heatmap.len(),
        );
    }

    #[test]
    fn test_heatmap_soft_dimension_attribution() {
        // 2 types: 1 compliant (log2=0), 1 with high log2 cardinality.
        let mut report = empty_report();
        report.functions = vec![compliant_function("ok")];
        report.types = vec![compliant_type("Good"), violating_type("Bad")];
        report.summary = Summary {
            production_function_count: 1,
            public_function_count: 1,
            function_overhead_ratio: 1.0,
            ..Summary::default()
        };

        let result = compute_compliance(&report, &CompliancePolicy::default());

        let state_entries: Vec<&HeatmapEntry> = result
            .heatmap
            .iter()
            .filter(|e| e.dimension == "state_cardinality")
            .collect();

        assert_eq!(
            state_entries.len(),
            1,
            "expected 1 state_cardinality heatmap entry, got {}",
            state_entries.len(),
        );
        assert_eq!(state_entries[0].function_name, "Bad");
        assert!(
            state_entries[0].responsibility > 0.0,
            "state_cardinality violator should have positive responsibility",
        );

        // Verify counterfactual math:
        // Bad type: log2=12.0, raw=12, norm=3.0, score=exp(-ln2*9)
        // cf_dim_score = geomean([1.0, 1.0]) with Bad replaced by 1.0 = 1.0
        let state_dim = find_soft_dim(&result, "state_cardinality");
        let s_j = (-std::f64::consts::LN_2 * 9.0).exp();
        let n: f64 = 2.0;
        let cf_dim_score = (state_dim.score / s_j.powf(1.0 / n)).min(1.0);

        let dim_scores: Vec<f64> = result.soft_dimensions.iter().map(|d| d.score).collect();
        let dim_index = result
            .soft_dimensions
            .iter()
            .position(|d| d.name == "state_cardinality")
            .unwrap();
        let cf_composite = counterfactual_composite(
            &dim_scores,
            dim_index,
            cf_dim_score,
            CompliancePolicy::default()
                .aggregation
                .objective_scalarization,
        );
        let expected = cf_composite - result.composite_score;

        assert!(
            (state_entries[0].responsibility - expected).abs() < 1e-10,
            "state_cardinality responsibility {:.6} should match expected {expected:.6}",
            state_entries[0].responsibility,
        );
    }

    #[test]
    fn test_heatmap_sorted_by_responsibility() {
        // Two types with DIFFERENT log2 cardinalities (12.0 vs 6.0).
        let mut report = empty_report();
        report.functions = vec![compliant_function("ok")];
        report.types = vec![
            // High cardinality type.
            TypeMetrics {
                name: "HighCard".to_string(),
                file: "a.rs".to_string(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 1,
                kind: crate::metrics::TypeKind::Struct,
                bool_fields: 8,
                option_fields: 4,
                total_fields: 12,
                state_cardinality: 4096,
                state_cardinality_log2: 12.0,
            },
            // Medium cardinality type.
            TypeMetrics {
                name: "MedCard".to_string(),
                file: "b.rs".to_string(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 1,
                kind: crate::metrics::TypeKind::Struct,
                bool_fields: 4,
                option_fields: 2,
                total_fields: 6,
                state_cardinality: 64,
                state_cardinality_log2: 6.0,
            },
            compliant_type("Good"),
        ];
        report.summary = Summary {
            production_function_count: 1,
            public_function_count: 1,
            function_overhead_ratio: 1.0,
            ..Summary::default()
        };

        let result = compute_compliance(&report, &CompliancePolicy::default());

        assert!(
            !result.heatmap.is_empty(),
            "heatmap should not be empty with violations",
        );

        // Entries must be sorted by responsibility descending.
        let resps: Vec<f64> = result.heatmap.iter().map(|e| e.responsibility).collect();
        for window in resps.windows(2) {
            assert!(
                window[0] >= window[1] - f64::EPSILON,
                "entries should be sorted by responsibility desc: {} before {}",
                window[0],
                window[1],
            );
        }
    }

    #[test]
    fn test_code_economy_heatmap_sums_to_dimension_responsibility() {
        let mut report = empty_report();
        let mut functions = vec![compliant_function("public_api")];
        for name in ["helper_a", "helper_b", "helper_c", "helper_d", "helper_e"] {
            let mut helper = compliant_function(name);
            helper.is_pub = false;
            functions.push(helper);
        }
        report.functions = functions;
        report.summary = Summary {
            production_function_count: 6,
            public_function_count: 1,
            function_overhead_ratio: 6.0,
            ..Summary::default()
        };

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let code_economy = find_soft_dim(&result, "code_economy");
        let dim_scores: Vec<f64> = result.soft_dimensions.iter().map(|d| d.score).collect();
        let dim_index = result
            .soft_dimensions
            .iter()
            .position(|d| d.name == "code_economy")
            .unwrap_or_else(|| panic!("missing code_economy"));
        let expected_total = counterfactual_composite(
            &dim_scores,
            dim_index,
            1.0,
            CompliancePolicy::default()
                .aggregation
                .objective_scalarization,
        ) - result.composite_score;

        let code_economy_entries: Vec<_> = result
            .heatmap
            .iter()
            .filter(|entry| entry.dimension == "code_economy")
            .collect();
        assert_eq!(code_economy_entries.len(), 5);

        let total: f64 = code_economy_entries
            .iter()
            .map(|entry| entry.responsibility)
            .sum();
        assert!(
            (total - expected_total).abs() < 1e-10,
            "expected total responsibility {expected_total}, got {total}",
        );
        assert!(
            code_economy.score < 1.0,
            "test setup should trigger a code_economy penalty",
        );
    }

    #[test]
    fn test_global_dimension_pipeline_traces_raw_and_calibrated_values() {
        let mut report = empty_report();
        // 6 production fns: 1 public + 5 non-public overhead.
        let mut functions = vec![compliant_function("public_api")];
        for name in ["helper_a", "helper_b", "helper_c", "helper_d", "helper_e"] {
            let mut helper = compliant_function(name);
            helper.is_pub = false;
            functions.push(helper);
        }
        report.functions = functions;
        report.summary = Summary {
            production_function_count: 6,
            public_function_count: 1,
            function_overhead_ratio: 6.0,
            ..Summary::default()
        };
        report.duplication = crate::DuplicationReport {
            functions_fingerprinted: 4,
            exact_duplicates: Vec::new(),
            near_duplicates: Vec::new(),
            duplication_score: 0.25,
        };

        let result = compute_compliance(&report, &CompliancePolicy::default());

        let code_economy = find_soft_dim(&result, "code_economy");
        assert_eq!(code_economy.pipeline.cohorts.len(), 1);
        // 5 per-function observations (one per non-public overhead fn).
        assert_eq!(code_economy.pipeline.observations.len(), 5);
        assert_eq!(code_economy.item_count, 6);
        assert!((code_economy.pipeline.aggregated_score - code_economy.score).abs() < 1e-12);

        // Each observation is a Function with raw=1.0, normalized=None, calibrated_score=0.0.
        for obs in &code_economy.pipeline.observations {
            assert_eq!(obs.kind, CompliancePipelineSubjectKind::Function);
            assert_eq!(obs.cohort, Some(NormalizationCohort::CodeEconomy));
            assert!((obs.raw - 1.0).abs() < 1e-12);
            assert!(obs.normalized.is_none());
            assert!((obs.calibrated_score - 0.0).abs() < 1e-12);
            assert!(
                obs.detail.contains("non-pub overhead fn"),
                "detail should describe overhead: {}",
                obs.detail,
            );
        }

        let duplication = find_soft_dim(&result, "duplication");
        assert!(duplication.pipeline.cohorts.is_empty());
        // No exact_duplicates or near_duplicates provided, so no per-function
        // observations are emitted (observations only cover duplicated functions).
        assert_eq!(duplication.pipeline.observations.len(), 0);
        assert!((duplication.pipeline.aggregated_score - duplication.score).abs() < 1e-12);
        assert_eq!(duplication.item_count, 4);
        assert!((duplication.score - 0.75).abs() < 1e-12);
    }

    fn production_fn(name: &str, line: usize, lines: usize) -> FunctionMetrics {
        FunctionMetrics {
            name: name.to_string(),
            file: "src/lib.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line,
            lines,
            params: 1,
            nesting_depth: 2,
            cyclomatic: 2,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }
    }

    #[test]
    fn test_bloat_heatmap_counterfactual_uses_full_population() {
        // Good functions: 10 lines. Bad functions: 200 lines.
        // Directional mode: all functions have some bloat pressure.
        let mut report = empty_report();
        report.functions = vec![
            production_fn("good_a", 1, 10),
            production_fn("good_b", 2, 10),
            production_fn("good_c", 3, 10),
            production_fn("good_d", 4, 10),
            production_fn("bad_a", 5, 200),
            production_fn("bad_b", 6, 200),
        ];
        report.summary = Summary {
            production_function_count: report.functions.len(),
            public_function_count: report.functions.len(),
            function_overhead_ratio: 1.0,
            ..Summary::default()
        };

        let good_score = {
            let n = 10.0_f64.log2() / 45.0_f64.log2();
            (-std::f64::consts::LN_2 * n.powi(3)).exp()
        };
        let bad_score = {
            let n = 200.0_f64.log2() / 45.0_f64.log2();
            (-std::f64::consts::LN_2 * n.powi(3)).exp()
        };
        let policy = CompliancePolicy::default();
        let result = compute_compliance(&report, &policy);

        // All functions have some loss in directional mode; the 200-line ones
        // have much more.  We verify the heatmap includes the two bad functions.
        let bloat_entries: Vec<_> = result
            .heatmap
            .iter()
            .filter(|entry| entry.dimension == "bloat" && entry.function_name.starts_with("bad"))
            .collect();
        assert_eq!(bloat_entries.len(), 2);

        // Counterfactual uses only composite dimensions.
        let composite_dim_scores: Vec<f64> = LossFunction::all()
            .iter()
            .zip(result.soft_dimensions.iter())
            .filter(|(lf, _)| lf.is_composite())
            .map(|(_, d)| d.score)
            .collect();
        let dim_index = LossFunction::all()
            .iter()
            .filter(|lf| lf.is_composite())
            .position(|lf| lf.name() == "bloat")
            .unwrap_or_else(|| panic!("missing bloat in composite dimensions"));

        let full_population_cf_dim_score = aggregate_artifact_scores(
            ArtifactAggregation::GeometricMeanScore,
            &[
                good_score, good_score, good_score, good_score, 1.0, bad_score,
            ],
        );
        let expected = counterfactual_composite(
            &composite_dim_scores,
            dim_index,
            full_population_cf_dim_score,
            policy.aggregation.objective_scalarization,
        ) - result.composite_score;

        for entry in bloat_entries {
            assert!(
                (entry.responsibility - expected).abs() < 1e-10,
                "expected responsibility {expected}, got {}",
                entry.responsibility,
            );
        }
    }

    // -----------------------------------------------------------------------
    // State cardinality: function-internal state
    // -----------------------------------------------------------------------

    #[test]
    fn test_state_cardinality_includes_function_internal() {
        // One type with log2=0.0 (score 1.0) + one fn with internal log2=5.0
        // fn raw=5.0, norm=5/4=1.25, score=exp(-ln2*1.25^2)=exp(-ln2*1.5625)
        // geomean([1.0, fn_score])
        let mut report = empty_report();
        report.types.push(TypeMetrics {
            name: "Simple".into(),
            file: "src/lib.rs".into(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 0,
            option_fields: 0,
            total_fields: 1,
            state_cardinality: 1,
            state_cardinality_log2: 0.0,
        });
        let mut f = compliant_function("stateful_fn");
        f.internal_state_cardinality_log2 = 5.0;
        report.functions.push(f);

        let policy = CompliancePolicy::default();
        let result = compute_compliance(&report, &policy);
        let dim = find_soft_dim(&result, "state_cardinality");
        assert_eq!(dim.item_count, 2); // 1 type + 1 fn
        let fn_score = (-std::f64::consts::LN_2 * (5.0_f64 / 4.0).powi(2)).exp();
        let expected = (1.0_f64 * fn_score).sqrt();
        assert!(
            (dim.score - expected).abs() < 1e-10,
            "expected {expected}, got {}",
            dim.score,
        );
    }

    #[test]
    fn test_state_cardinality_pipeline_traces_raw_normalized_and_aggregated_values() {
        let mut report = empty_report();
        report.types.push(TypeMetrics {
            name: "Simple".into(),
            file: "src/lib.rs".into(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 0,
            option_fields: 0,
            total_fields: 1,
            state_cardinality: 1,
            state_cardinality_log2: 0.0,
        });
        let mut f = compliant_function("stateful_fn");
        f.internal_state_cardinality_log2 = 5.0;
        report.functions.push(f);

        let result = compute_compliance(&report, &CompliancePolicy::default());
        let dim = find_soft_dim(&result, "state_cardinality");
        assert_eq!(dim.pipeline.cohorts.len(), 2);
        assert_eq!(dim.pipeline.observations.len(), 2);
        assert!((dim.pipeline.aggregated_score - dim.score).abs() < 1e-12);

        let Some(type_observation) = dim
            .pipeline
            .observations
            .iter()
            .find(|observation| observation.kind == CompliancePipelineSubjectKind::Type)
        else {
            panic!("type observation should exist");
        };
        assert_eq!(
            type_observation.cohort,
            Some(NormalizationCohort::StateCardinalityType)
        );
        // Directional: raw = max(0, log2_card) = 0.0
        assert!((type_observation.raw - 0.0).abs() < 1e-12);
        let Some(normalized_type) = type_observation.normalized else {
            panic!("normalized type should be Some");
        };
        // normalized = 0.0 / 4.0 = 0.0
        assert!((normalized_type - 0.0).abs() < 1e-12);
        // score = exp(-ln2 * 0) = 1.0
        assert!((type_observation.calibrated_score - 1.0).abs() < 1e-12);

        let Some(fn_observation) = dim
            .pipeline
            .observations
            .iter()
            .find(|observation| observation.kind == CompliancePipelineSubjectKind::Function)
        else {
            panic!("function observation should exist");
        };
        assert_eq!(
            fn_observation.cohort,
            Some(NormalizationCohort::StateCardinalityFunction)
        );
        // Directional: raw = max(0, 5.0) = 5.0
        assert!((fn_observation.raw - 5.0).abs() < 1e-12);
        let Some(normalized_fn) = fn_observation.normalized else {
            panic!("normalized fn should be Some");
        };
        // normalized = 5.0 / 4.0 = 1.25
        assert!((normalized_fn - 5.0 / 4.0).abs() < 1e-12);
        // score = exp(-ln2 * (5/4)^2)
        let expected_fn_score = (-std::f64::consts::LN_2 * (5.0_f64 / 4.0).powi(2)).exp();
        assert!((fn_observation.calibrated_score - expected_fn_score).abs() < 1e-12);
    }

    #[test]
    fn test_state_cardinality_excludes_zero_cardinality_functions() {
        let mut report = empty_report();
        report.types.push(TypeMetrics {
            name: "Big".into(),
            file: "src/lib.rs".into(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 8,
            option_fields: 4,
            total_fields: 12,
            state_cardinality: 4096,
            state_cardinality_log2: 12.0,
        });
        // Function with zero internal cardinality — should NOT participate
        let mut f = compliant_function("pure_fn");
        f.internal_state_cardinality_log2 = 0.0;
        report.functions.push(f);

        let policy = CompliancePolicy::default();
        let result = compute_compliance(&report, &policy);
        let dim = find_soft_dim(&result, "state_cardinality");
        assert_eq!(dim.item_count, 1); // only the type
        // raw=12.0, norm=12/4=3.0, score=exp(-ln2*9)
        let expected = (-std::f64::consts::LN_2 * 9.0).exp();
        assert!(
            (dim.score - expected).abs() < 1e-10,
            "expected {}, got {}",
            expected,
            dim.score,
        );
    }

    #[test]
    fn test_state_cardinality_excludes_test_functions() {
        let mut report = empty_report();
        let mut f = compliant_function("test_fn");
        f.is_test = true;
        f.internal_state_cardinality_log2 = 5.0; // should be excluded
        report.functions.push(f);

        let policy = CompliancePolicy::default();
        let result = compute_compliance(&report, &policy);
        let dim = find_soft_dim(&result, "state_cardinality");
        assert_eq!(dim.item_count, 0);
        assert!((dim.score - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_state_cardinality_supports_power_mean_loss_aggregation() {
        let mut report = empty_report();
        report.types = vec![
            TypeMetrics {
                name: "Borderline".into(),
                file: "src/lib.rs".into(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 1,
                kind: crate::metrics::TypeKind::Struct,
                bool_fields: 0,
                option_fields: 0,
                total_fields: 1,
                state_cardinality: 16,
                state_cardinality_log2: 4.0,
            },
            TypeMetrics {
                name: "Severe".into(),
                file: "src/lib.rs".into(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 2,
                kind: crate::metrics::TypeKind::Struct,
                bool_fields: 0,
                option_fields: 0,
                total_fields: 1,
                state_cardinality: 1024,
                state_cardinality_log2: 10.0,
            },
        ];

        let mut policy = CompliancePolicy::default();
        policy.aggregation.state_cardinality_aggregation =
            ArtifactAggregation::PowerMeanLoss { p: 2.0 };

        let result = compute_compliance(&report, &policy);
        let dim = find_soft_dim(&result, "state_cardinality");

        // Directional: raw = log2_card.max(0), normalized = raw / 4.0
        // Borderline log2=4.0: norm=1.0, score=exp(-ln2*1.0)
        // Severe log2=10.0: norm=2.5, score=exp(-ln2*6.25)
        let score_a = (-std::f64::consts::LN_2 * (4.0_f64 / 4.0).powi(2)).exp();
        let score_b = (-std::f64::consts::LN_2 * (10.0_f64 / 4.0).powi(2)).exp();
        let loss_a: f64 = 1.0 - score_a;
        let loss_b: f64 = 1.0 - score_b;
        let expected_loss = ((loss_a.powi(2) + loss_b.powi(2)) / 2.0).sqrt();
        let expected = 1.0 - expected_loss;
        assert!(
            (dim.score - expected).abs() < 1e-10,
            "expected {expected}, got {}",
            dim.score,
        );
    }

    #[test]
    fn test_objective_scalarization_can_use_arithmetic_mean() {
        let mut report = empty_report();
        report.functions = vec![violating_function("a"), violating_function("b")];
        report.types = vec![violating_type("Bad")];
        report.duplication.duplication_score = 0.8;
        report.summary.public_function_count = 1;
        report.summary.production_function_count = 2;
        report.summary.function_overhead_ratio = 8.0;

        let mut policy = CompliancePolicy::default();
        policy.aggregation.objective_scalarization = ObjectiveScalarization::ArithmeticMeanScore;

        let result = compute_compliance(&report, &policy);
        let composite_scores: Vec<f64> = LossFunction::all()
            .iter()
            .zip(result.soft_dimensions.iter())
            .filter(|(lf, _)| lf.is_composite())
            .map(|(_, d)| d.score)
            .collect();
        let expected = composite_scores.iter().sum::<f64>() / composite_scores.len() as f64;
        assert!(
            (result.composite_score - expected).abs() < 1e-10,
            "expected arithmetic composite {expected}, got {}",
            result.composite_score,
        );
    }

    #[test]
    fn test_bloat_supports_calibration_override() {
        // lines=70, raw = log2(70), normalized = log2(70)/log2(45) ~= 1.116.
        // LinearDecay zero_at=3.0: 1 - (1.116-1.0)/(3.0-1.0) = 0.942.
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "moderate".to_string(),
            file: "src/lib.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            lines: 70,
            params: 1,
            nesting_depth: 2,
            cyclomatic: 10,
            mutable_bindings: 0,
            internal_state_cardinality_log2: 0.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let mut policy = CompliancePolicy::default();
        policy.calibration.overrides.insert(
            NormalizationCohort::BloatFunction,
            crate::calibration::Calibrator::LinearDecayScore { zero_at: 3.0 },
        );

        let result = compute_compliance(&report, &policy);
        let dim = find_soft_dim(&result, "bloat");
        let normalized = 70.0_f64.log2() / 45.0_f64.log2();
        let expected = 1.0 - (normalized - 1.0) / (3.0 - 1.0);
        assert!(
            (dim.score - expected).abs() < 1e-10,
            "expected linear-decay calibrated score ~{expected:.6}, got {}",
            dim.score,
        );
    }

    #[test]
    fn test_state_cardinality_heatmap_includes_functions() {
        let mut report = empty_report();
        let mut f = compliant_function("mutable_fn");
        f.internal_state_cardinality_log2 = 5.0; // above K=3 threshold
        f.file = "src/lib.rs".into();
        f.line = 42;
        report.functions.push(f);
        // Need a public function for code_economy to not be degenerate
        report.summary.public_function_count = 1;
        report.summary.production_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let policy = CompliancePolicy::default();
        let result = compute_compliance(&report, &policy);
        let fn_entries: Vec<_> = result
            .heatmap
            .iter()
            .filter(|e| e.dimension == "state_cardinality" && e.function_name == "mutable_fn")
            .collect();
        assert!(
            !fn_entries.is_empty(),
            "expected heatmap entry for mutable_fn",
        );
        assert!(
            fn_entries[0]
                .detail
                .contains("fn internal log2 cardinality")
        );
    }

    #[test]
    fn test_nested_policy_json_deserializes() {
        let json = r#"{
            "directional": {
                "code_economy_log2_overhead": 2.0,
                "state_cardinality_log2": 4.0,
                "bloat_log2_lines": 3.5
            },
            "normalization": {
                "overrides": {
                    "bloat_function": {
                        "kind": "cohort_mean_stddev",
                        "stddev_multiplier": 1.5,
                        "min_count": 4
                    }
                }
            },
            "calibration": {
                "overrides": {
                    "bloat_function": { "kind": "linear_decay_score", "zero_at": 3.0 }
                }
            },
            "aggregation": {
                "state_cardinality_aggregation": { "kind": "power_mean_loss", "p": 2.0 },
                "objective_scalarization": { "kind": "arithmetic_mean_score" }
            }
        }"#;

        let Ok(policy) = serde_json::from_str::<CompliancePolicy>(json) else {
            panic!("nested policy JSON should deserialize");
        };

        assert!((policy.directional.code_economy_log2_overhead - 2.0).abs() < 1e-12);
        assert!((policy.directional.state_cardinality_log2 - 4.0).abs() < 1e-12);
        assert!((policy.directional.bloat_log2_lines - 3.5).abs() < 1e-12);
        assert_eq!(
            policy
                .normalization
                .overrides
                .get(&NormalizationCohort::BloatFunction),
            Some(
                &crate::normalization::CohortNormalizationStrategy::CohortMeanStddev {
                    stddev_multiplier: 1.5,
                    min_count: 4,
                }
            )
        );
        assert_eq!(
            policy.aggregation.state_cardinality_aggregation,
            ArtifactAggregation::PowerMeanLoss { p: 2.0 }
        );
        assert_eq!(
            policy.aggregation.objective_scalarization,
            ObjectiveScalarization::ArithmeticMeanScore
        );
        assert_eq!(
            policy
                .calibration
                .calibrator_for(NormalizationCohort::BloatFunction),
            crate::calibration::Calibrator::LinearDecayScore { zero_at: 3.0 }
        );
    }

    #[test]
    fn test_compute_compliance_with_context_supports_shared_cohort_normalization() {
        let mut report = empty_report();
        let mut bloat_fn = compliant_function("contextual_bloat");
        bloat_fn.lines = 20;
        report.functions = vec![bloat_fn];
        report.summary.production_function_count = 1;
        report.summary.public_function_count = 1;
        report.summary.function_overhead_ratio = 1.0;

        let mut policy = CompliancePolicy::default();
        policy.normalization.overrides.insert(
            NormalizationCohort::BloatFunction,
            crate::normalization::CohortNormalizationStrategy::CohortMean {
                multiplier: 1.0,
                min_count: 5,
            },
        );

        let mut builder = crate::normalization::NormalizationContextBuilder::default();
        for value in [40.0, 40.0, 40.0, 40.0, 40.0] {
            builder.observe(NormalizationCohort::BloatFunction, value);
        }
        let normalization_context = builder.build();

        let result =
            compute_compliance_with_context(&report, &policy, &normalization_context, None);
        let bloat = find_soft_dim(&result, "bloat");

        // Directional mode: raw=log2(20), cohort-mean normalizer threshold=40.0,
        // normalized=log2(20)/40~0.108, StretchedExponentialDecay score is very
        // close to 1.0 but not exact.
        assert!(
            bloat.score > 0.99,
            "shared cohort mean should treat 20 lines as nearly compliant when cohort mean is 40, got {}",
            bloat.score,
        );
    }

    #[test]
    fn test_compute_compliance_matches_explicit_report_context() {
        let mut report = empty_report();
        let mut bad = violating_function("bad");
        bad.cyclomatic = 4;
        bad.lines = 80;
        bad.internal_state_cardinality_log2 = 5.0;

        report.functions = vec![compliant_function("good"), bad];
        report.types = vec![violating_type("Flags")];
        report.summary = Summary {
            production_function_count: 2,
            public_function_count: 1,
            function_overhead_ratio: 2.0,
            ..Summary::default()
        };

        let policy = CompliancePolicy::default();
        let explicit_context = NormalizationContext::from_report(&report);
        let implicit = compute_compliance(&report, &policy);
        let explicit = compute_compliance_with_context(&report, &policy, &explicit_context, None);

        assert_eq!(
            implicit.soft_dimensions.len(),
            explicit.soft_dimensions.len()
        );
        for (implicit_dim, explicit_dim) in implicit
            .soft_dimensions
            .iter()
            .zip(&explicit.soft_dimensions)
        {
            assert_eq!(implicit_dim.name, explicit_dim.name);
            assert_eq!(implicit_dim.item_count, explicit_dim.item_count);
            assert_eq!(implicit_dim.rule, explicit_dim.rule);
            assert!(
                (implicit_dim.score - explicit_dim.score).abs() < 1e-12,
                "dimension {} should match between implicit and explicit contexts",
                implicit_dim.name,
            );
        }
        assert!((implicit.composite_score - explicit.composite_score).abs() < 1e-12);
        assert_eq!(implicit.heatmap.len(), explicit.heatmap.len());
        for (implicit_entry, explicit_entry) in implicit.heatmap.iter().zip(&explicit.heatmap) {
            assert_eq!(implicit_entry.file, explicit_entry.file);
            assert_eq!(implicit_entry.line, explicit_entry.line);
            assert_eq!(implicit_entry.function_name, explicit_entry.function_name);
            assert_eq!(implicit_entry.dimension, explicit_entry.dimension);
            assert_eq!(implicit_entry.detail, explicit_entry.detail);
            assert!((implicit_entry.responsibility - explicit_entry.responsibility).abs() < 1e-12);
        }
    }

    #[test]
    fn test_heatmap_responsibility_is_bounded_by_full_dimension_fix_with_nondefault_aggregation() {
        let mut report = empty_report();
        let mut mild = compliant_function("mild");
        mild.lines = 45; // above K=35
        let mut severe = compliant_function("severe");
        severe.lines = 90; // above K=35
        let mut worst = compliant_function("worst");
        worst.lines = 120; // above K=35
        report.functions = vec![mild, severe, worst];
        report.summary = Summary {
            production_function_count: 3,
            public_function_count: 3,
            function_overhead_ratio: 1.0,
            ..Summary::default()
        };

        let mut policy = CompliancePolicy::default();
        policy.aggregation.bloat_aggregation = ArtifactAggregation::MeanPlusCvarLoss {
            alpha: 0.34,
            tail_weight: 0.5,
        };
        policy.aggregation.objective_scalarization = ObjectiveScalarization::ArithmeticMeanScore;

        let result = compute_compliance(&report, &policy);
        let dim_scores: Vec<f64> = result.soft_dimensions.iter().map(|d| d.score).collect();
        let Some(bloat_index) = LossFunction::all()
            .iter()
            .position(|kind| *kind == LossFunction::Bloat)
        else {
            panic!("bloat dimension should exist");
        };
        let full_bloat_fix = counterfactual_composite(
            &dim_scores,
            bloat_index,
            1.0,
            policy.aggregation.objective_scalarization,
        ) - result.composite_score;

        assert!(full_bloat_fix > 0.0);

        let bloat_entries: Vec<_> = result
            .heatmap
            .iter()
            .filter(|entry| entry.dimension == "bloat")
            .collect();
        assert_eq!(bloat_entries.len(), 3);

        for entry in bloat_entries {
            assert!(entry.responsibility >= 0.0);
            assert!(
                entry.responsibility <= full_bloat_fix + 1e-12,
                "single-item responsibility {} should not exceed full-dimension fix {full_bloat_fix}",
                entry.responsibility,
            );
        }
    }

    #[test]
    fn test_report_deserializes_policy_field() {
        let json = r#"{
            "soft_dimensions": [],
            "composite_loss": 0.0,
            "heatmap": [],
            "policy": {
                "directional": {
                    "code_economy_log2_overhead": 2.5
                }
            }
        }"#;

        let Ok(report) = serde_json::from_str::<ComplianceReport>(json) else {
            panic!("report policy field should deserialize");
        };

        assert!((report.policy.directional.code_economy_log2_overhead - 2.5).abs() < f64::EPSILON);
    }

    // -----------------------------------------------------------------------
    // CouplingDensityDimension tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_coupling_density_without_semantic_data() {
        let report = empty_report();
        let result = compute_compliance_with_semantic(&report, &CompliancePolicy::default(), None);
        let dim = find_soft_dim(&result, "coupling_density");
        assert!(
            (dim.score - 1.0).abs() < f64::EPSILON,
            "no semantic data should yield score 1.0, got {}",
            dim.score,
        );
    }

    #[test]
    fn test_coupling_density_with_zero_edges() {
        use crate::semantic::{CouplingData, SemanticOverlay};
        use std::collections::{BTreeMap, BTreeSet};

        let report = empty_report();
        let overlay = SemanticOverlay {
            coupling: CouplingData {
                density: 0.0,
                module_count: 4,
                edge_count: 0,
                module_outgoing_edges: BTreeMap::new(),
                all_modules: ["a", "b", "c", "d"]
                    .iter()
                    .map(ToString::to_string)
                    .collect::<BTreeSet<_>>(),
                module_files: BTreeMap::new(),
                function_outgoing_edges: BTreeMap::new(),
                function_files: BTreeMap::new(),
            },
            ..Default::default()
        };
        let result =
            compute_compliance_with_semantic(&report, &CompliancePolicy::default(), Some(&overlay));
        let dim = find_soft_dim(&result, "coupling_density");
        assert!(
            (dim.score - 1.0).abs() < f64::EPSILON,
            "zero edges should yield score 1.0, got {}",
            dim.score,
        );
        // Should have per-module observations.
        assert_eq!(dim.pipeline.observations.len(), 4);
    }

    #[test]
    fn test_coupling_density_per_module_scoring() {
        // Build overlay via from_data so all_modules, module_files, etc. are populated.
        // With per-function data available, observations should be per-function
        // for callers, plus per-module for callee-only modules.
        use crate::semantic::{CallEdge, SemanticData, SemanticOverlay};

        let report = empty_report();
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: vec![
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "b".into(),
                    callee_file: "b.rs".into(),
                    caller_function: "do_stuff".into(),
                    caller_line: 10,
                },
                CallEdge {
                    caller_module: "b".into(),
                    caller_file: "b.rs".into(),
                    callee_module: "c".into(),
                    callee_file: "c.rs".into(),
                    caller_function: "handle".into(),
                    caller_line: 20,
                },
            ],
        };
        let overlay = SemanticOverlay::from_data(&data);
        // 3 modules (a, b, c), 2 edges. a::do_stuff->b, b::handle->c.
        // Per-function: a::do_stuff has 1 outgoing, b::handle has 1 outgoing.
        // Module c: callee-only, 0 outgoing, gets a Module observation.
        // Total: 2 function + 1 module = 3 observations.
        let result =
            compute_compliance_with_semantic(&report, &CompliancePolicy::default(), Some(&overlay));
        let dim = find_soft_dim(&result, "coupling_density");
        assert_eq!(dim.pipeline.observations.len(), 3);

        // Verify observation kinds.
        let fn_count = dim
            .pipeline
            .observations
            .iter()
            .filter(|o| o.kind == CompliancePipelineSubjectKind::Function)
            .count();
        let mod_count = dim
            .pipeline
            .observations
            .iter()
            .filter(|o| o.kind == CompliancePipelineSubjectKind::Module)
            .count();
        assert_eq!(fn_count, 2, "should have 2 per-function observations");
        assert_eq!(
            mod_count, 1,
            "should have 1 per-module observation for callee-only module c"
        );

        assert!(
            dim.score < 1.0,
            "functions with edges should yield score < 1.0, got {}",
            dim.score,
        );
        assert!(
            dim.score > 0.0,
            "score should be positive, got {}",
            dim.score,
        );
    }

    #[test]
    fn test_coupling_density_legacy_fallback_per_module() {
        // When caller_function is empty (legacy data), observations should be
        // per-module, not per-function.
        use crate::semantic::{CallEdge, SemanticData, SemanticOverlay};

        let report = empty_report();
        let data = SemanticData {
            crate_name: "test".into(),
            type_cardinalities: Vec::new(),
            function_cardinalities: Vec::new(),
            call_edges: vec![
                CallEdge {
                    caller_module: "a".into(),
                    caller_file: "a.rs".into(),
                    callee_module: "b".into(),
                    callee_file: "b.rs".into(),
                    caller_function: String::new(),
                    caller_line: 0,
                },
                CallEdge {
                    caller_module: "b".into(),
                    caller_file: "b.rs".into(),
                    callee_module: "c".into(),
                    callee_file: "c.rs".into(),
                    caller_function: String::new(),
                    caller_line: 0,
                },
            ],
        };
        let overlay = SemanticOverlay::from_data(&data);
        let result =
            compute_compliance_with_semantic(&report, &CompliancePolicy::default(), Some(&overlay));
        let dim = find_soft_dim(&result, "coupling_density");
        assert_eq!(dim.pipeline.observations.len(), 3);

        // All observations should be Module kind (legacy fallback).
        for obs in &dim.pipeline.observations {
            assert_eq!(
                obs.kind,
                CompliancePipelineSubjectKind::Module,
                "legacy data should produce per-module observations, got {:?} for {}",
                obs.kind,
                obs.name,
            );
        }
    }

    #[test]
    fn test_coupling_density_empty_overlay() {
        use crate::semantic::SemanticOverlay;

        let report = empty_report();
        let overlay = SemanticOverlay::default();
        let result =
            compute_compliance_with_semantic(&report, &CompliancePolicy::default(), Some(&overlay));
        let dim = find_soft_dim(&result, "coupling_density");
        assert!(
            (dim.score - 1.0).abs() < f64::EPSILON,
            "empty overlay should yield score 1.0, got {}",
            dim.score,
        );
    }

    #[test]
    fn test_coupling_density_fallback_to_semantic_summary() {
        use crate::metrics::SemanticSummary;

        let mut report = empty_report();
        report.semantic = Some(SemanticSummary {
            coupling_density: 0.3,
            coupling_module_count: 4,
            coupling_edge_count: 3,
        });
        // Pass None for live overlay -- the dimension should fall back to the
        // saved SemanticSummary on the report.
        let result = compute_compliance_with_semantic(&report, &CompliancePolicy::default(), None);
        let dim = find_soft_dim(&result, "coupling_density");
        assert!(
            (dim.score - 0.7).abs() < 1e-10,
            "fallback density 0.3 should yield score 0.7, got {}",
            dim.score,
        );
    }

    // -----------------------------------------------------------------------
    // Semantic enrichment of state cardinality
    // -----------------------------------------------------------------------

    #[test]
    fn test_state_cardinality_uses_semantic_overlay() {
        use crate::semantic::SemanticOverlay;

        // A type with syn-computed log2 = 1.0. Directional mode:
        //   raw = max(0, 1.0) = 1.0, norm = 1/4 = 0.25, score = exp(-ln2*0.0625)
        let mut report = empty_report();
        report.types = vec![TypeMetrics {
            name: "MyStruct".to_string(),
            file: "lib.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 1,
            option_fields: 0,
            total_fields: 1,
            state_cardinality: 2,
            state_cardinality_log2: 1.0,
        }];

        let policy = CompliancePolicy::default();

        // Without overlay: score = exp(-ln2 * (1/4)^2) = exp(-ln2 * 0.0625)
        let without = compute_compliance_with_semantic(&report, &policy, None);
        let dim_without = find_soft_dim(&without, "state_cardinality");
        let expected_without = (-std::f64::consts::LN_2 * (1.0_f64 / 4.0).powi(2)).exp();
        assert!(
            (dim_without.score - expected_without).abs() < 1e-10,
            "without overlay, score should be ~{expected_without:.4}, got {}",
            dim_without.score,
        );

        // Overlay resolves the same type to log2 = 4.0:
        //   raw = 4.0, norm = 1.0, score = exp(-ln2 * 1.0) = 0.5
        let mut overlay = SemanticOverlay::default();
        overlay.type_cardinalities.insert(
            ("lib.rs".to_string(), String::new(), "MyStruct".to_string()),
            4.0,
        );

        let with = compute_compliance_with_semantic(&report, &policy, Some(&overlay));
        let dim_with = find_soft_dim(&with, "state_cardinality");
        let expected_with = (-std::f64::consts::LN_2 * (4.0_f64 / 4.0).powi(2)).exp();
        assert!(
            (dim_with.score - expected_with).abs() < 1e-10,
            "with overlay (log2=4.0), score should be ~{expected_with:.4}, got {}",
            dim_with.score,
        );
        assert!(
            dim_with.score < dim_without.score,
            "overlay with higher cardinality should produce a worse (lower) score",
        );
    }

    #[test]
    fn test_state_cardinality_falls_back_without_overlay() {
        // Same type as above; passing None for the overlay must produce the
        // same score as the non-semantic compute_compliance entry point.
        let mut report = empty_report();
        report.types = vec![TypeMetrics {
            name: "MyStruct".to_string(),
            file: "lib.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 1,
            kind: crate::metrics::TypeKind::Struct,
            bool_fields: 1,
            option_fields: 0,
            total_fields: 1,
            state_cardinality: 2,
            state_cardinality_log2: 1.0,
        }];

        let policy = CompliancePolicy::default();
        let baseline = compute_compliance(&report, &policy);
        let with_none = compute_compliance_with_semantic(&report, &policy, None);

        let dim_baseline = find_soft_dim(&baseline, "state_cardinality");
        let dim_with_none = find_soft_dim(&with_none, "state_cardinality");
        assert!(
            (dim_baseline.score - dim_with_none.score).abs() < f64::EPSILON,
            "passing None should match baseline: {} vs {}",
            dim_baseline.score,
            dim_with_none.score,
        );
    }

    #[test]
    fn test_function_cardinality_uses_semantic_overlay() {
        use crate::semantic::SemanticOverlay;

        // Known limitation: state_cardinality_observations filters functions
        // with internal_state_cardinality_log2 <= 0.0 BEFORE consulting the
        // overlay. A function whose syn-computed cardinality is 0.0 will be
        // skipped even if the overlay provides a nonzero value. This is
        // arguably a bug (syn can miss `let mut x = compute_flag()`) but
        // documenting it here as tested behavior.
        //
        // To exercise the overlay path, the function must have syn-computed
        // cardinality > 0.0 so it passes the filter.
        let mut report = empty_report();
        report.functions = vec![FunctionMetrics {
            name: "process".to_string(),
            file: "lib.rs".to_string(),
            module_path: String::new(),
            scope_path: Vec::new(),
            line: 10,
            lines: 20,
            params: 2,
            nesting_depth: 2,
            cyclomatic: 3,
            mutable_bindings: 1,
            internal_state_cardinality_log2: 1.0,
            assertions: 0,
            meaningful_assertions: 0,
            is_test: false,
            is_pub: true,
        }];

        let policy = CompliancePolicy::default();

        // Without overlay: raw = max(0, 1.0) = 1.0, norm = 1/4 = 0.25,
        // score = exp(-ln2 * 0.0625).
        let without = compute_compliance_with_semantic(&report, &policy, None);
        let dim_without = find_soft_dim(&without, "state_cardinality");
        let expected_without = (-std::f64::consts::LN_2 * (1.0_f64 / 4.0).powi(2)).exp();
        assert!(
            (dim_without.score - expected_without).abs() < 1e-10,
            "without overlay, fn score should be ~{expected_without:.4}, got {}",
            dim_without.score,
        );

        // Overlay resolves the function to log2 = 5.0:
        //   raw = 5.0, norm = 5/4 = 1.25, score = exp(-ln2 * 1.5625).
        let mut overlay = SemanticOverlay::default();
        overlay.function_cardinalities.insert(
            (
                "lib.rs".to_string(),
                String::new(),
                "process".to_string(),
                10,
            ),
            5.0,
        );

        let with = compute_compliance_with_semantic(&report, &policy, Some(&overlay));
        let dim_with = find_soft_dim(&with, "state_cardinality");
        let expected = (-std::f64::consts::LN_2 * (5.0_f64 / 4.0).powi(2)).exp();
        assert!(
            (dim_with.score - expected).abs() < 1e-10,
            "with overlay (log2=5.0), fn score should be ~{expected:.4}, got {}",
            dim_with.score,
        );
        assert!(
            dim_with.score < dim_without.score,
            "overlay with higher cardinality should produce a worse (lower) score",
        );
    }
}

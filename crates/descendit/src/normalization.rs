//! Normalization helpers for converting raw metric values into comparable scales.
//!
//! The current pipeline is:
//!
//! raw metric -> normalized metric
//!
//! Defaults remain threshold-relative, but this module now also provides:
//!
//! - online/mergeable cohort statistics via Welford updates
//! - report-derived normalization contexts
//! - opt-in cohort-relative normalizers that derive scale from observed data
//!
//! Calibration and aggregation remain separate modules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::metrics::AnalysisReport;

/// A raw metric value normalized onto a dimensionless scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizedMetric {
    /// Original metric in source units.
    pub raw: f64,
    /// Dimensionless normalized value.
    pub normalized: f64,
}

impl NormalizedMetric {
    /// Whether this metric exceeds its target band.
    pub fn exceeds_target(self) -> bool {
        self.normalized > 1.0
    }
}

/// Summary statistics for a cohort of raw metric observations.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CohortStats {
    pub count: usize,
    pub mean: f64,
    pub stddev: f64,
    pub min: f64,
    pub max: f64,
    pub q25: f64,
    pub q50: f64,
    pub q75: f64,
}

impl CohortStats {
    pub fn iqr(self) -> f64 {
        self.q75 - self.q25
    }

    pub fn robust_stddev(self) -> f64 {
        self.iqr() / 1.349
    }
}

/// Online, mergeable cohort statistics using Welford updates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnlineStats {
    count: usize,
    mean: f64,
    m2: f64,
    min: f64,
    max: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct CohortAccumulator {
    online: OnlineStats,
    sketch: QuantileSketch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OnlineStatsSnapshot {
    count: usize,
    mean: f64,
    stddev: f64,
    min: f64,
    max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct QuantileCentroid {
    mean: f64,
    weight: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct CompressionProgress {
    centroid_index: usize,
    centroid_progress: f64,
}

/// A bounded, mergeable quantile sketch.
///
/// Small cohorts remain exact. Larger cohorts are compacted into a fixed
/// number of weighted centroids so quartile estimation remains bounded in
/// memory while still supporting deterministic merges.
#[derive(Debug, Clone, PartialEq)]
struct QuantileSketch {
    centroids: Vec<QuantileCentroid>,
    max_centroids: usize,
}

impl Default for OnlineStats {
    fn default() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl Default for QuantileSketch {
    fn default() -> Self {
        Self {
            centroids: Vec::new(),
            max_centroids: default_max_quantile_centroids(),
        }
    }
}

/// Supported metric normalizers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Normalizer {
    /// Normalize by dividing through a target threshold.
    ThresholdRatio { threshold: f64 },
    /// Normalize by dividing through a threshold derived from the cohort mean.
    CohortMeanRatio {
        threshold: f64,
        mean: f64,
        multiplier: f64,
        count: usize,
    },
    /// Normalize by dividing through a threshold derived from mean + k * stddev.
    CohortMeanStddevRatio {
        threshold: f64,
        mean: f64,
        stddev: f64,
        stddev_multiplier: f64,
        count: usize,
    },
    /// Normalize by dividing through a threshold derived from the cohort upper quartile.
    CohortUpperQuartileRatio {
        threshold: f64,
        q75: f64,
        multiplier: f64,
        count: usize,
    },
    /// Normalize by dividing through a threshold derived from median + k * robust_stddev.
    CohortMedianIqrRatio {
        threshold: f64,
        median: f64,
        iqr: f64,
        iqr_multiplier: f64,
        count: usize,
    },
}

/// Stable cohort keys for the soft-scoring pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalizationCohort {
    CodeEconomy,
    StateCardinalityType,
    StateCardinalityFunction,
    BloatFunction,
    CouplingModule,
}

impl NormalizationCohort {
    pub fn all() -> &'static [NormalizationCohort] {
        &[
            NormalizationCohort::CodeEconomy,
            NormalizationCohort::StateCardinalityType,
            NormalizationCohort::StateCardinalityFunction,
            NormalizationCohort::BloatFunction,
            NormalizationCohort::CouplingModule,
        ]
    }
}

/// Cohort-level normalization strategies.
///
/// The default strategy preserves the current threshold-relative behavior. More
/// sophisticated robust or online strategies can be added here later without
/// changing dimension code.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CohortNormalizationStrategy {
    /// Use the dimension's target threshold directly.
    #[default]
    UseTargetThreshold,
    /// Scale the dimension's target threshold for this cohort.
    ScaledTargetThreshold { multiplier: f64 },
    /// Use the observed cohort mean, optionally scaled, as the normalization reference.
    CohortMean {
        #[serde(default = "default_cohort_mean_multiplier")]
        multiplier: f64,
        /// Minimum number of raw observations required before switching away
        /// from the target-threshold fallback. This is cohort-specific:
        /// `code_economy` contributes one observation per report, while
        /// per-item cohorts such as `bloat_function` contribute one per item.
        #[serde(default = "default_min_cohort_count")]
        min_count: usize,
    },
    /// Use `mean + k * stddev` as the normalization reference.
    CohortMeanStddev {
        #[serde(default = "default_cohort_stddev_multiplier")]
        stddev_multiplier: f64,
        /// Minimum number of raw observations required before switching away
        /// from the target-threshold fallback. This is cohort-specific:
        /// `code_economy` contributes one observation per report, while
        /// per-item cohorts such as `bloat_function` contribute one per item.
        #[serde(default = "default_min_cohort_count")]
        min_count: usize,
    },
    /// Use the observed upper quartile, optionally scaled, as a robust threshold.
    CohortUpperQuartile {
        #[serde(default = "default_cohort_upper_quartile_multiplier")]
        multiplier: f64,
        /// Minimum number of raw observations required before switching away
        /// from the target-threshold fallback. This is cohort-specific:
        /// `code_economy` contributes one observation per report, while
        /// per-item cohorts such as `bloat_function` contribute one per item.
        #[serde(default = "default_min_cohort_count")]
        min_count: usize,
    },
    /// Use `median + k * robust_stddev`, where `robust_stddev = IQR / 1.349`.
    CohortMedianIqr {
        #[serde(default = "default_cohort_iqr_multiplier")]
        iqr_multiplier: f64,
        /// Minimum number of raw observations required before switching away
        /// from the target-threshold fallback. This is cohort-specific:
        /// `code_economy` contributes one observation per report, while
        /// per-item cohorts such as `bloat_function` contribute one per item.
        #[serde(default = "default_min_cohort_count")]
        min_count: usize,
    },
}

/// Cohort-aware normalization policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NormalizationPolicy {
    /// Optional per-cohort overrides. Missing cohorts fall back to the
    /// dimension's default target-threshold normalizer.
    #[serde(default)]
    pub overrides: BTreeMap<NormalizationCohort, CohortNormalizationStrategy>,
}

/// Resolved cohort statistics used during normalization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NormalizationContext {
    cohorts: BTreeMap<NormalizationCohort, CohortStats>,
}

/// Builder for a normalization context that can be updated online across reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NormalizationContextBuilder {
    cohorts: BTreeMap<NormalizationCohort, CohortAccumulator>,
}

fn default_cohort_mean_multiplier() -> f64 {
    1.0
}

fn default_cohort_stddev_multiplier() -> f64 {
    1.0
}

fn default_cohort_upper_quartile_multiplier() -> f64 {
    1.0
}

fn default_cohort_iqr_multiplier() -> f64 {
    1.0
}

fn default_min_cohort_count() -> usize {
    5
}

fn default_max_quantile_centroids() -> usize {
    128
}

impl Normalizer {
    pub fn threshold_ratio(threshold: f64) -> Self {
        Self::ThresholdRatio { threshold }
    }

    fn threshold(self) -> f64 {
        match self {
            Self::ThresholdRatio { threshold }
            | Self::CohortMeanRatio { threshold, .. }
            | Self::CohortMeanStddevRatio { threshold, .. }
            | Self::CohortUpperQuartileRatio { threshold, .. }
            | Self::CohortMedianIqrRatio { threshold, .. } => threshold,
        }
    }

    pub fn normalize(self, raw: f64) -> NormalizedMetric {
        let threshold = self.threshold();
        let normalized = if threshold <= f64::EPSILON {
            0.0
        } else {
            raw / threshold
        };
        NormalizedMetric { raw, normalized }
    }
}

impl CohortAccumulator {
    fn observe(&mut self, value: f64) {
        self.online.observe(value);
        if value.is_finite() {
            self.sketch.observe(value);
        }
    }

    fn merge(&mut self, other: Self) {
        self.online.merge(other.online);
        self.sketch.merge(other.sketch);
    }

    fn snapshot(self) -> Option<CohortStats> {
        let online = self.online.snapshot()?;
        let (q25, q50, q75) = self.sketch.quartiles()?;

        Some(CohortStats {
            count: online.count,
            mean: online.mean,
            stddev: online.stddev,
            min: online.min,
            max: online.max,
            q25,
            q50,
            q75,
        })
    }
}

impl QuantileSketch {
    fn observe(&mut self, value: f64) {
        self.centroids.push(QuantileCentroid {
            mean: value,
            weight: 1.0,
        });
        self.compress_if_needed();
    }

    fn merge(&mut self, other: Self) {
        if other.centroids.is_empty() {
            return;
        }
        if self.centroids.is_empty() {
            self.max_centroids = self.max_centroids.max(other.max_centroids);
            self.centroids = other.centroids;
            self.compress_if_needed();
            return;
        }

        self.max_centroids = self.max_centroids.max(other.max_centroids);
        self.centroids.extend(other.centroids);
        self.compress_if_needed();
    }

    fn quartiles(mut self) -> Option<(f64, f64, f64)> {
        if self.centroids.is_empty() {
            return None;
        }

        sort_centroids_by_mean(&mut self.centroids);
        Some((
            quantile_from_sorted_centroids(&self.centroids, 0.25)?,
            quantile_from_sorted_centroids(&self.centroids, 0.50)?,
            quantile_from_sorted_centroids(&self.centroids, 0.75)?,
        ))
    }

    #[cfg(test)]
    fn quantile(&self, p: f64) -> Option<f64> {
        if self.centroids.is_empty() {
            return None;
        }
        if self.centroids.len() == 1 {
            return Some(self.centroids[0].mean);
        }

        let mut centroids = self.centroids.clone();
        sort_centroids_by_mean(&mut centroids);
        quantile_from_sorted_centroids(&centroids, p)
    }

    fn compress_if_needed(&mut self) {
        if self.centroids.len() <= self.max_centroids * 2 {
            return;
        }
        self.compress_to(self.max_centroids);
    }

    fn compress_to(&mut self, target_centroids: usize) {
        if self.centroids.len() <= target_centroids {
            sort_centroids_by_mean(&mut self.centroids);
            return;
        }

        sort_centroids_by_mean(&mut self.centroids);
        let total_weight: f64 = self.centroids.iter().map(|centroid| centroid.weight).sum();
        if total_weight <= f64::EPSILON {
            self.centroids.clear();
            return;
        }

        self.centroids = compress_sorted_centroids(&self.centroids, target_centroids, total_weight);
    }
}

fn compress_sorted_centroids(
    centroids: &[QuantileCentroid],
    target_centroids: usize,
    total_weight: f64,
) -> Vec<QuantileCentroid> {
    let bucket_weight = total_weight / target_centroids as f64;
    let mut compressed: Vec<QuantileCentroid> = Vec::with_capacity(target_centroids);
    let mut progress = CompressionProgress::default();
    let mut bucket_start = 0.0;
    let mut bucket_end = bucket_weight;

    while compressed.len() + 1 < target_centroids && bucket_start < total_weight {
        if let Some(bucket) =
            consume_compression_bucket(centroids, &mut progress, bucket_start, bucket_end)
        {
            compressed.push(bucket);
        }
        bucket_start = bucket_end;
        bucket_end = (bucket_end + bucket_weight).min(total_weight);
    }

    if let Some(remainder) = drain_compression_remainder(centroids, &mut progress) {
        compressed.push(remainder);
    }

    compressed
}

fn consume_compression_bucket(
    centroids: &[QuantileCentroid],
    progress: &mut CompressionProgress,
    bucket_start: f64,
    bucket_end: f64,
) -> Option<QuantileCentroid> {
    let mut weighted_sum = 0.0;
    let mut weight = 0.0;

    while progress.centroid_index < centroids.len() && bucket_start + weight < bucket_end {
        let centroid = centroids[progress.centroid_index];
        let remaining_weight = centroid.weight - progress.centroid_progress;
        let available = (bucket_end - (bucket_start + weight)).max(0.0);
        let take = remaining_weight.min(available);
        if take <= f64::EPSILON {
            break;
        }

        weighted_sum += centroid.mean * take;
        weight += take;
        progress.centroid_progress += take;

        if progress.centroid_progress >= centroid.weight - f64::EPSILON {
            progress.centroid_index += 1;
            progress.centroid_progress = 0.0;
        }
    }

    centroid_from_weighted_sum(weighted_sum, weight)
}

fn drain_compression_remainder(
    centroids: &[QuantileCentroid],
    progress: &mut CompressionProgress,
) -> Option<QuantileCentroid> {
    let mut weighted_sum = 0.0;
    let mut weight = 0.0;

    while progress.centroid_index < centroids.len() {
        let centroid = centroids[progress.centroid_index];
        let take = centroid.weight - progress.centroid_progress;
        if take > f64::EPSILON {
            weighted_sum += centroid.mean * take;
            weight += take;
        }
        progress.centroid_index += 1;
        progress.centroid_progress = 0.0;
    }

    centroid_from_weighted_sum(weighted_sum, weight)
}

fn centroid_from_weighted_sum(weighted_sum: f64, weight: f64) -> Option<QuantileCentroid> {
    (weight > f64::EPSILON).then_some(QuantileCentroid {
        mean: weighted_sum / weight,
        weight,
    })
}

fn sort_centroids_by_mean(centroids: &mut [QuantileCentroid]) {
    centroids.sort_by(|a, b| {
        a.mean
            .partial_cmp(&b.mean)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn quantile_from_sorted_centroids(centroids: &[QuantileCentroid], p: f64) -> Option<f64> {
    if centroids.is_empty() {
        return None;
    }
    if centroids.len() == 1 {
        return Some(centroids[0].mean);
    }
    if centroids
        .iter()
        .all(|centroid| (centroid.weight - 1.0).abs() < f64::EPSILON)
    {
        let values: Vec<f64> = centroids.iter().map(|centroid| centroid.mean).collect();
        return quantile(&values, p);
    }

    let total_weight: f64 = centroids.iter().map(|centroid| centroid.weight).sum();
    if total_weight <= f64::EPSILON {
        return None;
    }

    let target = p.clamp(0.0, 1.0) * total_weight;
    let mut cumulative = 0.0;
    let mut previous_center = 0.0;
    let mut previous_mean = centroids[0].mean;

    for (index, centroid) in centroids.iter().enumerate() {
        let center = cumulative + centroid.weight / 2.0;
        if index == 0 && target <= center {
            return Some(centroid.mean);
        }
        if target <= center {
            let span = (center - previous_center).max(f64::EPSILON);
            let ratio = ((target - previous_center) / span).clamp(0.0, 1.0);
            return Some(previous_mean * (1.0 - ratio) + centroid.mean * ratio);
        }
        previous_center = center;
        previous_mean = centroid.mean;
        cumulative += centroid.weight;
    }

    centroids.last().map(|centroid| centroid.mean)
}

impl OnlineStats {
    pub fn observe(&mut self, value: f64) {
        debug_assert!(
            !value.is_nan(),
            "normalization stats require non-NaN values"
        );
        if !value.is_finite() {
            return;
        }

        self.count += 1;
        if self.count == 1 {
            self.mean = value;
            self.m2 = 0.0;
            self.min = value;
            self.max = value;
            return;
        }

        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    pub fn merge(&mut self, other: Self) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = other;
            return;
        }

        let combined_count = self.count + other.count;
        let delta = other.mean - self.mean;
        self.mean += delta * other.count as f64 / combined_count as f64;
        self.m2 += other.m2
            + delta * delta * self.count as f64 * other.count as f64 / combined_count as f64;
        self.count = combined_count;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn snapshot(self) -> Option<OnlineStatsSnapshot> {
        if self.count == 0 {
            return None;
        }

        // Population stddev, not sample stddev: this describes the observed
        // cohort we normalize against rather than estimating an unseen source.
        let stddev = if self.count > 1 {
            (self.m2 / self.count as f64).sqrt()
        } else {
            0.0
        };

        Some(OnlineStatsSnapshot {
            count: self.count,
            mean: self.mean,
            stddev,
            min: self.min,
            max: self.max,
        })
    }
}

/// Try to build a cohort-relative normalizer from the given strategy and stats.
///
/// Returns `None` when the cohort has too few observations or the derived
/// threshold is degenerate, in which case the caller falls back to the
/// target-threshold normalizer.
fn cohort_normalizer(
    strategy: CohortNormalizationStrategy,
    stats: &CohortStats,
) -> Option<Normalizer> {
    match strategy {
        CohortNormalizationStrategy::CohortMean {
            multiplier,
            min_count,
        } => {
            if stats.count < min_count {
                return None;
            }
            let threshold = stats.mean * multiplier;
            valid_threshold(threshold).then_some(Normalizer::CohortMeanRatio {
                threshold,
                mean: stats.mean,
                multiplier,
                count: stats.count,
            })
        }
        CohortNormalizationStrategy::CohortMeanStddev {
            stddev_multiplier,
            min_count,
        } => {
            if stats.count < min_count {
                return None;
            }
            let threshold = stats.mean + stddev_multiplier * stats.stddev;
            valid_threshold(threshold).then_some(Normalizer::CohortMeanStddevRatio {
                threshold,
                mean: stats.mean,
                stddev: stats.stddev,
                stddev_multiplier,
                count: stats.count,
            })
        }
        CohortNormalizationStrategy::CohortUpperQuartile {
            multiplier,
            min_count,
        } => {
            if stats.count < min_count {
                return None;
            }
            let threshold = stats.q75 * multiplier;
            valid_threshold(threshold).then_some(Normalizer::CohortUpperQuartileRatio {
                threshold,
                q75: stats.q75,
                multiplier,
                count: stats.count,
            })
        }
        CohortNormalizationStrategy::CohortMedianIqr {
            iqr_multiplier,
            min_count,
        } => {
            if stats.count < min_count {
                return None;
            }
            let threshold = stats.q50 + iqr_multiplier * stats.robust_stddev();
            valid_threshold(threshold).then_some(Normalizer::CohortMedianIqrRatio {
                threshold,
                median: stats.q50,
                iqr: stats.iqr(),
                iqr_multiplier,
                count: stats.count,
            })
        }
        // Non-cohort strategies are handled directly by `normalizer_for`.
        CohortNormalizationStrategy::UseTargetThreshold
        | CohortNormalizationStrategy::ScaledTargetThreshold { .. } => None,
    }
}

fn valid_threshold(threshold: f64) -> bool {
    threshold > f64::EPSILON && threshold.is_finite()
}

impl NormalizationPolicy {
    pub fn normalizer_for(
        &self,
        context: &NormalizationContext,
        cohort: NormalizationCohort,
        target_threshold: f64,
    ) -> Normalizer {
        let strategy = self
            .overrides
            .get(&cohort)
            .copied()
            .unwrap_or(CohortNormalizationStrategy::UseTargetThreshold);

        match strategy {
            CohortNormalizationStrategy::UseTargetThreshold => {
                Normalizer::threshold_ratio(target_threshold)
            }
            CohortNormalizationStrategy::ScaledTargetThreshold { multiplier } => {
                Normalizer::threshold_ratio(target_threshold * multiplier)
            }
            _ => context
                .stats_for(cohort)
                .and_then(|stats| cohort_normalizer(strategy, stats))
                .unwrap_or_else(|| Normalizer::threshold_ratio(target_threshold)),
        }
    }
}

impl NormalizationContext {
    pub fn from_report(report: &AnalysisReport) -> Self {
        let mut builder = NormalizationContextBuilder::default();
        builder.observe_report(report);
        builder.build()
    }

    pub fn stats_for(&self, cohort: NormalizationCohort) -> Option<&CohortStats> {
        self.cohorts.get(&cohort)
    }
}

impl NormalizationContextBuilder {
    pub fn observe(&mut self, cohort: NormalizationCohort, value: f64) {
        self.cohorts.entry(cohort).or_default().observe(value);
    }

    pub fn observe_report(&mut self, report: &AnalysisReport) {
        let adjusted_public =
            report.summary.public_function_count + report.summary.macro_export_fn_count;
        if adjusted_public > 0 {
            self.observe(
                NormalizationCohort::CodeEconomy,
                report.summary.function_overhead_ratio,
            );
        }

        for ty in &report.types {
            self.observe(
                NormalizationCohort::StateCardinalityType,
                1.0 + ty.state_cardinality_log2,
            );
        }

        for function in &report.functions {
            if !function.is_test && function.internal_state_cardinality_log2 > 0.0 {
                self.observe(
                    NormalizationCohort::StateCardinalityFunction,
                    1.0 + function.internal_state_cardinality_log2,
                );
            }
            if !function.is_test {
                self.observe(NormalizationCohort::BloatFunction, function.lines as f64);
            }
        }
    }

    pub fn merge(&mut self, other: &Self) {
        for (cohort, accumulator) in &other.cohorts {
            self.cohorts
                .entry(*cohort)
                .or_default()
                .merge(accumulator.clone());
        }
    }

    pub fn build(&self) -> NormalizationContext {
        let cohorts = self
            .cohorts
            .iter()
            .filter_map(|(cohort, accumulator)| {
                accumulator
                    .clone()
                    .snapshot()
                    .map(|snapshot| (*cohort, snapshot))
            })
            .collect();
        NormalizationContext { cohorts }
    }
}

fn quantile(sorted_values: &[f64], quantile: f64) -> Option<f64> {
    if sorted_values.is_empty() {
        return None;
    }
    if sorted_values.len() == 1 {
        return Some(sorted_values[0]);
    }

    let p = quantile.clamp(0.0, 1.0);
    let rank = p * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return Some(sorted_values[lower]);
    }

    let weight = rank - lower as f64;
    Some(sorted_values[lower] * (1.0 - weight) + sorted_values[upper] * weight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{
        AnalysisReport, EntropyMetrics, FunctionMetrics, Summary, TypeKind, TypeMetrics,
    };

    #[test]
    fn test_threshold_ratio_normalizer() {
        let metric = Normalizer::threshold_ratio(10.0).normalize(15.0);

        assert!((metric.raw - 15.0).abs() < f64::EPSILON);
        assert!((metric.normalized - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_normalization_policy_uses_scaled_threshold_override() {
        let mut policy = NormalizationPolicy::default();
        policy.overrides.insert(
            NormalizationCohort::BloatFunction,
            CohortNormalizationStrategy::ScaledTargetThreshold { multiplier: 2.0 },
        );

        let metric = policy
            .normalizer_for(
                &NormalizationContext::default(),
                NormalizationCohort::BloatFunction,
                10.0,
            )
            .normalize(15.0);

        assert!((metric.normalized - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_online_stats_merge_and_snapshot() {
        let mut left = OnlineStats::default();
        left.observe(10.0);
        left.observe(20.0);

        let mut right = OnlineStats::default();
        right.observe(30.0);
        right.observe(40.0);

        left.merge(right);
        let Some(snapshot) = left.snapshot() else {
            panic!("merged stats should be non-empty");
        };

        assert_eq!(snapshot.count, 4);
        assert!((snapshot.mean - 25.0).abs() < 1e-10);
        assert!((snapshot.stddev - 11.180339887498949).abs() < 1e-10);
        assert!((snapshot.min - 10.0).abs() < f64::EPSILON);
        assert!((snapshot.max - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_online_stats_merge_handles_empty_inputs() {
        let mut populated = OnlineStats::default();
        populated.observe(10.0);
        populated.observe(20.0);

        let mut empty_target = OnlineStats::default();
        empty_target.merge(populated);
        let Some(copied) = empty_target.snapshot() else {
            panic!("merge into empty should copy");
        };
        assert_eq!(copied.count, 2);
        assert!((copied.mean - 15.0).abs() < 1e-10);

        let before = copied;
        empty_target.merge(OnlineStats::default());
        let Some(after) = empty_target.snapshot() else {
            panic!("merge with empty should preserve populated stats");
        };
        assert_eq!(after, before);
    }

    #[test]
    fn test_quantile_sketch_approximates_quartiles_after_compression() {
        let mut sketch = QuantileSketch {
            centroids: Vec::new(),
            max_centroids: 8,
        };
        for value in 0..512 {
            sketch.observe(value as f64);
        }

        let Some(q25) = sketch.quantile(0.25) else {
            panic!("q25 should exist")
        };
        let Some(q50) = sketch.quantile(0.50) else {
            panic!("q50 should exist")
        };
        let Some(q75) = sketch.quantile(0.75) else {
            panic!("q75 should exist")
        };
        let expected_range = 511.0;
        // With bucketed compression, the effective error budget scales with
        // roughly half a bucket width across the observed range.
        let tolerance = expected_range / (2.0 * sketch.max_centroids as f64);

        assert!((q25 - 127.75).abs() < tolerance, "unexpected q25 {q25}");
        assert!((q50 - 255.5).abs() < tolerance, "unexpected q50 {q50}");
        assert!((q75 - 383.25).abs() < tolerance, "unexpected q75 {q75}");
    }

    #[test]
    fn test_quantile_sketch_merge_is_consistent_after_compression() {
        let mut left = QuantileSketch {
            centroids: Vec::new(),
            max_centroids: 8,
        };
        for value in 0..256 {
            left.observe(value as f64);
        }

        let mut right = QuantileSketch {
            centroids: Vec::new(),
            max_centroids: 8,
        };
        for value in 256..512 {
            right.observe(value as f64);
        }

        let mut merged = left.clone();
        merged.merge(right.clone());

        let mut direct = QuantileSketch {
            centroids: Vec::new(),
            max_centroids: 8,
        };
        for value in 0..512 {
            direct.observe(value as f64);
        }
        let tolerance = 511.0 / (2.0 * direct.max_centroids as f64);

        for quantile in [0.25, 0.50, 0.75] {
            let Some(merged_value) = merged.quantile(quantile) else {
                panic!("merged quantile should exist");
            };
            let Some(direct_value) = direct.quantile(quantile) else {
                panic!("direct quantile should exist");
            };
            assert!(
                (merged_value - direct_value).abs() < tolerance,
                "merge should preserve quantile {quantile}: merged={merged_value}, direct={direct_value}",
            );
        }
    }

    fn cohort_test_functions() -> Vec<FunctionMetrics> {
        vec![
            FunctionMetrics {
                name: "pub_fn".into(),
                file: "lib.rs".into(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 1,
                lines: 20,
                params: 0,
                nesting_depth: 1,
                cyclomatic: 4,
                mutable_bindings: 0,
                internal_state_cardinality_log2: 0.0,
                assertions: 0,
                meaningful_assertions: 0,
                is_test: false,
                is_pub: true,
            },
            FunctionMetrics {
                name: "stateful_fn".into(),
                file: "lib.rs".into(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 10,
                lines: 12,
                params: 0,
                nesting_depth: 1,
                cyclomatic: 3,
                mutable_bindings: 2,
                internal_state_cardinality_log2: 3.0,
                assertions: 0,
                meaningful_assertions: 0,
                is_test: false,
                is_pub: false,
            },
        ]
    }

    fn cohort_test_report() -> AnalysisReport {
        AnalysisReport {
            analysis_root: None,
            files_analyzed: 1,
            total_lines: 10,
            semantic: None,
            functions: cohort_test_functions(),
            types: vec![TypeMetrics {
                name: "Flags".into(),
                file: "lib.rs".into(),
                module_path: String::new(),
                scope_path: Vec::new(),
                line: 20,
                kind: TypeKind::Struct,
                bool_fields: 2,
                option_fields: 0,
                total_fields: 2,
                state_cardinality: 4,
                state_cardinality_log2: 2.0,
            }],
            entropy: EntropyMetrics {
                total_tokens: 0,
                vocabulary_size: 0,
                entropy_bits: 0.0,
                normalized_entropy: 0.0,
                per_file: Vec::new(),
            },
            duplication: crate::duplication::DuplicationReport {
                functions_fingerprinted: 0,
                exact_duplicates: Vec::new(),
                near_duplicates: Vec::new(),
                duplication_score: 0.0,
            },
            summary: Summary {
                public_function_count: 1,
                production_function_count: 2,
                function_overhead_ratio: 2.0,
                ..Summary::default()
            },
        }
    }

    fn assert_cohort_count(
        context: &NormalizationContext,
        cohort: NormalizationCohort,
        expected: usize,
    ) {
        let Some(stats) = context.stats_for(cohort) else {
            panic!("{cohort:?} stats should exist");
        };
        assert_eq!(stats.count, expected);
    }

    #[test]
    fn test_normalization_context_builder_observes_report_cohorts() {
        let context = NormalizationContext::from_report(&cohort_test_report());

        assert_cohort_count(&context, NormalizationCohort::CodeEconomy, 1);
        assert_cohort_count(&context, NormalizationCohort::StateCardinalityType, 1);
        assert_cohort_count(&context, NormalizationCohort::StateCardinalityFunction, 1);
        assert_cohort_count(&context, NormalizationCohort::BloatFunction, 2);

        let Some(bloat) = context.stats_for(NormalizationCohort::BloatFunction) else {
            panic!("bloat stats should exist");
        };
        // Observations are lines=20 and lines=12 (raw line counts).
        assert!((bloat.q25 - 14.0).abs() < 1e-10);
        assert!((bloat.q50 - 16.0).abs() < 1e-10);
        assert!((bloat.q75 - 18.0).abs() < 1e-10);
    }

    #[test]
    fn test_normalization_policy_can_use_cohort_mean_stddev() {
        let mut builder = NormalizationContextBuilder::default();
        for value in [10.0, 20.0, 30.0, 40.0, 50.0] {
            builder.observe(NormalizationCohort::BloatFunction, value);
        }
        let context = builder.build();

        let mut policy = NormalizationPolicy::default();
        policy.overrides.insert(
            NormalizationCohort::BloatFunction,
            CohortNormalizationStrategy::CohortMeanStddev {
                stddev_multiplier: 1.0,
                min_count: 5,
            },
        );

        let normalizer = policy.normalizer_for(&context, NormalizationCohort::BloatFunction, 10.0);
        let metric = normalizer.normalize(35.0);

        match normalizer {
            Normalizer::CohortMeanStddevRatio {
                mean,
                stddev,
                threshold,
                count,
                ..
            } => {
                assert_eq!(count, 5);
                assert!((mean - 30.0).abs() < 1e-10);
                assert!((stddev - 14.142135623730951).abs() < 1e-10);
                assert!((threshold - 44.14213562373095).abs() < 1e-10);
            }
            other => panic!("expected cohort mean/stddev normalizer, got {other:?}"),
        }

        assert!((metric.normalized - (35.0 / 44.14213562373095)).abs() < 1e-10);
    }

    #[test]
    fn test_context_builder_merge_matches_direct_observation() {
        let mut left = NormalizationContextBuilder::default();
        for value in [10.0, 20.0] {
            left.observe(NormalizationCohort::BloatFunction, value);
        }

        let mut right = NormalizationContextBuilder::default();
        for value in [30.0, 40.0] {
            right.observe(NormalizationCohort::BloatFunction, value);
        }

        let mut merged = left.clone();
        merged.merge(&right);

        let mut direct = NormalizationContextBuilder::default();
        for value in [10.0, 20.0, 30.0, 40.0] {
            direct.observe(NormalizationCohort::BloatFunction, value);
        }

        let Some(merged_stats) = merged
            .build()
            .stats_for(NormalizationCohort::BloatFunction)
            .copied()
        else {
            panic!("merged stats should exist");
        };
        let Some(direct_stats) = direct
            .build()
            .stats_for(NormalizationCohort::BloatFunction)
            .copied()
        else {
            panic!("direct stats should exist");
        };

        assert_eq!(merged_stats, direct_stats);
    }

    #[test]
    fn test_normalization_policy_can_use_upper_quartile() {
        let mut builder = NormalizationContextBuilder::default();
        for value in [10.0, 10.0, 10.0, 10.0, 1000.0] {
            builder.observe(NormalizationCohort::BloatFunction, value);
        }
        let context = builder.build();

        let mut policy = NormalizationPolicy::default();
        policy.overrides.insert(
            NormalizationCohort::BloatFunction,
            CohortNormalizationStrategy::CohortUpperQuartile {
                multiplier: 1.0,
                min_count: 5,
            },
        );

        let normalizer = policy.normalizer_for(&context, NormalizationCohort::BloatFunction, 10.0);
        let metric = normalizer.normalize(10.0);

        match normalizer {
            Normalizer::CohortUpperQuartileRatio {
                q75,
                threshold,
                count,
                ..
            } => {
                assert_eq!(count, 5);
                assert!((q75 - 10.0).abs() < 1e-10);
                assert!((threshold - 10.0).abs() < 1e-10);
            }
            other => panic!("expected upper quartile normalizer, got {other:?}"),
        }

        assert!((metric.normalized - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_normalization_policy_can_use_median_iqr() {
        let mut builder = NormalizationContextBuilder::default();
        for value in [10.0, 11.0, 12.0, 13.0, 1000.0] {
            builder.observe(NormalizationCohort::BloatFunction, value);
        }
        let context = builder.build();

        let mut policy = NormalizationPolicy::default();
        policy.overrides.insert(
            NormalizationCohort::BloatFunction,
            CohortNormalizationStrategy::CohortMedianIqr {
                iqr_multiplier: 1.0,
                min_count: 5,
            },
        );

        let normalizer = policy.normalizer_for(&context, NormalizationCohort::BloatFunction, 10.0);

        match normalizer {
            Normalizer::CohortMedianIqrRatio {
                median,
                iqr,
                threshold,
                count,
                ..
            } => {
                assert_eq!(count, 5);
                assert!((median - 12.0).abs() < 1e-10);
                assert!((iqr - 2.0).abs() < 1e-10);
                assert!((threshold - (12.0 + 2.0 / 1.349)).abs() < 1e-10);
            }
            other => panic!("expected median/IQR normalizer, got {other:?}"),
        }
    }

    #[test]
    fn test_cohort_relative_normalizer_falls_back_when_context_is_too_small() {
        let mut builder = NormalizationContextBuilder::default();
        builder.observe(NormalizationCohort::CodeEconomy, 8.0);
        let context = builder.build();

        let mut policy = NormalizationPolicy::default();
        policy.overrides.insert(
            NormalizationCohort::CodeEconomy,
            CohortNormalizationStrategy::CohortMean {
                multiplier: 1.0,
                min_count: 2,
            },
        );

        let normalizer = policy.normalizer_for(&context, NormalizationCohort::CodeEconomy, 5.0);
        assert_eq!(normalizer, Normalizer::ThresholdRatio { threshold: 5.0 });
    }
}

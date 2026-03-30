//! Aggregation policies for item-level utilities and cross-dimension scalarization.
//!
//! This separates two mathematical concerns:
//!
//! - artifact aggregation: how a dimension combines many item scores
//! - objective scalarization: how the final composite combines dimensions
//!
//! The defaults preserve current behavior, but the policy types make future
//! tail-aware or corpus-aware aggregation explicit.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Policies for aggregating item-level utility scores into a dimension score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactAggregation {
    /// Current default: geometric mean in utility space.
    #[default]
    GeometricMeanScore,
    /// Arithmetic mean in utility space.
    ArithmeticMeanScore,
    /// Aggregate on losses using an Lp / power mean.
    ///
    /// This policy is intended for `p >= 1.0`, where larger values punish bad
    /// tails more strongly. Values below `1.0` are clamped to `1.0`.
    PowerMeanLoss { p: f64 },
    /// Aggregate on losses with an average plus tail-risk term.
    ///
    /// Large `tail_weight` values can saturate the resulting score at `0.0`
    /// when the tail is severe enough.
    MeanPlusCvarLoss { alpha: f64, tail_weight: f64 },
    /// Aggregate item losses hierarchically: first within each file using
    /// size-weighted means, then across files using size-weighted means.
    ///
    /// This reduces split/merge gaming by making file-level loss less sensitive
    /// to the raw number of items in that file. The weighting is intentionally
    /// asymmetric: items are weighted by `size_weight(size_i)` within a file,
    /// while files are weighted by `size_weight(sum(size_i))` across files.
    HierarchicalFileWeightedMeanLoss {
        #[serde(default)]
        size_weighting: ArtifactSizeWeighting,
    },
}

/// Policies for combining dimension scores into a final composite.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectiveScalarization {
    /// Current default: geometric mean in utility space.
    #[default]
    GeometricMeanScore,
    /// Arithmetic mean in utility space.
    ArithmeticMeanScore,
}

/// Weighting functions for size-aware artifact aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ArtifactSizeWeighting {
    #[serde(rename = "uniform")]
    UniformWeight,
    #[serde(rename = "linear")]
    LinearWeight,
    #[serde(rename = "sqrt")]
    #[default]
    SqrtWeight,
    #[serde(rename = "log2")]
    Log2Weight,
}

/// One scored item eligible for artifact aggregation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArtifactAggregationObservation<'a> {
    pub file: &'a str,
    pub score: f64,
    pub size_hint: f64,
}

pub fn aggregate_artifact_scores(policy: ArtifactAggregation, scores: &[f64]) -> f64 {
    if scores.is_empty() {
        return 1.0;
    }

    match policy {
        ArtifactAggregation::GeometricMeanScore => geometric_mean(scores),
        ArtifactAggregation::ArithmeticMeanScore => arithmetic_mean(scores),
        ArtifactAggregation::PowerMeanLoss { p } => {
            let losses: Vec<f64> = scores.iter().map(|score| 1.0 - score).collect();
            let aggregated_loss = power_mean(&losses, p);
            (1.0 - aggregated_loss).clamp(0.0, 1.0)
        }
        ArtifactAggregation::MeanPlusCvarLoss { alpha, tail_weight } => {
            let losses: Vec<f64> = scores.iter().map(|score| 1.0 - score).collect();
            let mean_loss = arithmetic_mean(&losses);
            let tail_loss = cvar_loss(&losses, alpha);
            (1.0 - (mean_loss + tail_weight * tail_loss)).clamp(0.0, 1.0)
        }
        ArtifactAggregation::HierarchicalFileWeightedMeanLoss { .. } => {
            let observations: Vec<_> = scores
                .iter()
                .map(|&score| ArtifactAggregationObservation {
                    file: "<ungrouped>",
                    score,
                    size_hint: 1.0,
                })
                .collect();
            aggregate_artifact_observations(policy, &observations)
        }
    }
}

pub fn aggregate_artifact_observations(
    policy: ArtifactAggregation,
    observations: &[ArtifactAggregationObservation<'_>],
) -> f64 {
    if observations.is_empty() {
        return 1.0;
    }

    match policy {
        ArtifactAggregation::GeometricMeanScore
        | ArtifactAggregation::ArithmeticMeanScore
        | ArtifactAggregation::PowerMeanLoss { .. }
        | ArtifactAggregation::MeanPlusCvarLoss { .. } => {
            let scores: Vec<f64> = observations
                .iter()
                .map(|observation| observation.score)
                .collect();
            aggregate_artifact_scores(policy, &scores)
        }
        ArtifactAggregation::HierarchicalFileWeightedMeanLoss { size_weighting } => {
            hierarchical_file_weighted_mean_loss(observations, size_weighting)
        }
    }
}

pub fn scalarize_dimension_scores(policy: ObjectiveScalarization, scores: &[f64]) -> f64 {
    if scores.is_empty() {
        return 1.0;
    }

    match policy {
        ObjectiveScalarization::GeometricMeanScore => geometric_mean(scores),
        ObjectiveScalarization::ArithmeticMeanScore => arithmetic_mean(scores),
    }
}

fn arithmetic_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 1.0;
    }

    values.iter().sum::<f64>() / values.len() as f64
}

fn geometric_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 1.0;
    }

    if values.contains(&0.0) {
        return 0.0;
    }

    let log_sum: f64 = values.iter().map(|value| value.ln()).sum();
    (log_sum / values.len() as f64).exp()
}

fn power_mean(losses: &[f64], p: f64) -> f64 {
    if losses.is_empty() {
        return 0.0;
    }

    let exponent = p.max(1.0);
    let mean_power =
        losses.iter().map(|loss| loss.powf(exponent)).sum::<f64>() / losses.len() as f64;
    mean_power.powf(1.0 / exponent)
}

fn cvar_loss(losses: &[f64], alpha: f64) -> f64 {
    if losses.is_empty() {
        return 0.0;
    }

    let mut sorted = losses.to_vec();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let tail_fraction = alpha.clamp(f64::EPSILON, 1.0);
    let tail_count = ((sorted.len() as f64) * tail_fraction).ceil() as usize;
    let take = tail_count.clamp(1, sorted.len());
    arithmetic_mean(&sorted[..take])
}

fn hierarchical_file_weighted_mean_loss(
    observations: &[ArtifactAggregationObservation<'_>],
    size_weighting: ArtifactSizeWeighting,
) -> f64 {
    let mut files: BTreeMap<&str, Vec<ArtifactAggregationObservation<'_>>> = BTreeMap::new();
    for observation in observations {
        files
            .entry(observation.file)
            .or_default()
            .push(*observation);
    }

    let mut file_losses: Vec<f64> = Vec::with_capacity(files.len());
    let mut file_weights: Vec<f64> = Vec::with_capacity(files.len());

    for file_observations in files.values() {
        let item_losses: Vec<f64> = file_observations
            .iter()
            .map(|observation| 1.0 - observation.score)
            .collect();
        let item_weights: Vec<f64> = file_observations
            .iter()
            .map(|observation| size_weight(size_weighting, observation.size_hint))
            .collect();
        let file_loss = weighted_mean(&item_losses, &item_weights);
        let total_size: f64 = file_observations
            .iter()
            .map(|observation| observation.size_hint.max(1.0))
            .sum();

        file_losses.push(file_loss);
        file_weights.push(size_weight(size_weighting, total_size));
    }

    (1.0 - weighted_mean(&file_losses, &file_weights)).clamp(0.0, 1.0)
}

fn weighted_mean(values: &[f64], weights: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let total_weight: f64 = weights.iter().sum();
    if total_weight <= f64::EPSILON {
        return arithmetic_mean(values);
    }

    values
        .iter()
        .zip(weights)
        .map(|(value, weight)| value * weight)
        .sum::<f64>()
        / total_weight
}

fn size_weight(size_weighting: ArtifactSizeWeighting, size_hint: f64) -> f64 {
    let size = size_hint.max(1.0);
    match size_weighting {
        ArtifactSizeWeighting::UniformWeight => 1.0,
        ArtifactSizeWeighting::LinearWeight => size,
        ArtifactSizeWeighting::SqrtWeight => size.sqrt(),
        ArtifactSizeWeighting::Log2Weight => size.log2().max(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_artifact_aggregation_matches_geometric_mean() {
        let scores = [1.0, 0.5];
        let aggregated =
            aggregate_artifact_scores(ArtifactAggregation::GeometricMeanScore, &scores);

        assert!((aggregated - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-10);
    }

    #[test]
    fn test_power_mean_loss_penalizes_bad_tail() {
        let scores = [1.0, 0.2];
        let arithmetic =
            aggregate_artifact_scores(ArtifactAggregation::ArithmeticMeanScore, &scores);
        let power_mean =
            aggregate_artifact_scores(ArtifactAggregation::PowerMeanLoss { p: 2.0 }, &scores);

        assert!(power_mean < arithmetic);
    }

    #[test]
    fn test_power_mean_loss_clamps_subunit_exponents() {
        let scores = [1.0, 0.2];
        let arithmetic =
            aggregate_artifact_scores(ArtifactAggregation::ArithmeticMeanScore, &scores);
        let power_mean =
            aggregate_artifact_scores(ArtifactAggregation::PowerMeanLoss { p: 0.5 }, &scores);

        assert!((power_mean - arithmetic).abs() < 1e-10);
    }

    #[test]
    fn test_mean_plus_cvar_loss_penalizes_worst_tail() {
        let scores = [1.0, 1.0, 0.1];
        let arithmetic =
            aggregate_artifact_scores(ArtifactAggregation::ArithmeticMeanScore, &scores);
        let tail = aggregate_artifact_scores(
            ArtifactAggregation::MeanPlusCvarLoss {
                alpha: 0.34,
                tail_weight: 0.5,
            },
            &scores,
        );

        assert!(tail < arithmetic);
    }

    #[test]
    fn test_hierarchical_file_weighted_mean_loss_respects_file_grouping() {
        let observations = [
            ArtifactAggregationObservation {
                file: "src/a.rs",
                score: 0.2,
                size_hint: 100.0,
            },
            ArtifactAggregationObservation {
                file: "src/a.rs",
                score: 0.8,
                size_hint: 20.0,
            },
            ArtifactAggregationObservation {
                file: "src/b.rs",
                score: 0.9,
                size_hint: 10.0,
            },
        ];

        let score = aggregate_artifact_observations(
            ArtifactAggregation::HierarchicalFileWeightedMeanLoss {
                size_weighting: ArtifactSizeWeighting::LinearWeight,
            },
            &observations,
        );

        let file_a_loss = ((1.0 - 0.2) * 100.0 + (1.0 - 0.8) * 20.0) / 120.0;
        let file_b_loss = 1.0 - 0.9;
        let expected = 1.0 - ((file_a_loss * 120.0 + file_b_loss * 10.0) / 130.0);

        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_hierarchical_file_weighted_mean_loss_can_be_less_gameable_than_plain_mean() {
        let scores = [0.2, 0.8, 0.9];
        let plain = aggregate_artifact_scores(ArtifactAggregation::ArithmeticMeanScore, &scores);
        let hierarchical = aggregate_artifact_observations(
            ArtifactAggregation::HierarchicalFileWeightedMeanLoss {
                size_weighting: ArtifactSizeWeighting::LinearWeight,
            },
            &[
                ArtifactAggregationObservation {
                    file: "src/a.rs",
                    score: 0.2,
                    size_hint: 100.0,
                },
                ArtifactAggregationObservation {
                    file: "src/a.rs",
                    score: 0.8,
                    size_hint: 20.0,
                },
                ArtifactAggregationObservation {
                    file: "src/b.rs",
                    score: 0.9,
                    size_hint: 10.0,
                },
            ],
        );

        assert!(hierarchical < plain);
    }

    #[test]
    fn test_artifact_aggregation_is_monotone_under_regressions() {
        let baseline = [
            ArtifactAggregationObservation {
                file: "src/lib.rs",
                score: 1.0,
                size_hint: 10.0,
            },
            ArtifactAggregationObservation {
                file: "src/lib.rs",
                score: 1.0,
                size_hint: 20.0,
            },
            ArtifactAggregationObservation {
                file: "src/extra.rs",
                score: 1.0,
                size_hint: 5.0,
            },
        ];
        let mild_regression = [
            ArtifactAggregationObservation {
                score: 1.0,
                ..baseline[0]
            },
            ArtifactAggregationObservation {
                score: 0.8,
                ..baseline[1]
            },
            ArtifactAggregationObservation {
                score: 1.0,
                ..baseline[2]
            },
        ];
        let severe_regression = [
            ArtifactAggregationObservation {
                score: 1.0,
                ..baseline[0]
            },
            ArtifactAggregationObservation {
                score: 0.4,
                ..baseline[1]
            },
            ArtifactAggregationObservation {
                score: 1.0,
                ..baseline[2]
            },
        ];

        for policy in [
            ArtifactAggregation::GeometricMeanScore,
            ArtifactAggregation::ArithmeticMeanScore,
            ArtifactAggregation::PowerMeanLoss { p: 2.0 },
            ArtifactAggregation::MeanPlusCvarLoss {
                alpha: 0.34,
                tail_weight: 0.5,
            },
            ArtifactAggregation::HierarchicalFileWeightedMeanLoss {
                size_weighting: ArtifactSizeWeighting::SqrtWeight,
            },
        ] {
            let baseline_score = aggregate_artifact_observations(policy, &baseline);
            let mild_score = aggregate_artifact_observations(policy, &mild_regression);
            let severe_score = aggregate_artifact_observations(policy, &severe_regression);

            assert!(
                baseline_score >= mild_score - 1e-12,
                "{policy:?} should be monotone"
            );
            assert!(
                mild_score >= severe_score - 1e-12,
                "{policy:?} should be monotone"
            );
        }
    }

    #[test]
    fn test_objective_scalarization_is_monotone_under_dimension_regressions() {
        let baseline = [1.0, 1.0, 1.0];
        let mild_regression = [1.0, 0.8, 1.0];
        let severe_regression = [1.0, 0.4, 1.0];

        for policy in [
            ObjectiveScalarization::GeometricMeanScore,
            ObjectiveScalarization::ArithmeticMeanScore,
        ] {
            let baseline_score = scalarize_dimension_scores(policy, &baseline);
            let mild_score = scalarize_dimension_scores(policy, &mild_regression);
            let severe_score = scalarize_dimension_scores(policy, &severe_regression);

            assert!(
                baseline_score >= mild_score - 1e-12,
                "{policy:?} should be monotone"
            );
            assert!(
                mild_score >= severe_score - 1e-12,
                "{policy:?} should be monotone"
            );
        }
    }
}

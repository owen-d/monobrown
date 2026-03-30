//! Calibration helpers for shaping normalized metrics into bounded scores.
//!
//! This makes the scoring pipeline explicit:
//!
//! raw metric -> normalized metric -> calibrated score
//!
//! The default calibrator preserves the current reciprocal-decay behavior, but
//! the policy surface can now vary shaping independently from normalization.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::normalization::{NormalizationCohort, NormalizedMetric};

/// Practical tolerance for score comparisons near `1.0`.
pub const SCORE_TOLERANCE: f64 = 1e-12;

/// A normalized metric after calibration into a bounded utility score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CalibratedMetric {
    /// Original metric in source units.
    pub raw: f64,
    /// Dimensionless normalized value.
    pub normalized: f64,
    /// Bounded utility score in `[0.0, 1.0]`.
    pub score: f64,
}

impl CalibratedMetric {
    /// Whether this metric contributes any non-zero loss.
    pub fn has_loss(self) -> bool {
        self.score < 1.0 - SCORE_TOLERANCE
    }

    /// The bounded loss corresponding to this score.
    pub fn loss(self) -> f64 {
        1.0 - self.score
    }
}

/// Supported shaping functions for normalized metrics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Calibrator {
    /// Values at or below the target map to `1.0`; larger values decay as `1/x`.
    #[default]
    CappedReciprocalScore,
    /// A thresholdless decay where each `half_life` normalized units halves the score.
    ExponentialDecayScore { half_life: f64 },
    /// Values at or below the target map to `1.0`; values at or above `zero_at`
    /// map to `0.0`; the interval in between decays linearly.
    LinearDecayScore { zero_at: f64 },
    /// Stretched exponential (Weibull) decay where higher `shape` concentrates
    /// penalty at the high end of the metric range.
    ///
    /// `score = exp(-ln2 * (normalized / half_life)^shape)`
    ///
    /// # Why this shape?
    ///
    /// Plain exponential decay (`shape = 1.0`) penalizes each unit of normalized
    /// metric equally in proportional terms -- a function going from 1->2 lines
    /// loses as much *fraction* of its score as one going from 50->100 lines.
    /// This means the largest absolute loss happens at the low end, where code
    /// is short and the penalty is least actionable.
    ///
    /// With `shape > 1`, the marginal penalty per unit *increases* with the
    /// metric value. Low values (short functions, small state) are nearly free;
    /// high values (bloated functions, explosive state) pay disproportionately.
    /// This matches the intuition that loss should correlate with actionability:
    /// a 10-line function is essentially unavoidable, but a 70-line function is
    /// a design choice you can fix.
    ///
    /// `shape = 2` gives quadratic growth in the exponent (moderate shift).
    /// `shape = 3` gives cubic growth (strong shift toward penalizing the top).
    /// As `shape -> infinity`, the curve approaches a step function -- recreating
    /// the compliance cliff -- so values of 2-3 are the practical sweet spot.
    StretchedExponentialDecay { half_life: f64, shape: f64 },
}

impl Calibrator {
    pub fn calibrate(self, metric: NormalizedMetric) -> CalibratedMetric {
        let score = match self {
            Self::CappedReciprocalScore => {
                if metric.normalized <= 1.0 {
                    1.0
                } else {
                    1.0 / metric.normalized
                }
            }
            Self::ExponentialDecayScore { half_life } => {
                if half_life <= f64::EPSILON {
                    0.0
                } else {
                    let normalized = metric.normalized.max(0.0);
                    (-std::f64::consts::LN_2 * normalized / half_life).exp()
                }
            }
            Self::LinearDecayScore { zero_at } => {
                if metric.normalized <= 1.0 {
                    1.0
                } else if zero_at <= 1.0 || metric.normalized >= zero_at {
                    0.0
                } else {
                    1.0 - ((metric.normalized - 1.0) / (zero_at - 1.0))
                }
            }
            Self::StretchedExponentialDecay { half_life, shape } => {
                if half_life <= f64::EPSILON {
                    0.0
                } else {
                    let normalized = metric.normalized.max(0.0);
                    let x = normalized / half_life;
                    (-std::f64::consts::LN_2 * x.powf(shape)).exp()
                }
            }
        }
        .clamp(0.0, 1.0);

        CalibratedMetric {
            raw: metric.raw,
            normalized: metric.normalized,
            score,
        }
    }
}

/// Cohort-aware calibration policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CalibrationPolicy {
    /// Optional per-cohort overrides. Missing cohorts fall back to the default
    /// calibrator, which preserves the existing reciprocal-decay behavior.
    #[serde(default)]
    pub overrides: BTreeMap<NormalizationCohort, Calibrator>,
}

#[cfg(test)]
impl CalibrationPolicy {
    pub(crate) fn calibrator_for(&self, cohort: NormalizationCohort) -> Calibrator {
        self.overrides.get(&cohort).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capped_reciprocal_calibrator() {
        let metric = NormalizedMetric {
            raw: 20.0,
            normalized: 2.0,
        };
        let calibrated = Calibrator::CappedReciprocalScore.calibrate(metric);

        assert!((calibrated.score - 0.5).abs() < f64::EPSILON);
        assert!((calibrated.loss() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_linear_decay_calibrator() {
        let metric = NormalizedMetric {
            raw: 15.0,
            normalized: 1.5,
        };
        let calibrated = Calibrator::LinearDecayScore { zero_at: 2.0 }.calibrate(metric);

        assert!((calibrated.score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_exponential_decay_calibrator() {
        let origin = NormalizedMetric {
            raw: 0.0,
            normalized: 0.0,
        };
        let midpoint = NormalizedMetric {
            raw: 8.0,
            normalized: 1.0,
        };
        let farther = NormalizedMetric {
            raw: 16.0,
            normalized: 2.0,
        };
        let default_half_life =
            Calibrator::ExponentialDecayScore { half_life: 1.0 }.calibrate(midpoint);
        let doubled_half_life =
            Calibrator::ExponentialDecayScore { half_life: 2.0 }.calibrate(midpoint);
        let origin_calibrated =
            Calibrator::ExponentialDecayScore { half_life: 1.0 }.calibrate(origin);
        let farther_calibrated =
            Calibrator::ExponentialDecayScore { half_life: 1.0 }.calibrate(farther);

        assert!((origin_calibrated.score - 1.0).abs() < 1e-12);
        assert!((default_half_life.score - 0.5).abs() < 1e-12);
        assert!(doubled_half_life.score > default_half_life.score);
        assert!(farther_calibrated.score < default_half_life.score);
    }

    #[test]
    fn test_stretched_exponential_decay_calibrator() {
        let half_life = 1.0;
        let shape = 2.0;
        let cal = Calibrator::StretchedExponentialDecay { half_life, shape };

        // score = 1.0 at normalized = 0
        let origin = NormalizedMetric {
            raw: 0.0,
            normalized: 0.0,
        };
        assert!((cal.calibrate(origin).score - 1.0).abs() < 1e-12);

        // score = 0.5 at normalized = half_life (for any shape)
        let midpoint = NormalizedMetric {
            raw: 8.0,
            normalized: half_life,
        };
        assert!((cal.calibrate(midpoint).score - 0.5).abs() < 1e-12);

        // Also true for shape=3
        let cal3 = Calibrator::StretchedExponentialDecay {
            half_life,
            shape: 3.0,
        };
        assert!((cal3.calibrate(midpoint).score - 0.5).abs() < 1e-12);

        // shape=1.0 matches ExponentialDecayScore
        let cal1 = Calibrator::StretchedExponentialDecay {
            half_life,
            shape: 1.0,
        };
        let exp = Calibrator::ExponentialDecayScore { half_life };
        for normalized in [0.0, 0.5, 1.0, 1.5, 2.0, 3.0] {
            let metric = NormalizedMetric {
                raw: normalized * 8.0,
                normalized,
            };
            assert!(
                (cal1.calibrate(metric).score - exp.calibrate(metric).score).abs() < 1e-12,
                "shape=1 should match ExponentialDecayScore at normalized={normalized}"
            );
        }

        // Higher shape gives higher scores at low normalized values
        // and lower scores at high normalized values
        let low = NormalizedMetric {
            raw: 2.0,
            normalized: 0.3,
        };
        let high = NormalizedMetric {
            raw: 16.0,
            normalized: 2.0,
        };
        let shape1 = Calibrator::StretchedExponentialDecay {
            half_life,
            shape: 1.0,
        };
        let shape3 = Calibrator::StretchedExponentialDecay {
            half_life,
            shape: 3.0,
        };
        assert!(
            shape3.calibrate(low).score > shape1.calibrate(low).score,
            "higher shape should give higher score at low normalized"
        );
        assert!(
            shape3.calibrate(high).score < shape1.calibrate(high).score,
            "higher shape should give lower score at high normalized"
        );
    }

    #[test]
    fn test_calibration_policy_override() {
        let mut policy = CalibrationPolicy::default();
        policy.overrides.insert(
            NormalizationCohort::BloatFunction,
            Calibrator::LinearDecayScore { zero_at: 3.0 },
        );

        assert_eq!(
            policy.calibrator_for(NormalizationCohort::BloatFunction),
            Calibrator::LinearDecayScore { zero_at: 3.0 }
        );
        assert_eq!(
            policy.calibrator_for(NormalizationCohort::CodeEconomy),
            Calibrator::CappedReciprocalScore
        );
    }

    #[test]
    fn test_has_loss_uses_score_tolerance() {
        let almost_perfect = CalibratedMetric {
            raw: 0.0,
            normalized: 1.0,
            score: 1.0 - 1e-14,
        };
        let meaningfully_below = CalibratedMetric {
            raw: 0.0,
            normalized: 1.0,
            score: 1.0 - 1e-10,
        };

        assert!(!almost_perfect.has_loss());
        assert!(meaningfully_below.has_loss());
    }
}

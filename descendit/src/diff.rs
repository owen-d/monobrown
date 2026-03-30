//! Diff mode and loss vector output for comparing analysis snapshots.
//!
//! Compares two `Summary` structs (before/after) and produces a delta report
//! showing which metrics improved, regressed, or stayed the same. Can also
//! convert summaries/diffs into raw metric vectors, and calibrated compliance
//! reports into structured loss vector output for agent optimization loops.

use std::cmp::Ordering;

use serde::Serialize;

use crate::metrics::{SemanticSummary, Summary};

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

/// A single metric delta with directionality.
#[derive(Debug, Clone, Serialize)]
pub struct MetricDelta {
    pub name: String,
    pub before: f64,
    pub after: f64,
    pub delta: f64,
    /// "lower" means lower values are better; "higher" means higher values are better.
    pub direction: Direction,
    /// Did this metric improve, regress, or stay the same?
    pub assessment: Assessment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Lower,
    Higher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    Improved,
    Regressed,
    Unchanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub deltas: Vec<MetricDelta>,
    /// Number of metrics that improved.
    pub improved: usize,
    /// Number of metrics that regressed.
    pub regressed: usize,
    /// Number of metrics unchanged.
    pub unchanged: usize,
}

// ---------------------------------------------------------------------------
// Vector output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct LossEntry {
    pub name: String,
    pub value: LossValueOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MetricValue {
    Number(f64),
    Count(u64),
    Flag(bool),
}

pub type LossValueOut = MetricValue;
pub type RawMetricValueOut = MetricValue;

#[derive(Debug, Clone, Serialize)]
pub struct LossVectorOut {
    pub entries: Vec<LossEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawMetricEntry {
    pub name: String,
    pub value: MetricValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawMetricVectorOut {
    pub entries: Vec<RawMetricEntry>,
}

impl From<RawMetricEntry> for LossEntry {
    fn from(entry: RawMetricEntry) -> Self {
        Self {
            name: entry.name,
            value: entry.value,
            notes: entry.notes,
        }
    }
}

impl From<RawMetricVectorOut> for LossVectorOut {
    fn from(vector: RawMetricVectorOut) -> Self {
        Self {
            entries: vector.entries.into_iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Diff computation
// ---------------------------------------------------------------------------

/// Build a `(name, before, after, direction)` tuple for each Summary field.
///
/// Each metric is annotated with its directionality (lower-is-better or
/// higher-is-better) and assessed as improved, regressed, or unchanged.
macro_rules! comparison_pairs {
    ($before:expr, $after:expr, [
        $( $field:ident => $dir:ident ),+ $(,)?
    ]) => {
        vec![
            $( (stringify!($field), $before.$field as f64, $after.$field as f64, Direction::$dir) ),+
        ]
    };
}

fn metric_comparison_pairs(
    before: &Summary,
    after: &Summary,
) -> Vec<(&'static str, f64, f64, Direction)> {
    comparison_pairs!(before, after, [
        max_function_lines            => Lower,
        mean_function_lines           => Lower,
        functions_over_70_lines       => Lower,
        max_nesting_depth             => Lower,
        mean_nesting_depth            => Lower,
        max_cyclomatic                => Lower,
        mean_cyclomatic               => Lower,
        max_params                    => Lower,
        total_mutable_bindings        => Lower,
        total_bool_fields             => Lower,
        total_option_fields           => Lower,
        max_state_cardinality_log2    => Lower,
        functions_under_2_assertions  => Lower,
        mean_assertions_per_function  => Higher,
        mean_meaningful_assertions_per_function => Higher,
        test_density                  => Higher,
        duplication_score             => Lower,
        function_overhead_ratio       => Lower,
    ])
}

fn assess_delta(delta: f64, dir: Direction) -> Assessment {
    match (dir, delta.partial_cmp(&0.0)) {
        (_, Some(Ordering::Equal)) => Assessment::Unchanged,
        (Direction::Lower, Some(Ordering::Less)) => Assessment::Improved,
        (Direction::Lower, Some(Ordering::Greater)) => Assessment::Regressed,
        (Direction::Higher, Some(Ordering::Greater)) => Assessment::Improved,
        (Direction::Higher, Some(Ordering::Less)) => Assessment::Regressed,
        _ => Assessment::Unchanged,
    }
}

pub fn diff_summaries(
    before: &Summary,
    after: &Summary,
    before_semantic: Option<&SemanticSummary>,
    after_semantic: Option<&SemanticSummary>,
) -> DiffReport {
    let mut deltas: Vec<MetricDelta> = metric_comparison_pairs(before, after)
        .into_iter()
        .map(|(name, bv, av, dir)| {
            let delta = av - bv;
            MetricDelta {
                name: name.to_string(),
                before: bv,
                after: av,
                delta,
                direction: dir,
                assessment: assess_delta(delta, dir),
            }
        })
        .collect();

    if before_semantic.is_some() || after_semantic.is_some() {
        let bv = before_semantic.map_or(0.0, |s| s.coupling_density);
        let av = after_semantic.map_or(0.0, |s| s.coupling_density);
        let delta = av - bv;
        deltas.push(MetricDelta {
            name: "coupling_density".to_string(),
            before: bv,
            after: av,
            delta,
            direction: Direction::Lower,
            assessment: assess_delta(delta, Direction::Lower),
        });
    }

    let improved = deltas
        .iter()
        .filter(|d| d.assessment == Assessment::Improved)
        .count();
    let regressed = deltas
        .iter()
        .filter(|d| d.assessment == Assessment::Regressed)
        .count();
    let unchanged = deltas
        .iter()
        .filter(|d| d.assessment == Assessment::Unchanged)
        .count();

    DiffReport {
        deltas,
        improved,
        regressed,
        unchanged,
    }
}

// ---------------------------------------------------------------------------
// Loss vector conversion
// ---------------------------------------------------------------------------

/// Convert a diff report into a raw signed delta vector.
///
/// Each metric becomes a signed raw delta:
/// positive = bad (regression), negative = good (improvement). This preserves
/// metric directionality but does not normalize scales across dimensions.
pub fn diff_to_raw_delta_vector(diff: &DiffReport) -> RawMetricVectorOut {
    let mut entries: Vec<RawMetricEntry> = diff
        .deltas
        .iter()
        .map(|d| {
            // Keep a common sign convention without calibrating raw units.
            let signed_delta = match d.direction {
                Direction::Lower => d.delta,
                Direction::Higher => -d.delta,
            };
            let notes = Some(format!(
                "{}: {:.4} -> {:.4}",
                match d.assessment {
                    Assessment::Improved => "improved",
                    Assessment::Regressed => "regressed",
                    Assessment::Unchanged => "unchanged",
                },
                d.before,
                d.after,
            ));
            RawMetricEntry {
                name: d.name.clone(),
                value: RawMetricValueOut::Number(signed_delta),
                notes,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    RawMetricVectorOut { entries }
}

/// Convert a summary into a raw metric snapshot vector.
///
/// These entries preserve the original metric units and are useful for
/// inspection, but they are not calibrated into a common loss scale.
fn raw_metric_entries(summary: &Summary) -> Vec<RawMetricEntry> {
    vec![
        RawMetricEntry {
            name: "duplication_score".into(),
            value: RawMetricValueOut::Number(summary.duplication_score),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "function_overhead_ratio".into(),
            value: RawMetricValueOut::Number(summary.function_overhead_ratio),
            notes: Some("lower is better; non-test functions / public functions".into()),
        },
        RawMetricEntry {
            name: "functions_over_70_lines".into(),
            value: RawMetricValueOut::Count(summary.functions_over_70_lines as u64),
            notes: Some("lower is better; structural violation".into()),
        },
        RawMetricEntry {
            name: "max_cyclomatic".into(),
            value: RawMetricValueOut::Count(summary.max_cyclomatic as u64),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "max_function_lines".into(),
            value: RawMetricValueOut::Count(summary.max_function_lines as u64),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "max_nesting_depth".into(),
            value: RawMetricValueOut::Count(summary.max_nesting_depth as u64),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "max_state_cardinality_log2".into(),
            value: RawMetricValueOut::Number(summary.max_state_cardinality_log2),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "mean_assertions_per_function".into(),
            value: RawMetricValueOut::Number(summary.mean_assertions_per_function),
            notes: Some("higher is better; target >= 2.0".into()),
        },
        RawMetricEntry {
            name: "mean_cyclomatic".into(),
            value: RawMetricValueOut::Number(summary.mean_cyclomatic),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "mean_function_lines".into(),
            value: RawMetricValueOut::Number(summary.mean_function_lines),
            notes: Some("lower is better".into()),
        },
        RawMetricEntry {
            name: "total_bool_fields".into(),
            value: RawMetricValueOut::Count(summary.total_bool_fields as u64),
            notes: Some("lower is better; consider enums".into()),
        },
        RawMetricEntry {
            name: "total_mutable_bindings".into(),
            value: RawMetricValueOut::Count(summary.total_mutable_bindings as u64),
            notes: Some("lower is better".into()),
        },
    ]
}

pub fn summary_to_raw_metric_vector(
    summary: &Summary,
    semantic: Option<&SemanticSummary>,
) -> RawMetricVectorOut {
    let mut entries = raw_metric_entries(summary);
    if let Some(sem) = semantic {
        entries.push(RawMetricEntry {
            name: "coupling_density".into(),
            value: RawMetricValueOut::Number(sem.coupling_density),
            notes: Some("lower is better; cross-module call graph density".into()),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    RawMetricVectorOut { entries }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Build a summary with all fields at their default (zero) values.
    fn zero_summary() -> Summary {
        Summary::default()
    }

    #[test]
    fn test_diff_unchanged() {
        let s = zero_summary();
        let report = diff_summaries(&s, &s, None, None);

        assert!(
            report.improved == 0,
            "expected 0 improved, got {}",
            report.improved
        );
        assert!(
            report.regressed == 0,
            "expected 0 regressed, got {}",
            report.regressed
        );
        assert_eq!(report.unchanged, report.deltas.len());

        for d in &report.deltas {
            assert_eq!(
                d.assessment,
                Assessment::Unchanged,
                "metric {} should be unchanged",
                d.name
            );
            assert!(
                d.delta.abs() < f64::EPSILON,
                "metric {} should have zero delta, got {}",
                d.name,
                d.delta,
            );
        }
    }

    #[test]
    fn test_diff_improvement() {
        let before = Summary {
            max_function_lines: 100,
            ..zero_summary()
        };
        let after = Summary {
            max_function_lines: 50,
            ..zero_summary()
        };

        let report = diff_summaries(&before, &after, None, None);
        let delta = report
            .deltas
            .iter()
            .find(|d| d.name == "max_function_lines")
            .unwrap();

        assert_eq!(delta.assessment, Assessment::Improved);
        assert_eq!(delta.direction, Direction::Lower);
        assert!((delta.delta - (-50.0)).abs() < f64::EPSILON);
        assert!(report.improved >= 1);
    }

    #[test]
    fn test_diff_regression() {
        let before = Summary {
            duplication_score: 0.1,
            ..zero_summary()
        };
        let after = Summary {
            duplication_score: 0.5,
            ..zero_summary()
        };

        let report = diff_summaries(&before, &after, None, None);
        let delta = report
            .deltas
            .iter()
            .find(|d| d.name == "duplication_score")
            .unwrap();

        assert_eq!(delta.assessment, Assessment::Regressed);
        assert_eq!(delta.direction, Direction::Lower);
        assert!((delta.delta - 0.4).abs() < 1e-10);
        assert!(report.regressed >= 1);
    }

    #[test]
    fn test_diff_higher_is_better() {
        let before = Summary {
            mean_assertions_per_function: 1.0,
            ..zero_summary()
        };
        let after = Summary {
            mean_assertions_per_function: 2.5,
            ..zero_summary()
        };

        let report = diff_summaries(&before, &after, None, None);
        let delta = report
            .deltas
            .iter()
            .find(|d| d.name == "mean_assertions_per_function")
            .unwrap();

        assert_eq!(delta.assessment, Assessment::Improved);
        assert_eq!(delta.direction, Direction::Higher);
        assert!((delta.delta - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_loss_vector_sorted() {
        let s = Summary {
            max_function_lines: 42,
            mean_cyclomatic: 3.0,
            duplication_score: 0.1,
            mean_assertions_per_function: 1.5,
            ..zero_summary()
        };
        let lv = summary_to_raw_metric_vector(&s, None);

        let names: Vec<&str> = lv.entries.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "loss vector entries must be sorted by name");
    }
}

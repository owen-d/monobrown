#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::collections::HashSet;

use support::benchmark_cases::{all_benchmark_cases, analyze_case};

#[test]
fn test_benchmark_case_names_are_unique() {
    let mut seen = HashSet::new();
    for case in all_benchmark_cases() {
        assert!(
            seen.insert(case.name),
            "duplicate benchmark case name: {}",
            case.name,
        );
    }
}

#[test]
fn test_benchmark_cases_analyze_and_change_behavior() {
    for case in all_benchmark_cases() {
        let (before_analysis, _before_compliance, after_analysis, _after_compliance) =
            analyze_case(case);

        let before_summary =
            serde_json::to_string(&before_analysis.summary).expect("serialize before summary");
        let after_summary =
            serde_json::to_string(&after_analysis.summary).expect("serialize after summary");

        assert_ne!(
            before_summary, after_summary,
            "benchmark case '{}' should change the analyzed summary",
            case.name,
        );
    }
}

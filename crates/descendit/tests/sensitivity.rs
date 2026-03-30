//! Integration tests verifying that descendit metrics detect real improvements and regressions.
//!
//! For each test scenario, we create two versions of code (before/after), analyze both,
//! diff them, and verify specific metrics moved in the expected direction.

#![allow(clippy::unwrap_used, clippy::too_many_lines, clippy::expect_used)]

use std::fs;
use std::path::Path;

use descendit::{
    Assessment, CompliancePolicy, ComplianceReport, LossValueOut, analyze_path,
    compliance_to_loss_vector, compute_compliance, diff_summaries,
};

fn write_source(source: &str, dir: &Path) {
    fs::write(dir.join("lib.rs"), source).unwrap();
}

fn assert_metric_assessment(diff: &descendit::DiffReport, metric: &str, expected: Assessment) {
    let delta = diff
        .deltas
        .iter()
        .find(|d| d.name == metric)
        .unwrap_or_else(|| panic!("metric '{metric}' not found in diff"));
    assert_eq!(
        delta.assessment, expected,
        "metric '{metric}': expected {expected:?}, got {delta:?}"
    );
}

// ---------------------------------------------------------------------------
// 1. Splitting a long function improves function-length metrics
// ---------------------------------------------------------------------------

#[test]
fn test_splitting_long_function_improves_metrics() {
    // Before: one function with 80+ lines (over the 70-line structural limit).
    let mut before_src = String::from(
        "fn process_data(items: &[u32]) -> Vec<u32> {\n    let mut result = Vec::new();\n",
    );
    for i in 0..78 {
        before_src.push_str(&format!("    let _v{i} = {i};\n"));
    }
    before_src.push_str("    result\n}\n");

    // After: same logic split into three functions of ~25 lines each.
    let mut after_src = String::new();

    after_src
        .push_str("fn step_one(items: &[u32]) -> Vec<u32> {\n    let mut result = Vec::new();\n");
    for i in 0..25 {
        after_src.push_str(&format!("    let _v{i} = {i};\n"));
    }
    after_src.push_str("    result\n}\n\n");

    after_src
        .push_str("fn step_two(items: &[u32]) -> Vec<u32> {\n    let mut result = Vec::new();\n");
    for i in 25..50 {
        after_src.push_str(&format!("    let _v{i} = {i};\n"));
    }
    after_src.push_str("    result\n}\n\n");

    after_src
        .push_str("fn step_three(items: &[u32]) -> Vec<u32> {\n    let mut result = Vec::new();\n");
    for i in 50..78 {
        after_src.push_str(&format!("    let _v{i} = {i};\n"));
    }
    after_src.push_str("    result\n}\n");

    let dir = tempfile::tempdir().unwrap();

    write_source(&before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(&after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    assert_metric_assessment(&diff, "max_function_lines", Assessment::Improved);
    assert_metric_assessment(&diff, "functions_over_70_lines", Assessment::Improved);
}

// ---------------------------------------------------------------------------
// 2. Adding assertions improves assertion-density metrics
// ---------------------------------------------------------------------------

#[test]
fn test_adding_assertions_improves_metrics() {
    let before_src = r#"
fn validate(x: i32) -> i32 {
    let y = x + 1;
    let z = y * 2;
    if z > 100 {
        z - 50
    } else {
        z
    }
}

fn transform(a: i32, b: i32) -> i32 {
    let sum = a + b;
    let product = a * b;
    if sum > product {
        sum
    } else {
        product
    }
}
"#;

    let after_src = r#"
fn validate(x: i32) -> i32 {
    assert!(x >= 0);
    assert!(x < 1_000_000);
    let y = x + 1;
    let z = y * 2;
    if z > 100 {
        z - 50
    } else {
        z
    }
}

fn transform(a: i32, b: i32) -> i32 {
    assert!(a > 0);
    assert!(b > 0);
    let sum = a + b;
    let product = a * b;
    if sum > product {
        sum
    } else {
        product
    }
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    assert_metric_assessment(&diff, "mean_assertions_per_function", Assessment::Improved);
    assert_metric_assessment(&diff, "functions_under_2_assertions", Assessment::Improved);
}

// ---------------------------------------------------------------------------
// 3. Replacing bools with an enum improves state-cardinality metrics
// ---------------------------------------------------------------------------

#[test]
fn test_replacing_bools_with_enum_improves_metrics() {
    // Before: struct with 3 bool fields (2^3 = 8 states).
    let before_src = r#"
struct Config {
    enabled: bool,
    verbose: bool,
    debug: bool,
    name: String,
}

fn use_config(c: &Config) -> i32 {
    if c.enabled { 1 } else { 0 }
}
"#;

    // After: replaced with an enum (4 variants ~ 2^2 states).
    let after_src = r#"
enum Mode {
    Disabled,
    Normal,
    Verbose,
    Debug,
}

struct Config {
    mode: Mode,
    name: String,
}

fn use_config(c: &Config) -> i32 {
    match c.mode {
        Mode::Disabled => 0,
        Mode::Normal => 1,
        Mode::Verbose => 2,
        Mode::Debug => 3,
    }
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    assert_metric_assessment(&diff, "total_bool_fields", Assessment::Improved);
    assert_metric_assessment(&diff, "max_state_cardinality_log2", Assessment::Improved);
}

// ---------------------------------------------------------------------------
// 4. Adding deeply nested code regresses nesting and complexity metrics
// ---------------------------------------------------------------------------

#[test]
fn test_adding_deeply_nested_code_regresses() {
    let before_src = r#"
fn process(items: &[i32]) -> i32 {
    let mut total = 0;
    let a = items.len();
    let b = a + 1;
    let c = b + 2;
    total = a + b + c;
    total
}
"#;

    // After: same intent but deeply nested with ifs inside loops.
    let after_src = r#"
fn process(items: &[i32]) -> i32 {
    let mut total = 0;
    for i in 0..items.len() {
        if items[i] > 0 {
            for j in 0..items[i] {
                if j % 2 == 0 {
                    if j > 5 {
                        total = total + j;
                    }
                }
            }
        }
    }
    total
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    assert_metric_assessment(&diff, "max_nesting_depth", Assessment::Regressed);
    assert_metric_assessment(&diff, "max_cyclomatic", Assessment::Regressed);
}

// ---------------------------------------------------------------------------
// 5. Introducing duplication regresses the duplication score
// ---------------------------------------------------------------------------

#[test]
fn test_introducing_duplication_regresses() {
    // Before: one function with a non-trivial body (10+ shape tokens).
    let before_src = r#"
fn compute(x: i32) -> i32 {
    let a = x + 1;
    let b = a * 2;
    let c = b - 3;
    if c > 10 {
        return c * 4;
    }
    let d = c + 5;
    d
}
"#;

    // After: the same function copy-pasted with a different name.
    let after_src = r#"
fn compute(x: i32) -> i32 {
    let a = x + 1;
    let b = a * 2;
    let c = b - 3;
    if c > 10 {
        return c * 4;
    }
    let d = c + 5;
    d
}

fn compute_copy(y: i32) -> i32 {
    let a = y + 1;
    let b = a * 2;
    let c = b - 3;
    if c > 10 {
        return c * 4;
    }
    let d = c + 5;
    d
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    assert_metric_assessment(&diff, "duplication_score", Assessment::Regressed);
}

// ---------------------------------------------------------------------------
// 6. Identical code produces no metric changes
// ---------------------------------------------------------------------------

#[test]
fn test_metric_stability_on_no_change() {
    let source = r#"
struct Point {
    x: f64,
    y: f64,
}

fn distance(a: &Point, b: &Point) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    assert!(dx.is_finite());
    assert!(dy.is_finite());
    (dx * dx + dy * dy).sqrt()
}

fn midpoint(a: &Point, b: &Point) -> Point {
    assert!(a.x.is_finite());
    Point {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    }
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(source, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    // Analyze the same source again (no changes).
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    assert_eq!(
        diff.improved, 0,
        "no metrics should improve on identical code"
    );
    assert_eq!(
        diff.regressed, 0,
        "no metrics should regress on identical code"
    );
    assert_eq!(
        diff.unchanged,
        diff.deltas.len(),
        "all metrics should be unchanged"
    );

    for delta in &diff.deltas {
        assert_eq!(
            delta.assessment,
            Assessment::Unchanged,
            "metric '{}' should be unchanged, got {:?}",
            delta.name,
            delta.assessment,
        );
    }
}

// ===========================================================================
// Compliance-level sensitivity tests
// ===========================================================================

fn compliance_for(source: &str) -> ComplianceReport {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), source).unwrap();
    let report = analyze_path(dir.path()).unwrap();
    compute_compliance(&report, &CompliancePolicy::default())
}

// ---------------------------------------------------------------------------
// 7. Adding meaningful assertions improves assertion metrics at the diff level
// ---------------------------------------------------------------------------

#[test]
fn test_compliance_improves_when_adding_meaningful_assertions() {
    // assertion_density is parked as a soft dimension, but the underlying
    // diff-level metrics (mean_assertions_per_function, functions_under_2_assertions)
    // still detect assertion improvements.
    let before_src = r#"
fn validate_input(x: i32, y: i32) -> i32 {
    let sum = x + y;
    let product = x * y;
    let diff = x - y;
    let ratio = if y != 0 { x / y } else { 0 };
    if sum > 100 {
        product + diff
    } else {
        ratio + sum
    }
}

fn transform_data(values: &[i32]) -> Vec<i32> {
    let mut output = Vec::new();
    let len = values.len();
    let threshold = len / 2;
    let scale = if len > 10 { 2 } else { 1 };
    for v in values {
        if *v > threshold as i32 {
            output.push(v * scale);
        } else {
            output.push(*v);
        }
    }
    output
}

fn merge_results(a: i32, b: i32, c: i32) -> i32 {
    let ab = a + b;
    let bc = b + c;
    let ac = a + c;
    let max_pair = if ab > bc { ab } else { bc };
    let result = if max_pair > ac {
        max_pair
    } else {
        ac
    };
    result
}
"#;

    let after_src = r#"
fn validate_input(x: i32, y: i32) -> i32 {
    assert!(x > i32::MIN / 2);
    assert!(y > i32::MIN / 2);
    let sum = x + y;
    let product = x * y;
    let diff = x - y;
    let ratio = if y != 0 { x / y } else { 0 };
    if sum > 100 {
        product + diff
    } else {
        ratio + sum
    }
}

fn transform_data(values: &[i32]) -> Vec<i32> {
    assert!(!values.is_empty());
    assert!(values.len() < 10_000);
    let mut output = Vec::new();
    let len = values.len();
    let threshold = len / 2;
    let scale = if len > 10 { 2 } else { 1 };
    for v in values {
        if *v > threshold as i32 {
            output.push(v * scale);
        } else {
            output.push(*v);
        }
    }
    output
}

fn merge_results(a: i32, b: i32, c: i32) -> i32 {
    assert!(a >= 0);
    assert!(b >= 0);
    assert!(c >= 0);
    let ab = a + b;
    let bc = b + c;
    let ac = a + c;
    let max_pair = if ab > bc { ab } else { bc };
    let result = if max_pair > ac {
        max_pair
    } else {
        ac
    };
    result
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    // assertion_density is parked from soft dimensions, but the underlying
    // summary metrics still track assertion improvements.
    assert_metric_assessment(&diff, "mean_assertions_per_function", Assessment::Improved);
    assert_metric_assessment(&diff, "functions_under_2_assertions", Assessment::Improved);
}

// ---------------------------------------------------------------------------
// 9. Fixing multiple dimensions improves composite score significantly
// ---------------------------------------------------------------------------

/// Build a `#[cfg(test)]` module with a test function containing `n` assertions.
///
/// This ensures `test_density` compliance is non-zero so the geometric mean
/// (composite score) is not dragged to zero by a missing test suite.
fn test_module_source(assertion_count: usize) -> String {
    let mut s = String::new();
    s.push_str("#[cfg(test)]\nmod tests {\n    #[test]\n    fn test_invariants() {\n");
    for i in 0..assertion_count {
        s.push_str(&format!("        assert!({i} < {});\n", i + 1));
    }
    s.push_str("    }\n}\n");
    s
}

#[test]
fn test_composite_score_improves_with_balanced_changes() {
    // Before: code that violates multiple soft dimensions.
    // - Bools instead of enums (high state cardinality)
    // - Low semantic density (many filler lines)
    let mut before_src = String::new();

    // A struct with many bool fields (high state cardinality).
    before_src.push_str(
        "struct Config {\n\
         \x20   enabled: bool,\n\
         \x20   verbose: bool,\n\
         \x20   debug: bool,\n\
         \x20   cached: bool,\n\
         \x20   compressed: bool,\n\
         \x20   encrypted: bool,\n\
         \x20   name: String,\n\
         }\n\n",
    );

    // A function with many mutable bindings.
    before_src.push_str("fn process(items: &[i32], _config: &Config) -> Vec<i32> {\n");
    before_src.push_str("    let mut output = Vec::new();\n");
    before_src.push_str("    let mut acc = 0;\n");
    before_src.push_str("    let mut count = 0;\n");
    before_src.push_str("    let mut temp = 0;\n");
    before_src.push_str("    let mut flag = false;\n");
    for i in 0..30 {
        before_src.push_str(&format!("    let _local_{i} = {i};\n"));
    }
    before_src.push_str("    for item in items {\n");
    before_src.push_str("        if *item > 0 {\n");
    before_src.push_str("            for j in 0..*item {\n");
    before_src.push_str("                if j % 2 == 0 {\n");
    before_src.push_str("                    if j > 5 {\n");
    before_src.push_str("                        output.push(j);\n");
    before_src.push_str("                    }\n");
    before_src.push_str("                }\n");
    before_src.push_str("            }\n");
    before_src.push_str("        }\n");
    before_src.push_str("    }\n");
    for i in 30..60 {
        before_src.push_str(&format!("    let _tail_{i} = {i};\n"));
    }
    before_src.push_str("    output\n}\n\n");
    before_src.push_str(&test_module_source(20));

    // After: all violations fixed.
    let mut after_src = String::from(
        r#"
enum Mode {
    Disabled,
    Normal,
    Verbose,
    Debug,
    Cached,
    Compressed,
    Encrypted,
}

struct Config {
    mode: Mode,
    name: String,
}

fn filter_positive(items: &[i32]) -> Vec<i32> {
    assert!(!items.is_empty());
    assert!(items.len() < 100_000);
    let mut result = Vec::new();
    for item in items {
        if *item > 0 {
            result.push(*item);
        }
    }
    result
}

fn apply_threshold(values: &[i32], threshold: i32) -> Vec<i32> {
    assert!(threshold >= 0);
    assert!(!values.is_empty());
    values.iter().filter(|v| **v > threshold).copied().collect()
}

fn process(items: &[i32], _config: &Config) -> Vec<i32> {
    assert!(!items.is_empty());
    assert!(items.len() < 100_000);
    let positives = filter_positive(items);
    apply_threshold(&positives, 5)
}
"#,
    );
    after_src.push_str(&test_module_source(20));

    let before_compliance = compliance_for(&before_src);
    let after_compliance = compliance_for(&after_src);

    // The composite reflects soft dimensions (duplication, code_economy,
    // state_cardinality, bloat). The "before" code has
    // high state cardinality, dragging the score down.
    assert!(
        after_compliance.composite_score > before_compliance.composite_score,
        "composite should improve: before={}, after={}",
        before_compliance.composite_score,
        after_compliance.composite_score,
    );
}

// ---------------------------------------------------------------------------
// 10. Compliance loss vector reflects improvement (loss decreases)
// ---------------------------------------------------------------------------

#[test]
fn test_compliance_loss_vector_reflects_improvement() {
    // Before: violations across multiple soft dimensions (bool-heavy struct,
    // many mutable bindings).
    let mut before_src = String::new();

    before_src.push_str(
        "struct State {\n\
         \x20   active: bool,\n\
         \x20   ready: bool,\n\
         \x20   locked: bool,\n\
         \x20   stale: bool,\n\
         \x20   name: String,\n\
         }\n\n",
    );

    before_src.push_str("fn handle(data: &[i32], _state: &State) -> Vec<i32> {\n");
    before_src.push_str("    let mut out = Vec::new();\n");
    before_src.push_str("    let mut acc = 0;\n");
    before_src.push_str("    let mut count = 0;\n");
    before_src.push_str("    let mut temp = 0;\n");
    before_src.push_str("    let mut flag = false;\n");
    for i in 0..50 {
        before_src.push_str(&format!("    let _x_{i} = {i};\n"));
    }
    before_src.push_str("    for d in data {\n");
    before_src.push_str("        if *d > 0 {\n");
    before_src.push_str("            for k in 0..*d {\n");
    before_src.push_str("                if k > 3 {\n");
    before_src.push_str("                    if k % 2 == 0 {\n");
    before_src.push_str("                        out.push(k);\n");
    before_src.push_str("                    }\n");
    before_src.push_str("                }\n");
    before_src.push_str("            }\n");
    before_src.push_str("        }\n");
    before_src.push_str("    }\n");
    for i in 50..70 {
        before_src.push_str(&format!("    let _y_{i} = {i};\n"));
    }
    before_src.push_str("    out\n}\n\n");
    before_src.push_str(&test_module_source(20));

    // After: violations fixed (short fns, flat nesting, enum, fewer mutable bindings).
    let mut after_src = String::from(
        r#"
enum Phase {
    Inactive,
    Active,
    Ready,
    Locked,
}

struct State {
    phase: Phase,
    name: String,
}

fn preprocess(data: &[i32]) -> Vec<i32> {
    assert!(!data.is_empty());
    assert!(data.len() < 50_000);
    data.iter().filter(|d| **d > 0).copied().collect()
}

fn handle(data: &[i32], _state: &State) -> Vec<i32> {
    assert!(!data.is_empty());
    assert!(data.len() < 50_000);
    let positive = preprocess(data);
    positive.iter().filter(|v| **v > 3 && **v % 2 == 0).copied().collect()
}
"#,
    );
    after_src.push_str(&test_module_source(20));

    let before_compliance = compliance_for(&before_src);
    let after_compliance = compliance_for(&after_src);

    let before_loss = compliance_to_loss_vector(&before_compliance);
    let after_loss = compliance_to_loss_vector(&after_compliance);

    // The loss vector should have 6 entries: 5 soft dimensions + 1 composite.
    assert_eq!(
        before_loss.entries.len(),
        6,
        "expected 6 loss entries, got {}",
        before_loss.entries.len(),
    );

    // Find the composite entry in both loss vectors.
    let before_composite = before_loss
        .entries
        .iter()
        .find(|e| e.name == "compliance_composite")
        .expect("missing compliance_composite in before loss vector");
    let after_composite = after_loss
        .entries
        .iter()
        .find(|e| e.name == "compliance_composite")
        .expect("missing compliance_composite in after loss vector");

    let before_val = match before_composite.value {
        LossValueOut::Number(v) => v,
        _ => panic!("compliance_composite should be a Number"),
    };
    let after_val = match after_composite.value {
        LossValueOut::Number(v) => v,
        _ => panic!("compliance_composite should be a Number"),
    };

    // Loss = 1.0 - compliance, so lower loss = better.
    assert!(
        after_val < before_val,
        "compliance_composite loss should decrease: before={before_val}, after={after_val}",
    );
}

// ---------------------------------------------------------------------------
// 11. Replacing functions with macro_rules! does NOT improve overhead ratio
// ---------------------------------------------------------------------------

#[test]
fn test_macro_hiding_does_not_improve_overhead_ratio() {
    // Before: 1 pub fn + 2 private helper fns => overhead = 3/1 = 3.0
    let before_src = r#"
pub fn api_entry(x: i32) -> i32 {
    let y = helper_a(x);
    helper_b(y)
}

fn helper_a(x: i32) -> i32 {
    if x > 0 { x * 2 } else { x }
}

fn helper_b(x: i32) -> i32 {
    match x {
        0 => 1,
        _ => x + 1,
    }
}
"#;

    // After: same logic, but helpers replaced with macro_rules! containing control flow.
    // Without macro counting, overhead would drop to 1/1 = 1.0 (gaming vector).
    // With macro counting, overhead = (1 fn + 2 macros) / 1 pub = 3.0 (no improvement).
    let after_src = r#"
pub fn api_entry(x: i32) -> i32 {
    let y = helper_a!(x);
    helper_b!(y)
}

macro_rules! helper_a {
    ($x:expr) => {
        if $x > 0 { $x * 2 } else { $x }
    };
}

macro_rules! helper_b {
    ($x:expr) => {
        match $x {
            0 => 1,
            _ => $x + 1,
        }
    };
}
"#;

    let dir = tempfile::tempdir().unwrap();

    write_source(before_src, dir.path());
    let before_report = analyze_path(dir.path()).unwrap();

    write_source(after_src, dir.path());
    let after_report = analyze_path(dir.path()).unwrap();

    let diff = diff_summaries(&before_report.summary, &after_report.summary, None, None);

    // The overhead ratio must NOT improve when replacing functions with macros.
    assert_metric_assessment(&diff, "function_overhead_ratio", Assessment::Unchanged);
}

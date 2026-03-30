#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]
//! Counterfactual invariant tests: verify that removing an artifact and
//! recomputing from scratch produces a delta consistent with the loss
//! function's attribution model.

use std::fs;

use descendit::{
    AnalysisReport, CallEdge, CompliancePolicy, ComplianceReport, SemanticData, SemanticOverlay,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn compliance_for(policy: &CompliancePolicy, source: &str) -> (AnalysisReport, ComplianceReport) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("lib.rs"), source).expect("write");
    let report = descendit::analyze_path(dir.path()).expect("analyze");
    let compliance = descendit::compute_compliance(&report, policy);
    (report, compliance)
}

fn compliance_with_semantic(
    policy: &CompliancePolicy,
    source: &str,
    overlay: &SemanticOverlay,
) -> (AnalysisReport, ComplianceReport) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("lib.rs"), source).expect("write");
    let report = descendit::analyze_path(dir.path()).expect("analyze");
    let compliance = descendit::compute_compliance_with_semantic(&report, policy, Some(overlay));
    (report, compliance)
}

fn dimension_score(report: &ComplianceReport, name: &str) -> f64 {
    report
        .soft_dimensions
        .iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("missing dimension {name}"))
        .score
}

fn assert_approx(actual: f64, expected: f64, tolerance: f64, context: &str) {
    assert!(
        (actual - expected).abs() < tolerance,
        "{context}: expected {expected:.6}, got {actual:.6} (diff {:.2e})",
        (actual - expected).abs(),
    );
}

/// Build a function body with the given structure, ensuring >= 5 shape tokens.
/// Each variant produces a distinct AST shape so functions are not accidentally
/// duplicated across tests.
fn unique_function(name: &str, variant: usize, lines: usize) -> String {
    unique_function_vis(name, variant, lines, false)
}

fn pub_function(name: &str, variant: usize, lines: usize) -> String {
    unique_function_vis(name, variant, lines, true)
}

fn unique_function_vis(name: &str, variant: usize, lines: usize, is_pub: bool) -> String {
    let vis = if is_pub { "pub " } else { "" };
    let mut src = format!("{vis}fn {name}(x: i32) -> i32 {{\n");
    src.push_str("    let mut total = 0;\n");
    match variant {
        0 => {
            src.push_str("    if x > 0 {\n");
            for i in 0..lines.saturating_sub(5) {
                src.push_str(&format!("        total += {i};\n"));
            }
            src.push_str("    }\n");
        }
        1 => {
            src.push_str("    while total < x {\n");
            for i in 0..lines.saturating_sub(5) {
                src.push_str(&format!("        total += {i} + 1;\n"));
            }
            src.push_str("    }\n");
        }
        2 => {
            src.push_str("    for i in 0..x {\n");
            for i in 0..lines.saturating_sub(5) {
                src.push_str(&format!("        total += i + {i};\n"));
            }
            src.push_str("    }\n");
        }
        3 => {
            src.push_str("    loop {\n");
            src.push_str("        if total >= x { break; }\n");
            for i in 0..lines.saturating_sub(6) {
                src.push_str(&format!("        total += {i} + 2;\n"));
            }
            src.push_str("    }\n");
        }
        _ => {
            src.push_str("    match x {\n");
            src.push_str("        0 => total = 1,\n");
            for i in 0..lines.saturating_sub(7) {
                src.push_str(&format!("        {i} => total += {i},\n"));
            }
            src.push_str("        _ => total = x,\n");
            src.push_str("    }\n");
        }
    }
    src.push_str("    total\n}\n\n");
    src
}

/// Build a function with the exact same structure as `unique_function` variant 0
/// but with a different name. Two calls with different names produce exact duplicates.
fn duplicate_function(name: &str, lines: usize) -> String {
    let mut src = format!("fn {name}(x: i32) -> i32 {{\n");
    src.push_str("    let mut total = 0;\n");
    src.push_str("    if x > 0 {\n");
    for i in 0..lines.saturating_sub(5) {
        src.push_str(&format!("        total += {i};\n"));
    }
    src.push_str("    }\n");
    src.push_str("    total\n}\n\n");
    src
}

// ---------------------------------------------------------------------------
// Test 1: Duplication — pair of 2
// ---------------------------------------------------------------------------

#[test]
fn test_duplication_counterfactual_matches_recomputation() {
    let policy = CompliancePolicy::default();

    // 2 exact duplicates (dup_a, dup_b) + 2 unique functions = 4 total.
    let full_source = format!(
        "{}{}{}{}",
        duplicate_function("dup_a", 10),
        duplicate_function("dup_b", 10),
        unique_function("unique_c", 1, 10),
        unique_function("unique_d", 2, 10),
    );

    let (_, full_compliance) = compliance_for(&policy, &full_source);
    let full_dup_score = dimension_score(&full_compliance, "duplication");
    // duplication_score = 2/4 = 0.5, so score = 1 - 0.5 = 0.5.
    assert_approx(full_dup_score, 0.5, 1e-6, "full corpus duplication score");

    // Remove dup_a: now dup_b is no longer duplicated => 0 dups / 3 total => score = 1.0.
    let without_a = format!(
        "{}{}{}",
        duplicate_function("dup_b", 10),
        unique_function("unique_c", 1, 10),
        unique_function("unique_d", 2, 10),
    );
    let (_, without_a_compliance) = compliance_for(&policy, &without_a);
    let without_a_score = dimension_score(&without_a_compliance, "duplication");
    assert_approx(
        without_a_score,
        1.0,
        1e-6,
        "removing one of a duplicate pair should eliminate duplication",
    );

    // Delta = new_score - old_score = 1.0 - 0.5 = 0.5.
    let delta_a = without_a_score - full_dup_score;
    assert_approx(delta_a, 0.5, 1e-6, "delta from removing dup_a");

    // Remove dup_b instead: same result by symmetry.
    let without_b = format!(
        "{}{}{}",
        duplicate_function("dup_a", 10),
        unique_function("unique_c", 1, 10),
        unique_function("unique_d", 2, 10),
    );
    let (_, without_b_compliance) = compliance_for(&policy, &without_b);
    let without_b_score = dimension_score(&without_b_compliance, "duplication");
    assert_approx(
        without_b_score,
        1.0,
        1e-6,
        "removing the other duplicate should also eliminate duplication",
    );

    let delta_b = without_b_score - full_dup_score;
    assert_approx(delta_b, delta_a, 1e-6, "symmetry: delta_a == delta_b");

    // Remove a unique function: duplication_score = 2/3, score = 1 - 2/3 = 1/3.
    let without_unique = format!(
        "{}{}{}",
        duplicate_function("dup_a", 10),
        duplicate_function("dup_b", 10),
        unique_function("unique_c", 1, 10),
    );
    let (_, without_unique_compliance) = compliance_for(&policy, &without_unique);
    let without_unique_score = dimension_score(&without_unique_compliance, "duplication");
    assert_approx(
        without_unique_score,
        1.0 / 3.0,
        1e-6,
        "removing a unique fn should worsen dup ratio",
    );

    // Delta from removing a unique function is negative (score gets worse).
    let delta_unique = without_unique_score - full_dup_score;
    assert!(
        delta_unique < 0.0,
        "removing a unique function should worsen duplication score, got delta={delta_unique}",
    );
}

// ---------------------------------------------------------------------------
// Test 2: Duplication — group of 3
// ---------------------------------------------------------------------------

#[test]
fn test_duplication_counterfactual_group_of_three() {
    let policy = CompliancePolicy::default();

    // 3 exact duplicates + 1 unique = 4 total.
    // duplication_score = 3/4 = 0.75, score = 1 - 0.75 = 0.25.
    let full_source = format!(
        "{}{}{}{}",
        duplicate_function("trip_a", 10),
        duplicate_function("trip_b", 10),
        duplicate_function("trip_c", 10),
        unique_function("unique_d", 1, 10),
    );

    let (_, full_compliance) = compliance_for(&policy, &full_source);
    let full_dup_score = dimension_score(&full_compliance, "duplication");
    assert_approx(full_dup_score, 0.25, 1e-6, "group-of-3 full dup score");

    // Remove one duplicate: remaining 2 are still duplicates.
    // duplication_score = 2/3, score = 1 - 2/3 ≈ 0.333.
    let without_one = format!(
        "{}{}{}",
        duplicate_function("trip_b", 10),
        duplicate_function("trip_c", 10),
        unique_function("unique_d", 1, 10),
    );
    let (_, without_one_compliance) = compliance_for(&policy, &without_one);
    let without_one_score = dimension_score(&without_one_compliance, "duplication");
    assert_approx(
        without_one_score,
        1.0 / 3.0,
        1e-6,
        "removing one from group-of-3 leaves a dup pair",
    );

    // The score is better (higher) than the original, but only slightly.
    // 0.333 > 0.25, so the delta is positive but small.
    let delta = without_one_score - full_dup_score;
    assert!(
        delta > 0.0,
        "removing one from a group-of-3 should improve score, got delta={delta}",
    );
    assert_approx(
        delta,
        1.0 / 3.0 - 0.25,
        1e-6,
        "delta should be 1/3 - 1/4 = 1/12",
    );

    // Verify the subtle property: removing one from a group of 3 does NOT
    // eliminate duplication — the remaining 2 are still exact duplicates.
    assert!(
        without_one_score < 1.0,
        "score should still show duplication after removing one of three",
    );
}

// ---------------------------------------------------------------------------
// Test 3: Code economy
// ---------------------------------------------------------------------------

#[test]
fn test_code_economy_counterfactual_matches_recomputation() {
    let policy = CompliancePolicy::default();

    // 2 pub functions + 6 private functions = 8 total.
    // overhead_ratio = 8 / 2 = 4.0.
    let full_source = format!(
        "{}{}{}{}{}{}{}{}",
        pub_function("pub_a", 0, 10),
        pub_function("pub_b", 1, 10),
        unique_function("priv_c", 2, 10),
        unique_function("priv_d", 3, 10),
        unique_function("priv_e", 4, 10),
        unique_function("priv_f", 0, 15), // variant 0 with different line count to avoid dup
        unique_function("priv_g", 1, 15),
        unique_function("priv_h", 2, 15),
    );

    let (_, full_compliance) = compliance_for(&policy, &full_source);
    let full_ce_score = dimension_score(&full_compliance, "code_economy");

    // Remove one private function: ratio = 7/2 = 3.5.
    let minus_one_priv = format!(
        "{}{}{}{}{}{}{}",
        pub_function("pub_a", 0, 10),
        pub_function("pub_b", 1, 10),
        unique_function("priv_c", 2, 10),
        unique_function("priv_d", 3, 10),
        unique_function("priv_e", 4, 10),
        unique_function("priv_f", 0, 15),
        unique_function("priv_g", 1, 15),
    );

    let (_, minus_one_compliance) = compliance_for(&policy, &minus_one_priv);
    let minus_one_ce_score = dimension_score(&minus_one_compliance, "code_economy");

    // Removing a private function should improve the code economy score
    // (lower overhead ratio => higher score).
    let delta_one = minus_one_ce_score - full_ce_score;
    assert!(
        delta_one > 0.0,
        "removing a private fn should improve code_economy, got delta={delta_one}",
    );

    // Remove two private functions: ratio = 6/2 = 3.0.
    let minus_two_priv = format!(
        "{}{}{}{}{}{}",
        pub_function("pub_a", 0, 10),
        pub_function("pub_b", 1, 10),
        unique_function("priv_c", 2, 10),
        unique_function("priv_d", 3, 10),
        unique_function("priv_e", 4, 10),
        unique_function("priv_f", 0, 15),
    );

    let (_, minus_two_compliance) = compliance_for(&policy, &minus_two_priv);
    let minus_two_ce_score = dimension_score(&minus_two_compliance, "code_economy");

    let delta_two = minus_two_ce_score - full_ce_score;
    assert!(
        delta_two > delta_one,
        "removing two private fns should improve more than one: \
         delta_two={delta_two}, delta_one={delta_one}",
    );

    // Verify the scores are monotonically increasing as we remove overhead.
    assert!(
        minus_two_ce_score > minus_one_ce_score,
        "fewer private fns should mean better code_economy: \
         minus_two={minus_two_ce_score}, minus_one={minus_one_ce_score}",
    );
    assert!(
        minus_one_ce_score > full_ce_score,
        "removing one private fn should improve over full: \
         minus_one={minus_one_ce_score}, full={full_ce_score}",
    );
}

// ---------------------------------------------------------------------------
// Test 4: Bloat
// ---------------------------------------------------------------------------

#[test]
fn test_bloat_counterfactual_matches_recomputation() {
    let policy = CompliancePolicy::default();

    // 3 functions with different line counts: 10, 40, 80.
    // Each has a distinct structure to avoid accidental duplication.
    let full_source = format!(
        "{}{}{}",
        unique_function("small", 0, 10),
        unique_function("medium", 1, 40),
        unique_function("large", 2, 80),
    );

    let (_, full_compliance) = compliance_for(&policy, &full_source);
    let full_bloat = dimension_score(&full_compliance, "bloat");

    // Remove the large function (80 lines): recompute with just small + medium.
    let without_large = format!(
        "{}{}",
        unique_function("small", 0, 10),
        unique_function("medium", 1, 40),
    );
    let (_, without_large_compliance) = compliance_for(&policy, &without_large);
    let without_large_bloat = dimension_score(&without_large_compliance, "bloat");

    // Removing the largest function should improve bloat.
    let delta_large = without_large_bloat - full_bloat;
    assert!(
        delta_large > 0.0,
        "removing 80-line function should improve bloat, got delta={delta_large}",
    );

    // Remove the small function (10 lines): recompute with just medium + large.
    let without_small = format!(
        "{}{}",
        unique_function("medium", 1, 40),
        unique_function("large", 2, 80),
    );
    let (_, without_small_compliance) = compliance_for(&policy, &without_small);
    let without_small_bloat = dimension_score(&without_small_compliance, "bloat");

    let delta_small = without_small_bloat - full_bloat;

    // Removing the large function should help more than removing the small one.
    assert!(
        delta_large > delta_small,
        "removing the 80-line fn should help more than the 10-line fn: \
         delta_large={delta_large}, delta_small={delta_small}",
    );

    // Removing the small function might worsen the score (geometric mean of
    // remaining medium+large is worse than geomean of small+medium+large),
    // or it might slightly help depending on calibration. Either way, the
    // magnitude should be less than removing the large one.
    assert!(
        delta_large.abs() > delta_small.abs(),
        "large function removal should have bigger impact than small: \
         |delta_large|={}, |delta_small|={}",
        delta_large.abs(),
        delta_small.abs(),
    );
}

// ---------------------------------------------------------------------------
// Test 5: State cardinality
// ---------------------------------------------------------------------------

#[test]
fn test_state_cardinality_counterfactual_matches_recomputation() {
    let policy = CompliancePolicy::default();

    // High-cardinality struct (5 bool fields => cardinality 2^5 = 32, log2 = 5.0)
    // Low-cardinality struct (1 bool field => cardinality 2, log2 = 1.0)
    // Function with 2 mut bool bindings (cardinality 2^2 = 4, log2 = 2.0)
    // A "clean" function with no mutable state (does not participate in
    // state_cardinality for functions, but the structs always participate).
    let full_source = "\
struct HighCard {
    a: bool,
    b: bool,
    c: bool,
    d: bool,
    e: bool,
}

struct LowCard {
    a: bool,
}

fn stateful(x: i32) -> i32 {
    let mut flag_a = false;
    let mut flag_b = true;
    if x > 0 {
        flag_a = true;
    } else {
        flag_b = false;
    }
    if flag_a { 1 } else if flag_b { 2 } else { 0 }
}

fn clean(x: i32) -> i32 {
    let result = x + 1;
    if result > 10 {
        result - 5
    } else {
        result + 3
    }
}
";

    let (_, full_compliance) = compliance_for(&policy, full_source);
    let full_sc_score = dimension_score(&full_compliance, "state_cardinality");

    // Remove the high-cardinality struct: should improve score significantly.
    let without_high = "\
struct LowCard {
    a: bool,
}

fn stateful(x: i32) -> i32 {
    let mut flag_a = false;
    let mut flag_b = true;
    if x > 0 {
        flag_a = true;
    } else {
        flag_b = false;
    }
    if flag_a { 1 } else if flag_b { 2 } else { 0 }
}

fn clean(x: i32) -> i32 {
    let result = x + 1;
    if result > 10 {
        result - 5
    } else {
        result + 3
    }
}
";

    let (_, without_high_compliance) = compliance_for(&policy, without_high);
    let without_high_score = dimension_score(&without_high_compliance, "state_cardinality");

    let delta_high = without_high_score - full_sc_score;
    assert!(
        delta_high > 0.0,
        "removing high-cardinality struct should improve state_cardinality, got delta={delta_high}",
    );

    // Remove the low-cardinality struct instead: should also improve but less.
    let without_low = "\
struct HighCard {
    a: bool,
    b: bool,
    c: bool,
    d: bool,
    e: bool,
}

fn stateful(x: i32) -> i32 {
    let mut flag_a = false;
    let mut flag_b = true;
    if x > 0 {
        flag_a = true;
    } else {
        flag_b = false;
    }
    if flag_a { 1 } else if flag_b { 2 } else { 0 }
}

fn clean(x: i32) -> i32 {
    let result = x + 1;
    if result > 10 {
        result - 5
    } else {
        result + 3
    }
}
";

    let (_, without_low_compliance) = compliance_for(&policy, without_low);
    let without_low_score = dimension_score(&without_low_compliance, "state_cardinality");

    let delta_low = without_low_score - full_sc_score;

    // Removing the high-cardinality struct should help more than removing the
    // low-cardinality one. Under geometric mean, removing a bad item improves
    // the mean, while removing a good item (low cardinality) can worsen it.
    assert!(
        delta_high > delta_low,
        "removing high-card struct should help more than low-card: \
         delta_high={delta_high}, delta_low={delta_low}",
    );

    // Removing the low-cardinality struct may actually worsen the score:
    // it scores well under geometric mean, so removing it shifts the mean
    // toward the worse-scoring items. This is the correct behavior.
    assert!(
        delta_low < delta_high,
        "low-card removal should have less impact than high-card: \
         delta_low={delta_low}, delta_high={delta_high}",
    );

    // Cross-check: recompute the full corpus score from its observations by
    // verifying the dimension has the expected number of items.
    let full_sc_dim = full_compliance
        .soft_dimensions
        .iter()
        .find(|d| d.name == "state_cardinality")
        .expect("state_cardinality dimension");

    // Should have 2 types + 1 stateful function = 3 observations.
    // (The `clean` function has no mutable bindings, so it doesn't participate.)
    assert_eq!(
        full_sc_dim.pipeline.observations.len(),
        3,
        "expected 3 state_cardinality observations (2 types + 1 fn with mut bindings)",
    );
}

// ---------------------------------------------------------------------------
// Test 6: Coupling density
// ---------------------------------------------------------------------------

#[test]
fn test_coupling_density_counterfactual_matches_recomputation() {
    let policy = CompliancePolicy::default();

    // Minimal source so the analysis has some functions (other dimensions need
    // at least one function to avoid vacuous scores).
    let source = format!(
        "{}{}{}",
        unique_function("mod_a_fn", 0, 10),
        unique_function("mod_b_fn", 1, 10),
        unique_function("mod_c_fn", 2, 10),
    );

    // Full scenario: 3 modules (a, b, c).
    //   Module a: 5 outgoing edges (a->b x5 different call sites, deduplicated
    //             to a single directed edge per pair, but we model 5 distinct
    //             target modules to get 5 outgoing).
    //   Module b: 1 outgoing edge (b->c).
    //   Module c: 0 outgoing edges.
    // To give module "a" 5 outgoing edges we need 5 callee modules. Add d, e, f, g
    // in addition to b so a calls b, d, e, f, g.
    let full_data = SemanticData {
        crate_name: "test".into(),
        type_cardinalities: Vec::new(),
        function_cardinalities: Vec::new(),
        call_edges: vec![
            // Module a -> 5 distinct targets
            CallEdge {
                caller_module: "a".into(),
                caller_file: "a.rs".into(),
                callee_module: "b".into(),
                callee_file: "b.rs".into(),
                caller_function: "dispatch".into(),
                caller_line: 10,
            },
            CallEdge {
                caller_module: "a".into(),
                caller_file: "a.rs".into(),
                callee_module: "d".into(),
                callee_file: "d.rs".into(),
                caller_function: "dispatch".into(),
                caller_line: 10,
            },
            CallEdge {
                caller_module: "a".into(),
                caller_file: "a.rs".into(),
                callee_module: "e".into(),
                callee_file: "e.rs".into(),
                caller_function: "dispatch".into(),
                caller_line: 10,
            },
            CallEdge {
                caller_module: "a".into(),
                caller_file: "a.rs".into(),
                callee_module: "f".into(),
                callee_file: "f.rs".into(),
                caller_function: "dispatch".into(),
                caller_line: 10,
            },
            CallEdge {
                caller_module: "a".into(),
                caller_file: "a.rs".into(),
                callee_module: "g".into(),
                callee_file: "g.rs".into(),
                caller_function: "dispatch".into(),
                caller_line: 10,
            },
            // Module b -> 1 target
            CallEdge {
                caller_module: "b".into(),
                caller_file: "b.rs".into(),
                callee_module: "c".into(),
                callee_file: "c.rs".into(),
                caller_function: "process".into(),
                caller_line: 5,
            },
        ],
    };
    let full_overlay = SemanticOverlay::from_data(&full_data);
    let (_, full_compliance) = compliance_with_semantic(&policy, &source, &full_overlay);
    let full_coupling = dimension_score(&full_compliance, "coupling_density");
    let full_composite = full_compliance.composite_score;

    // Modified scenario: remove module a's outgoing edges entirely.
    // Now only b->c remains.
    let reduced_data = SemanticData {
        crate_name: "test".into(),
        type_cardinalities: Vec::new(),
        function_cardinalities: Vec::new(),
        call_edges: vec![CallEdge {
            caller_module: "b".into(),
            caller_file: "b.rs".into(),
            callee_module: "c".into(),
            callee_file: "c.rs".into(),
            caller_function: "process".into(),
            caller_line: 5,
        }],
    };
    let reduced_overlay = SemanticOverlay::from_data(&reduced_data);
    let (_, reduced_compliance) = compliance_with_semantic(&policy, &source, &reduced_overlay);
    let reduced_coupling = dimension_score(&reduced_compliance, "coupling_density");
    let reduced_composite = reduced_compliance.composite_score;

    // Removing the high-coupling module's edges should improve the coupling
    // dimension score (and thus the composite).
    assert!(
        reduced_coupling > full_coupling,
        "removing high-edge module's edges should improve coupling_density: \
         reduced={reduced_coupling}, full={full_coupling}",
    );
    assert!(
        reduced_composite > full_composite,
        "removing high-edge module's edges should improve composite: \
         reduced={reduced_composite}, full={full_composite}",
    );

    // Verify heatmap: module a (high edges) should have greater responsibility
    // than module b (low edges).
    let coupling_heatmap: Vec<_> = full_compliance
        .heatmap
        .iter()
        .filter(|e| e.dimension == "coupling_density")
        .collect();

    let a_responsibility: f64 = coupling_heatmap
        .iter()
        .filter(|e| e.function_name.starts_with("a::"))
        .map(|e| e.responsibility)
        .sum();
    let b_responsibility: f64 = coupling_heatmap
        .iter()
        .filter(|e| e.function_name.starts_with("b::"))
        .map(|e| e.responsibility)
        .sum();

    assert!(
        a_responsibility > b_responsibility,
        "high-edge module a should have greater heatmap responsibility than low-edge module b: \
         a={a_responsibility}, b={b_responsibility}",
    );

    // Sanity: modules with 0 outgoing edges should not appear in the heatmap.
    let zero_edge_modules: Vec<_> = coupling_heatmap
        .iter()
        .filter(|e| {
            e.function_name == "c"
                || e.function_name == "d"
                || e.function_name == "e"
                || e.function_name == "f"
                || e.function_name == "g"
        })
        .collect();
    assert!(
        zero_edge_modules.is_empty(),
        "modules with 0 outgoing edges should not appear in the coupling heatmap, \
         but found: {zero_edge_modules:?}",
    );
}

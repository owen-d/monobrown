//! Golden-file integration tests for descendit.
//!
//! These tests run the CLI against purpose-built fixture files that each
//! trigger a specific loss dimension, then snapshot the output with insta.
//!
//! Since fixture files are standalone .rs files (not real crates), we provide
//! an empty semantic data file via `--semantic-path` to bypass RA analysis.

#![allow(clippy::expect_used)]

use assert_cmd::Command;

fn descendit_cmd() -> Command {
    assert_cmd::cargo_bin_cmd!("descendit")
}

fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("golden")
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    fixtures_dir().join(name)
}

fn empty_semantic_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("empty_semantic.json")
}

/// Run `descendit analyze --semantic-path <empty> --agent` on a single fixture.
fn analyze_fixture(name: &str) -> String {
    let output = descendit_cmd()
        .args(["analyze", "--semantic-path"])
        .arg(empty_semantic_path())
        .arg("--agent")
        .arg(fixture_path(name))
        .output()
        .expect("failed to run descendit analyze");
    assert!(
        output.status.success(),
        "descendit analyze failed for {name}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("non-utf8 stdout")
}

// ---------------------------------------------------------------------------
// Per-fixture agent snapshots
// ---------------------------------------------------------------------------

#[test]
fn golden_clean() {
    let output = analyze_fixture("clean.rs");
    insta::assert_snapshot!("analyze_clean", output);
}

#[test]
fn golden_bloated() {
    let output = analyze_fixture("bloated.rs");
    insta::assert_snapshot!("analyze_bloated", output);
}

#[test]
fn golden_duplicated() {
    let output = analyze_fixture("duplicated.rs");
    insta::assert_snapshot!("analyze_duplicated", output);
}

#[test]
fn golden_complex_types() {
    let output = analyze_fixture("complex_types.rs");
    insta::assert_snapshot!("analyze_complex_types", output);
}

#[test]
fn golden_overhead() {
    let output = analyze_fixture("overhead.rs");
    insta::assert_snapshot!("analyze_overhead", output);
}

#[test]
fn golden_mixed() {
    let output = analyze_fixture("mixed.rs");
    insta::assert_snapshot!("analyze_mixed", output);
}

// ---------------------------------------------------------------------------
// Aggregate: analyze the whole fixtures/golden/ directory
// ---------------------------------------------------------------------------

#[test]
fn golden_aggregate() {
    // Multi-path analyze produces raw corpus experiment output (no --agent).
    let output = descendit_cmd()
        .args(["analyze", "--semantic-path"])
        .arg(empty_semantic_path())
        .arg(fixtures_dir())
        .output()
        .expect("failed to run descendit analyze on fixtures dir");
    assert!(
        output.status.success(),
        "descendit analyze (aggregate) failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("non-utf8 stdout");
    insta::assert_snapshot!("analyze_aggregate", stdout);
}

// ---------------------------------------------------------------------------
// Heatmap on the fixtures directory
// ---------------------------------------------------------------------------

#[test]
fn golden_heatmap() {
    let output = descendit_cmd()
        .args(["heatmap", "--semantic-path"])
        .arg(empty_semantic_path())
        .arg(fixtures_dir())
        .output()
        .expect("failed to run descendit heatmap");
    assert!(
        output.status.success(),
        "descendit heatmap failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("non-utf8 stdout");
    insta::assert_snapshot!("heatmap_golden", stdout);
}

// ---------------------------------------------------------------------------
// List dimensions
// ---------------------------------------------------------------------------

#[test]
fn golden_list() {
    let output = descendit_cmd()
        .arg("list")
        .output()
        .expect("failed to run descendit list");
    assert!(
        output.status.success(),
        "descendit list failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("non-utf8 stdout");
    insta::assert_snapshot!("list_dimensions", stdout);
}

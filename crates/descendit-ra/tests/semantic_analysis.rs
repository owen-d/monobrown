//! Integration tests for the RA semantic backend.
//!
//! These tests analyze the fixture crate and verify the output.

use std::path::PathBuf;
use std::sync::OnceLock;

use descendit_ra::{SemanticData, TypeTraitFactKind};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-crate")
}

/// Cache the analysis result -- loading a workspace is expensive.
fn cached_analysis() -> Result<&'static SemanticData, &'static str> {
    static DATA: OnceLock<Result<SemanticData, String>> = OnceLock::new();
    let result = DATA.get_or_init(|| {
        descendit_ra::analyze(&fixture_path()).map_err(|e| format!("analysis failed: {e}"))
    });
    result.as_ref().map_err(String::as_str)
}

#[test]
fn analyze_produces_expected_fixture_semantics() -> Result<(), Box<dyn std::error::Error>> {
    let data = cached_analysis()?;

    assert_eq!(data.crate_name, "sample_crate");
    assert!(
        !data.type_cardinalities.is_empty(),
        "should find type cardinalities"
    );
    assert!(!data.call_edges.is_empty(), "should find call edges");

    let config = data
        .type_cardinalities
        .iter()
        .find(|tc| tc.name == "Config")
        .ok_or("should find Config type")?;
    assert!(
        (config.cardinality_log2 - 2.0).abs() < 0.01,
        "Config cardinality_log2 should be 2.0 (2 bools), got {}",
        config.cardinality_log2
    );

    let state = data
        .type_cardinalities
        .iter()
        .find(|tc| tc.name == "State")
        .ok_or("should find State enum")?;
    assert!(
        (state.cardinality_log2 - 2.0).abs() < 0.01,
        "State cardinality_log2 should be 2.0, got {}",
        state.cardinality_log2
    );

    let orch = data
        .function_cardinalities
        .iter()
        .find(|fc| fc.name == "orchestrate")
        .ok_or("should find orchestrate function")?;
    assert!(
        (orch.internal_state_cardinality_log2 - 1.0).abs() < 0.01,
        "orchestrate internal state cardinality should be 1.0 (one mut bool), got {}",
        orch.internal_state_cardinality_log2
    );

    let has_types_edge = data
        .call_edges
        .iter()
        .any(|e| e.callee_module.contains("types") || e.callee_file.contains("types"));
    assert!(has_types_edge, "should have edge to types module");

    assert!(
        data.type_trait_facts.iter().any(|fact| matches!(
            &fact.kind,
            TypeTraitFactKind::TraitDecl { name } if name.ends_with("types::Runnable")
        )),
        "should find Runnable trait declaration"
    );
    assert!(
        data.type_trait_facts.iter().any(|fact| matches!(
            &fact.kind,
            TypeTraitFactKind::TypeImplTrait { ty, tr }
                if ty.ends_with("types::Config") && tr.ends_with("types::Runnable")
        )),
        "should find Config -> Runnable impl"
    );

    // Test modules are included because the backend loads all targets.
    // The fixture's test module has no types, so we just verify analysis succeeds
    // without filtering — no type cardinalities from "tests" is expected here
    // since the fixture's #[cfg(test)] mod only contains a #[test] fn.
    assert!(
        !data.type_cardinalities.is_empty(),
        "should still find non-test type cardinalities"
    );

    let json = serde_json::to_string_pretty(data)?;
    let parsed: serde_json::Value = serde_json::from_str(&json)?;
    assert!(parsed.is_object(), "top-level JSON should be an object");
    Ok(())
}

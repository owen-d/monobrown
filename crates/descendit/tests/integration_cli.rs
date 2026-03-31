use assert_cmd::Command;

fn descendit_cmd() -> Command {
    assert_cmd::cargo_bin_cmd!("descendit")
}

#[test]
fn help_output_succeeds() {
    descendit_cmd().arg("--help").assert().success();
}

#[test]
fn analyze_with_semantic_path_succeeds() {
    let semantic_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("empty_semantic.json");
    descendit_cmd()
        .args(["analyze", "--semantic-path"])
        .arg(&semantic_path)
        .arg(".")
        .assert()
        .success();
}

#[test]
fn analyze_rejects_semantic_off_flag() {
    descendit_cmd()
        .args(["analyze", "--semantic", "off", "."])
        .assert()
        .failure();
}

#[test]
fn list_runs() {
    descendit_cmd().arg("list").assert().success();
}

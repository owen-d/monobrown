use assert_cmd::Command;

fn descendit_cmd() -> Command {
    assert_cmd::cargo_bin_cmd!("descendit")
}

#[test]
fn help_output_succeeds() {
    descendit_cmd().arg("--help").assert().success();
}

#[test]
fn analyze_semantic_off_succeeds() {
    descendit_cmd()
        .args(["analyze", "--semantic", "off", "."])
        .assert()
        .success();
}

#[test]
fn list_runs() {
    descendit_cmd().arg("list").assert().success();
}

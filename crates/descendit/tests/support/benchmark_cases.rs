#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    dead_code
)]

use std::fs;
use std::path::Path;

use descendit::{
    AnalysisReport, CompliancePolicy, ComplianceReport, analyze_path, compute_compliance,
};

pub struct BenchmarkCase {
    pub name: &'static str,
    pub before: &'static str,
    pub after: &'static str,
}

pub fn all_benchmark_cases() -> &'static [BenchmarkCase] {
    &[
        BenchmarkCase {
            name: "split_god_function",
            before: r#"
pub fn process_items(items: &[i32]) -> i32 {
    let mut total = 0;
    for item in items {
        if *item > 0 {
            total += item * 2;
        } else {
            total += item.abs();
        }
    }
    total
}
"#,
            after: r#"
pub fn process_items(items: &[i32]) -> i32 {
    let cleaned = normalize(items);
    accumulate(&cleaned)
}

fn normalize(items: &[i32]) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if *item > 0 {
            out.push(item * 2);
        } else {
            out.push(item.abs());
        }
    }
    out
}

fn accumulate(items: &[i32]) -> i32 {
    let mut total = 0;
    for item in items {
        total += item;
    }
    total
}
"#,
        },
        BenchmarkCase {
            name: "merge_helpers_into_one_function",
            before: r#"
pub fn process_items(items: &[i32]) -> i32 {
    let cleaned = normalize(items);
    accumulate(&cleaned)
}

fn normalize(items: &[i32]) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if *item > 0 {
            out.push(item * 2);
        } else {
            out.push(item.abs());
        }
    }
    out
}

fn accumulate(items: &[i32]) -> i32 {
    let mut total = 0;
    for item in items {
        total += item;
    }
    total
}
"#,
            after: r#"
pub fn process_items(items: &[i32]) -> i32 {
    let mut total = 0;
    for item in items {
        if *item > 0 {
            total += item * 2;
        } else {
            total += item.abs();
        }
    }
    total
}
"#,
        },
        BenchmarkCase {
            name: "add_duplication",
            before: r#"
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
"#,
            after: r#"
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
"#,
        },
        BenchmarkCase {
            name: "replace_bool_soup_with_enum",
            before: r#"
struct Config {
    enabled: bool,
    verbose: bool,
    debug: bool,
    name: String,
}

fn use_config(c: &Config) -> i32 {
    if c.enabled { 1 } else { 0 }
}
"#,
            after: r#"
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
"#,
        },
        BenchmarkCase {
            name: "inflate_public_api",
            before: r#"
pub fn api_entry(x: i32) -> i32 {
    helper(x)
}

fn helper(x: i32) -> i32 {
    if x > 0 { x * 2 } else { x }
}
"#,
            after: r#"
pub fn api_entry(x: i32) -> i32 {
    helper(x)
}

pub fn helper(x: i32) -> i32 {
    if x > 0 { x * 2 } else { x }
}
"#,
        },
        BenchmarkCase {
            name: "hide_overhead_in_macro",
            before: r#"
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
"#,
            after: r#"
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
"#,
        },
    ]
}

pub fn benchmark_case(name: &str) -> &'static BenchmarkCase {
    all_benchmark_cases()
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("unknown benchmark case: {name}"))
}

fn write_source(source: &str, dir: &Path) {
    fs::write(dir.join("lib.rs"), source).expect("write benchmark case source");
}

pub fn analyze_case(
    case: &BenchmarkCase,
) -> (
    AnalysisReport,
    ComplianceReport,
    AnalysisReport,
    ComplianceReport,
) {
    let dir = tempfile::tempdir().expect("create benchmark tempdir");

    write_source(case.before, dir.path());
    let before_analysis = analyze_path(dir.path()).expect("analyze benchmark before");
    let before_compliance = compute_compliance(&before_analysis, &CompliancePolicy::default());

    write_source(case.after, dir.path());
    let after_analysis = analyze_path(dir.path()).expect("analyze benchmark after");
    let after_compliance = compute_compliance(&after_analysis, &CompliancePolicy::default());

    (
        before_analysis,
        before_compliance,
        after_analysis,
        after_compliance,
    )
}

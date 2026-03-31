//! CLI for the descendit structural analysis tool.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

#[cfg(all(unix, feature = "semantic"))]
mod client;
mod semantic_runtime;
#[cfg(all(unix, feature = "semantic"))]
mod server;
#[cfg(all(unix, feature = "semantic"))]
mod server_protocol;

use semantic_runtime::{ensure_semantic_data, resolve_semantic};

/// Deterministic loss functions for code.
#[derive(Debug, Parser)]
#[command(
    name = "descendit",
    bin_name = "descendit",
    version,
    about = "Deterministic structural loss functions for Rust code"
)]
struct Cli {
    /// Connect to a running analysis server via this Unix socket path.
    #[arg(long, global = true)]
    sock: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scan source code and produce a raw metrics snapshot.
    ///
    /// This is the foundation of the pipeline. Use it to capture the current
    /// state of a crate or directory, then feed the output into `diff`
    /// or `heatmap` for further analysis.
    Analyze {
        /// Paths to analyze (directories or .rs files). Multiple paths use shared normalization.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Agent-friendly compact output: composite loss, per-dimension losses, top heatmap items.
        #[arg(long)]
        agent: bool,
        /// Number of top heatmap items to include (requires --agent).
        #[arg(long, default_value_t = 10, requires = "agent")]
        top: usize,
        /// Path to semantic data JSON (skip backend generation when provided).
        #[arg(long = "semantic-path")]
        semantic_path: Option<PathBuf>,
    },
    /// Compare two analysis snapshots and show what changed.
    ///
    /// Takes two `analyze` JSON files (baseline and current) and produces
    /// per-dimension deltas. Use it to check whether a change improved or
    /// regressed code quality.
    #[command(group(clap::ArgGroup::new("compliance_mode")
        .args(["loss_vector", "compliance", "heatmap"])
        .multiple(true)
        .required(false)))]
    Diff {
        /// Baseline analysis JSON file.
        baseline: PathBuf,
        /// Current analysis JSON file.
        current: PathBuf,
        /// Output as structured loss vector.
        #[arg(long)]
        loss_vector: bool,
        /// Compare at the compliance/loss level (composite + per-dimension deltas).
        #[arg(long, conflicts_with = "loss_vector")]
        compliance: bool,
        /// Custom compliance policy JSON file (requires --compliance, --heatmap, or --loss-vector).
        #[arg(long, requires = "compliance_mode")]
        policy: Option<PathBuf>,
        /// Show heatmap item changes between snapshots.
        #[arg(long)]
        heatmap: bool,
        /// Output as JSON (requires --heatmap).
        #[arg(long, requires = "heatmap")]
        json: bool,
        /// Path to semantic data JSON.
        #[arg(long = "semantic-path")]
        semantic_path: Option<PathBuf>,
    },
    /// List all available loss dimensions and their descriptions.
    List {
        /// Output as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,
    },
    /// Watch paths for changes and serve analysis over a Unix socket.
    #[cfg(all(unix, feature = "semantic"))]
    Watch {
        /// Path for the Unix socket to listen on.
        #[arg(long)]
        sock: PathBuf,
        /// Paths to watch for changes.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Shut down a running watch server.
    ///
    /// Sends a reap request to the server identified by the global
    /// `--sock` option.
    #[cfg(all(unix, feature = "semantic"))]
    Reap,
    /// Drill down into which functions and types contribute most to loss.
    ///
    /// Attributes loss across all dimensions to individual code items. Use it
    /// when overall scores are high and you need to find what to fix first.
    Heatmap {
        /// Paths to analyze. Multiple paths use shared normalization.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Custom compliance policy JSON file (optional).
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Output as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,
        /// Render as hierarchical rollup tree instead of flat list.
        #[arg(long)]
        tree: bool,
        /// Limit output to the top N entries by responsibility.
        #[arg(long)]
        top: Option<usize>,
        /// Path to semantic data JSON (skip backend generation when provided).
        #[arg(long = "semantic-path")]
        semantic_path: Option<PathBuf>,
    },
    /// Agent-oriented utilities.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Dump the default compliance policy as JSON.
    ///
    /// Useful for understanding the built-in thresholds or as a starting
    /// point for a custom policy file.
    Policy {
        /// Dump the default policy.
        #[arg(long)]
        dump_default: bool,
    },
    /// Interactive flamegraph explorer for loss drill-down.
    ///
    /// Renders an interactive TUI showing per-item loss attribution with
    /// split/stacked detail views and syntax-highlighted source preview.
    #[cfg(feature = "explore")]
    Explore {
        /// Paths to analyze. Multiple paths use shared normalization.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Custom compliance policy JSON file (optional).
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Path to semantic data JSON (skip backend generation when provided).
        #[arg(long = "semantic-path")]
        semantic_path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// Emit a self-contained guide for LLM / agent consumption.
    ///
    /// Prints a markdown document covering installation, all subcommands,
    /// loss dimensions, and the analyze-diff workflow.
    Guide,
}

// ---------------------------------------------------------------------------
// Presentation-layer types
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct AgentAnalysisSummary {
    composite_loss: f64,
    dimensions: std::collections::BTreeMap<String, AgentDimensionSummary>,
    top_heatmap: Vec<descendit::HeatmapEntry>,
    heatmap_entry_count: usize,
    dimension_totals: Vec<descendit::ExperimentHeatmapDimensionSummary>,
}

#[derive(serde::Serialize)]
struct AgentDimensionSummary {
    loss: f64,
    item_count: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    not_measured: bool,
}

#[derive(serde::Serialize)]
struct ComplianceDiffReport {
    composite: ComplianceDiffEntry,
    dimensions: Vec<ComplianceDiffDimension>,
}

#[derive(serde::Serialize)]
struct ComplianceDiffDimension {
    name: String,
    #[serde(flatten)]
    diff: ComplianceDiffEntry,
}

#[derive(serde::Serialize)]
struct ComplianceDiffEntry {
    before_loss: f64,
    after_loss: f64,
    delta: f64,
    assessment: descendit::Assessment,
}

#[derive(serde::Serialize)]
struct HeatmapDiffReport {
    appeared: Vec<descendit::HeatmapEntry>,
    disappeared: Vec<descendit::HeatmapEntry>,
    changed: Vec<HeatmapDiffEntry>,
    summary: HeatmapDiffSummary,
}

#[derive(serde::Serialize)]
struct HeatmapDiffEntry {
    file: String,
    function_name: String,
    dimension: String,
    before_responsibility: f64,
    after_responsibility: f64,
    delta: f64,
    assessment: descendit::Assessment,
    before_detail: String,
    after_detail: String,
}

#[derive(serde::Serialize)]
struct HeatmapDiffSummary {
    appeared_count: usize,
    disappeared_count: usize,
    improved_count: usize,
    regressed_count: usize,
    unchanged_count: usize,
}

#[derive(serde::Serialize)]
struct FullDiffReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    compliance: Option<ComplianceDiffReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    heatmap: Option<HeatmapDiffReport>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    dispatch(cli.command, cli.sock.as_deref())
}

#[allow(clippy::too_many_lines)]
fn dispatch(command: Command, socket: Option<&Path>) -> anyhow::Result<()> {
    match command {
        Command::Analyze {
            paths,
            agent,
            top,
            semantic_path,
        } => {
            if paths.len() == 1 {
                let overlay = ensure_semantic_data(
                    semantic_path.as_deref(),
                    Some(&paths[0]),
                    socket,
                )?;
                run_analyze(&AnalyzeParams {
                    path: &paths[0],
                    agent,
                    top,
                    semantic: Some(&overlay),
                })?;
            } else {
                if agent {
                    anyhow::bail!(
                        "--agent is not supported with multiple paths"
                    );
                }
                if semantic_path.is_some() {
                    anyhow::bail!(
                        "--semantic-path is not supported with multiple paths; \
                         use --sock for semantic analysis"
                    );
                }
                run_analyze_multi(&paths, socket)?;
            }
        }
        Command::Diff {
            baseline,
            current,
            loss_vector,
            compliance,
            policy,
            heatmap,
            json,
            semantic_path,
        } => {
            let overlay = resolve_semantic(semantic_path.as_deref(), None)?;
            run_diff(&DiffParams {
                baseline: &baseline,
                current: &current,
                loss_vector,
                compliance,
                heatmap,
                json,
                policy_path: policy.as_deref(),
                semantic: overlay.as_ref(),
            })?;
        }
        #[cfg(all(unix, feature = "semantic"))]
        Command::Watch { sock, paths } => {
            server::run_watch(&sock, &paths)?;
        }
        #[cfg(all(unix, feature = "semantic"))]
        Command::Reap => dispatch_reap(socket)?,
        Command::List { json } => run_list(json)?,
        Command::Heatmap {
            paths,
            policy,
            json,
            tree,
            top,
            semantic_path,
        } => {
            if paths.len() == 1 {
                let overlay = ensure_semantic_data(
                    semantic_path.as_deref(),
                    Some(&paths[0]),
                    socket,
                )?;
                run_heatmap(&paths[0], policy.as_deref(), json, tree, top, Some(&overlay))?;
            } else {
                if semantic_path.is_some() {
                    anyhow::bail!(
                        "--semantic-path is not supported with multiple paths; \
                         use --sock for semantic analysis"
                    );
                }
                run_heatmap_multi(&paths, policy.as_deref(), json, tree, top, socket)?;
            }
        }
        Command::Agent { command } => match command {
            AgentCommand::Guide => run_guide(),
        },
        Command::Policy { dump_default } => run_policy(dump_default)?,
        #[cfg(feature = "explore")]
        Command::Explore {
            paths,
            policy,
            semantic_path,
        } => {
            if paths.len() == 1 {
                let overlay = ensure_semantic_data(
                    semantic_path.as_deref(),
                    Some(&paths[0]),
                    socket,
                )?;
                run_explore(&paths[0], policy.as_deref(), Some(&overlay))?;
            } else {
                if semantic_path.is_some() {
                    anyhow::bail!(
                        "--semantic-path is not supported with multiple paths; \
                         use --sock for semantic analysis"
                    );
                }
                run_explore_multi(&paths, policy.as_deref(), socket)?;
            }
        }
    }
    Ok(())
}

#[cfg(all(unix, feature = "semantic"))]
fn dispatch_reap(socket: Option<&Path>) -> anyhow::Result<()> {
    let socket_path = socket.ok_or_else(|| anyhow::anyhow!("--sock is required for reap"))?;
    client::reap(socket_path)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn load_policy(policy_path: Option<&Path>) -> anyhow::Result<descendit::CompliancePolicy> {
    match policy_path {
        Some(path) => {
            let json = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&json)?)
        }
        None => Ok(descendit::CompliancePolicy::default()),
    }
}

/// For loss values, a negative delta means improvement (loss decreased).
fn assess_loss_delta(delta: f64) -> descendit::Assessment {
    if delta.abs() < 1e-10 {
        descendit::Assessment::Unchanged
    } else if delta < 0.0 {
        descendit::Assessment::Improved
    } else {
        descendit::Assessment::Regressed
    }
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

struct AnalyzeParams<'a> {
    path: &'a Path,
    agent: bool,
    top: usize,
    semantic: Option<&'a descendit::SemanticOverlay>,
}

fn run_analyze(params: &AnalyzeParams<'_>) -> anyhow::Result<()> {
    let mut report = descendit::analyze_path(params.path)?;
    if report.files_analyzed == 0 {
        anyhow::bail!(
            "no .rs files found in {}",
            params.path.display()
        );
    }
    if let Some(overlay) = params.semantic {
        report.semantic = Some(descendit::SemanticSummary::from_overlay(overlay));
    }

    if params.agent {
        let policy = descendit::CompliancePolicy::default();
        let cr = descendit::compute_compliance_with_semantic(&report, &policy, params.semantic);
        print_agent_summary(&cr, params.top)?;
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn print_agent_summary(cr: &descendit::ComplianceReport, top: usize) -> anyhow::Result<()> {
    let heatmap_summary = descendit::summarize_heatmap(&cr.heatmap, top);

    let mut dimensions = std::collections::BTreeMap::new();
    for dim in &cr.soft_dimensions {
        let not_measured = dim.name == "coupling_density" && dim.item_count == 0;
        dimensions.insert(
            dim.name.clone(),
            AgentDimensionSummary {
                loss: 1.0 - dim.score,
                item_count: dim.item_count,
                not_measured,
            },
        );
    }

    let summary = AgentAnalysisSummary {
        composite_loss: 1.0 - cr.composite_score,
        dimensions,
        top_heatmap: heatmap_summary.top_entries,
        heatmap_entry_count: heatmap_summary.entry_count,
        dimension_totals: heatmap_summary.dimension_totals,
    };
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn run_list(json: bool) -> anyhow::Result<()> {
    let all = descendit::LossFunction::all();

    if json {
        let entries: Vec<serde_json::Value> = all
            .iter()
            .map(|lf| {
                let info = lf.scoring_info();
                serde_json::json!({
                    "name": lf.name(),
                    "description": lf.description(),
                    "calculation": lf.calculation(),
                    "formula": info.formula,
                    "aggregation": lf.aggregation(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    // Human-readable output.
    println!("Loss Dimensions");
    println!("===============");
    println!("Default composite loss = 1 - geometric_mean(dimension_scores). 0.0 = perfect.");
    println!();

    for (i, lf) in all.iter().enumerate() {
        let info = lf.scoring_info();
        let tag = if lf.is_composite() {
            ""
        } else {
            " (diagnostic only)"
        };
        println!("{}. {}{}", i + 1, lf.name(), tag);
        println!();
        println!("   {}", lf.description());
        println!();
        // Wrap calculation text with indentation for "Calculation:" label alignment.
        print_wrapped("   Calculation:  ", lf.calculation());
        println!("   Formula:      loss = {}", info.formula);
        println!("   Aggregation:  {}", lf.aggregation());
        println!();
    }

    Ok(())
}

/// Print a labeled, wrapped line. The first line uses the label; continuation
/// lines are indented to match the label width.
fn print_wrapped(label: &str, text: &str) {
    let indent = " ".repeat(label.len());
    let max_width: usize = 80;
    let content_width = max_width.saturating_sub(label.len());

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();

    for word in &words {
        if current_line.is_empty() {
            current_line.push_str(word);
        } else if current_line.len() + 1 + word.len() <= content_width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            lines.push(current_line);
            current_line = word.to_string();
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            println!("{label}{line}");
        } else {
            println!("{indent}{line}");
        }
    }
}

struct DiffParams<'a> {
    baseline: &'a Path,
    current: &'a Path,
    loss_vector: bool,
    compliance: bool,
    heatmap: bool,
    json: bool,
    policy_path: Option<&'a Path>,
    semantic: Option<&'a descendit::SemanticOverlay>,
}

fn run_diff(params: &DiffParams<'_>) -> anyhow::Result<()> {
    let baseline_json = std::fs::read_to_string(params.baseline)?;
    let current_json = std::fs::read_to_string(params.current)?;

    let baseline_report: descendit::AnalysisReport = serde_json::from_str(&baseline_json)?;
    let current_report: descendit::AnalysisReport = serde_json::from_str(&current_json)?;

    let needs_compliance = params.compliance || params.heatmap || params.loss_vector;
    if needs_compliance {
        let policy = load_policy(params.policy_path)?;
        let baseline_cr =
            descendit::compute_compliance_with_semantic(&baseline_report, &policy, params.semantic);
        let current_cr =
            descendit::compute_compliance_with_semantic(&current_report, &policy, params.semantic);

        if params.compliance || params.heatmap {
            return print_compliance_heatmap_diff(
                &baseline_cr,
                &current_cr,
                params.compliance,
                params.heatmap,
                params.json,
            );
        }
        // loss_vector
        let lv = descendit::compliance_delta_to_loss_vector(&baseline_cr, &current_cr)?;
        println!("{}", serde_json::to_string_pretty(&lv)?);
    } else {
        let diff = descendit::diff::diff_summaries(
            &baseline_report.summary,
            &current_report.summary,
            baseline_report.semantic.as_ref(),
            current_report.semantic.as_ref(),
        );
        println!("{}", serde_json::to_string_pretty(&diff)?);
    }

    Ok(())
}

/// Format and print compliance and/or heatmap diff output.
fn print_compliance_heatmap_diff(
    baseline_cr: &descendit::ComplianceReport,
    current_cr: &descendit::ComplianceReport,
    compliance: bool,
    heatmap: bool,
    json: bool,
) -> anyhow::Result<()> {
    if compliance && heatmap {
        let report = FullDiffReport {
            compliance: Some(build_compliance_diff(baseline_cr, current_cr)),
            heatmap: Some(build_heatmap_diff(baseline_cr, current_cr)),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if compliance {
        let report = build_compliance_diff(baseline_cr, current_cr);
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let heatmap_diff = build_heatmap_diff(baseline_cr, current_cr);
        if json {
            println!("{}", serde_json::to_string_pretty(&heatmap_diff)?);
        } else {
            print_heatmap_diff_human(&heatmap_diff);
        }
    }
    Ok(())
}

fn build_compliance_diff(
    baseline_cr: &descendit::ComplianceReport,
    current_cr: &descendit::ComplianceReport,
) -> ComplianceDiffReport {
    let before_composite_loss = 1.0 - baseline_cr.composite_score;
    let after_composite_loss = 1.0 - current_cr.composite_score;
    let composite_delta = after_composite_loss - before_composite_loss;

    let composite = ComplianceDiffEntry {
        before_loss: before_composite_loss,
        after_loss: after_composite_loss,
        delta: composite_delta,
        assessment: assess_loss_delta(composite_delta),
    };

    let dimensions: Vec<ComplianceDiffDimension> = baseline_cr
        .soft_dimensions
        .iter()
        .zip(current_cr.soft_dimensions.iter())
        .map(|(b, c)| {
            debug_assert_eq!(b.name, c.name, "dimension order mismatch");
            let bl = 1.0 - b.score;
            let cl = 1.0 - c.score;
            let d = cl - bl;
            ComplianceDiffDimension {
                name: b.name.clone(),
                diff: ComplianceDiffEntry {
                    before_loss: bl,
                    after_loss: cl,
                    delta: d,
                    assessment: assess_loss_delta(d),
                },
            }
        })
        .collect();

    ComplianceDiffReport {
        composite,
        dimensions,
    }
}

type HeatmapKey = (String, String, String);

fn build_heatmap_diff(
    baseline_cr: &descendit::ComplianceReport,
    current_cr: &descendit::ComplianceReport,
) -> HeatmapDiffReport {
    use std::collections::BTreeMap;

    fn build_map(
        entries: &[descendit::HeatmapEntry],
    ) -> BTreeMap<HeatmapKey, &descendit::HeatmapEntry> {
        entries
            .iter()
            .map(|e| {
                (
                    (e.file.clone(), e.function_name.clone(), e.dimension.clone()),
                    e,
                )
            })
            .collect()
    }

    let baseline_map = build_map(&baseline_cr.heatmap);
    let current_map = build_map(&current_cr.heatmap);

    let (appeared, disappeared, mut changed) =
        classify_heatmap_entries(&baseline_map, &current_map);

    // Sort changed by absolute delta descending.
    changed.sort_by(|a, b| {
        b.delta
            .abs()
            .partial_cmp(&a.delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    build_heatmap_summary(appeared, disappeared, changed)
}

/// Classify heatmap entries as appeared, disappeared, or changed.
fn classify_heatmap_entries(
    baseline_map: &std::collections::BTreeMap<HeatmapKey, &descendit::HeatmapEntry>,
    current_map: &std::collections::BTreeMap<HeatmapKey, &descendit::HeatmapEntry>,
) -> (
    Vec<descendit::HeatmapEntry>,
    Vec<descendit::HeatmapEntry>,
    Vec<HeatmapDiffEntry>,
) {
    let mut all_keys: Vec<&HeatmapKey> = baseline_map.keys().chain(current_map.keys()).collect();
    all_keys.sort();
    all_keys.dedup();

    let mut appeared = Vec::new();
    let mut disappeared = Vec::new();
    let mut changed = Vec::new();

    for key in all_keys {
        match (baseline_map.get(key), current_map.get(key)) {
            (None, Some(entry)) => appeared.push((*entry).clone()),
            (Some(entry), None) => disappeared.push((*entry).clone()),
            (Some(b), Some(c)) => {
                let delta = c.responsibility - b.responsibility;
                changed.push(HeatmapDiffEntry {
                    file: b.file.clone(),
                    function_name: b.function_name.clone(),
                    dimension: b.dimension.clone(),
                    before_responsibility: b.responsibility,
                    after_responsibility: c.responsibility,
                    delta,
                    assessment: assess_loss_delta(delta),
                    before_detail: b.detail.clone(),
                    after_detail: c.detail.clone(),
                });
            }
            (None, None) => unreachable!(),
        }
    }

    (appeared, disappeared, changed)
}

/// Build the final heatmap diff report with summary counts.
fn build_heatmap_summary(
    appeared: Vec<descendit::HeatmapEntry>,
    disappeared: Vec<descendit::HeatmapEntry>,
    changed: Vec<HeatmapDiffEntry>,
) -> HeatmapDiffReport {
    let mut improved_count: usize = 0;
    let mut regressed_count: usize = 0;
    let mut unchanged_count: usize = 0;
    for entry in &changed {
        match entry.assessment {
            descendit::Assessment::Improved => improved_count += 1,
            descendit::Assessment::Regressed => regressed_count += 1,
            descendit::Assessment::Unchanged => unchanged_count += 1,
        }
    }

    HeatmapDiffReport {
        summary: HeatmapDiffSummary {
            appeared_count: appeared.len(),
            disappeared_count: disappeared.len(),
            improved_count,
            regressed_count,
            unchanged_count,
        },
        appeared,
        disappeared,
        changed,
    }
}

fn print_heatmap_diff_human(report: &HeatmapDiffReport) {
    println!(
        "Heatmap diff: {} appeared, {} disappeared, {} improved, {} regressed, {} unchanged",
        report.summary.appeared_count,
        report.summary.disappeared_count,
        report.summary.improved_count,
        report.summary.regressed_count,
        report.summary.unchanged_count,
    );
    println!();

    if !report.appeared.is_empty() {
        println!("Appeared:");
        for entry in &report.appeared {
            println!(
                "  + {} {} [{}] resp={:.4} ({})",
                entry.file,
                entry.function_name,
                entry.dimension,
                entry.responsibility,
                entry.detail
            );
        }
        println!();
    }

    if !report.disappeared.is_empty() {
        println!("Disappeared:");
        for entry in &report.disappeared {
            println!(
                "  - {} {} [{}] resp={:.4} ({})",
                entry.file,
                entry.function_name,
                entry.dimension,
                entry.responsibility,
                entry.detail
            );
        }
        println!();
    }

    if !report.changed.is_empty() {
        println!("Changed:");
        for entry in &report.changed {
            let arrow = match entry.assessment {
                descendit::Assessment::Improved => "v",
                descendit::Assessment::Regressed => "^",
                descendit::Assessment::Unchanged => "=",
            };
            println!(
                "  {arrow} {} {} [{}] {:.4} -> {:.4} (delta: {:+.4})",
                entry.file,
                entry.function_name,
                entry.dimension,
                entry.before_responsibility,
                entry.after_responsibility,
                entry.delta,
            );
        }
    }
}

fn resolve_batch_semantics(
    paths: &[PathBuf],
    socket: Option<&Path>,
) -> anyhow::Result<Vec<Option<descendit::SemanticOverlay>>> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        semantic_runtime::run_ra_analysis_batch(paths, socket)
    }))
    .unwrap_or_else(|payload| {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        Err(anyhow::anyhow!("rust-analyzer panicked: {msg}"))
    }) {
        Ok(batch) => {
            let map: std::collections::HashMap<PathBuf, descendit::SemanticOverlay> = batch
                .into_iter()
                .map(|(p, data)| {
                    let json = serde_json::to_string(&data)?;
                    let data: descendit::SemanticData = serde_json::from_str(&json)?;
                    Ok((p, descendit::SemanticOverlay::from_data(&data)))
                })
                .collect::<anyhow::Result<_>>()?;
            Ok(paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .ok()
                        .and_then(|c| map.get(&c).cloned())
                        .or_else(|| map.get(p).cloned())
                })
                .collect())
        }
        Err(e) => Err(e.context("semantic analysis failed")),
    }
}

fn run_analyze_multi(
    paths: &[PathBuf],
    socket: Option<&Path>,
) -> anyhow::Result<()> {
    let policy = descendit::CompliancePolicy::default();
    let semantic_overlays = resolve_batch_semantics(paths, socket)?;

    let targets: Vec<descendit::CorpusExperimentTarget> = paths
        .iter()
        .zip(semantic_overlays)
        .map(|(path, semantic)| {
            let analysis = descendit::analyze_path(path)?;
            if analysis.files_analyzed == 0 {
                anyhow::bail!("no .rs files found in {}", path.display());
            }
            Ok(descendit::CorpusExperimentTarget {
                label: path.display().to_string(),
                analysis,
                semantic,
            })
        })
        .collect::<anyhow::Result<_>>()?;

    let run = descendit::run_corpus_experiment(&targets, &policy);
    println!("{}", serde_json::to_string_pretty(&run)?);

    Ok(())
}

fn run_heatmap(
    path: &Path,
    policy_path: Option<&Path>,
    json: bool,
    tree: bool,
    top: Option<usize>,
    semantic: Option<&descendit::SemanticOverlay>,
) -> anyhow::Result<()> {
    let mut report = descendit::analyze_path(path)?;
    if report.files_analyzed == 0 {
        anyhow::bail!("no .rs files found in {}", path.display());
    }
    if let Some(overlay) = semantic {
        report.semantic = Some(descendit::SemanticSummary::from_overlay(overlay));
    }

    let policy = load_policy(policy_path)?;
    let cr = descendit::compute_compliance_with_semantic(&report, &policy, semantic);

    if cr.heatmap.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No loss hotspots — all dimensions at 0.0 loss.");
        }
        return Ok(());
    }

    let entries: &[descendit::HeatmapEntry] = &cr.heatmap;
    let truncated;
    let entries = if let Some(n) = top {
        truncated = entries.iter().take(n).cloned().collect::<Vec<_>>();
        &truncated
    } else {
        entries
    };

    if tree {
        let roots = descendit::build_heatmap_tree(entries);
        if json {
            println!("{}", serde_json::to_string_pretty(&roots)?);
        } else {
            print_heatmap_tree(&roots);
        }
    } else if json {
        println!("{}", serde_json::to_string_pretty(entries)?);
    } else {
        print_flat_heatmap(entries);
    }

    Ok(())
}

fn print_flat_heatmap(entries: &[descendit::HeatmapEntry]) {
    let mut current_file = String::new();
    for entry in entries {
        if entry.file != current_file {
            if !current_file.is_empty() {
                println!();
            }
            current_file = entry.file.clone();
            println!("{current_file}");
        }

        let bar_len = (entry.responsibility * 10.0).round() as usize;
        let bar: String = "@".repeat(bar_len.min(10));
        let pad: String = ".".repeat(10 - bar_len.min(10));

        let tag = format!("{} ({:.3})", entry.dimension, entry.responsibility);

        println!(
            "  L{:<4} {:<30} {}{} {}",
            entry.line, entry.function_name, bar, pad, tag
        );
    }
}

fn run_heatmap_multi(
    paths: &[PathBuf],
    policy_path: Option<&Path>,
    json: bool,
    tree: bool,
    top: Option<usize>,
    socket: Option<&Path>,
) -> anyhow::Result<()> {
    let policy = load_policy(policy_path)?;
    let semantic_overlays = resolve_batch_semantics(paths, socket)?;

    let targets: Vec<(
        String,
        descendit::AnalysisReport,
        Option<descendit::SemanticOverlay>,
    )> = paths
        .iter()
        .zip(semantic_overlays)
        .map(|(path, semantic)| {
            let mut report = descendit::analyze_path(path)?;
            if report.files_analyzed == 0 {
                anyhow::bail!("no .rs files found in {}", path.display());
            }
            if let Some(ref overlay) = semantic {
                report.semantic = Some(descendit::SemanticSummary::from_overlay(overlay));
            }
            Ok((path.display().to_string(), report, semantic))
        })
        .collect::<anyhow::Result<_>>()?;

    let mut builder = descendit::NormalizationContextBuilder::default();
    for (_, report, _) in &targets {
        builder.observe_report(report);
    }
    let norm_ctx = builder.build();

    if json {
        print_heatmap_multi_json(&targets, &policy, &norm_ctx, tree, top)?;
    } else {
        print_heatmap_multi_human(&targets, &policy, &norm_ctx, tree, top);
    }

    Ok(())
}

fn print_heatmap_multi_json(
    targets: &[(
        String,
        descendit::AnalysisReport,
        Option<descendit::SemanticOverlay>,
    )],
    policy: &descendit::CompliancePolicy,
    norm_ctx: &descendit::NormalizationContext,
    tree: bool,
    top: Option<usize>,
) -> anyhow::Result<()> {
    let mut results = Vec::new();
    for (label, report, semantic) in targets {
        let cr =
            descendit::compute_compliance_with_context(report, policy, norm_ctx, semantic.as_ref());
        let entries = if let Some(n) = top {
            cr.heatmap.into_iter().take(n).collect::<Vec<_>>()
        } else {
            cr.heatmap
        };
        let heatmap_data = if tree {
            serde_json::to_value(descendit::build_heatmap_tree(&entries))?
        } else {
            serde_json::to_value(&entries)?
        };
        results.push(serde_json::json!({
            "label": label,
            "heatmap": heatmap_data,
        }));
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

fn print_heatmap_multi_human(
    targets: &[(
        String,
        descendit::AnalysisReport,
        Option<descendit::SemanticOverlay>,
    )],
    policy: &descendit::CompliancePolicy,
    norm_ctx: &descendit::NormalizationContext,
    tree: bool,
    top: Option<usize>,
) {
    for (i, (label, report, semantic)) in targets.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("=== {label} ===");
        let cr =
            descendit::compute_compliance_with_context(report, policy, norm_ctx, semantic.as_ref());
        if cr.heatmap.is_empty() {
            println!("No loss hotspots.");
            continue;
        }
        let entries = if let Some(n) = top {
            cr.heatmap.into_iter().take(n).collect::<Vec<_>>()
        } else {
            cr.heatmap
        };
        if tree {
            let roots = descendit::build_heatmap_tree(&entries);
            print_heatmap_tree(&roots);
        } else {
            print_flat_heatmap(&entries);
        }
    }
}

fn print_heatmap_tree(roots: &[descendit::HeatmapTreeNode]) {
    for (i, root) in roots.iter().enumerate() {
        let is_last = i == roots.len() - 1;
        print_tree_node(root, "", is_last, true);
    }
}

fn print_tree_node(node: &descendit::HeatmapTreeNode, prefix: &str, is_last: bool, is_root: bool) {
    let label = format!("{} ({:.4})", node.segment, node.responsibility);

    if is_root {
        println!("{label}");
    } else {
        let connector = if is_last { "`-- " } else { "|-- " };
        println!("{prefix}{connector}{label}");
    }

    let child_prefix = if is_root {
        prefix.to_string()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}|   ")
    };

    if node.children.is_empty() && !node.dimension_responsibilities.is_empty() {
        // Leaf node: print per-dimension breakdown.
        let dims: Vec<String> = node
            .dimension_responsibilities
            .iter()
            .map(|(dim, val)| format!("{dim}: {val:.4}"))
            .collect();
        println!("{child_prefix}{}", dims.join("  "));
    }

    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == node.children.len() - 1;
        print_tree_node(child, &child_prefix, child_is_last, false);
    }
}




// ---------------------------------------------------------------------------
// Guide
// ---------------------------------------------------------------------------

fn run_guide() {
    print!(include_str!("guide.md"));
}

// ---------------------------------------------------------------------------
// Explore (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "explore")]
fn run_explore(
    path: &Path,
    policy_path: Option<&Path>,
    semantic: Option<&descendit::SemanticOverlay>,
) -> anyhow::Result<()> {
    descendit::explore::run_explore(path, policy_path, semantic)
}

#[cfg(feature = "explore")]
fn run_explore_multi(
    paths: &[PathBuf],
    policy_path: Option<&Path>,
    socket: Option<&Path>,
) -> anyhow::Result<()> {
    let policy = load_policy(policy_path)?;
    let semantic_overlays = resolve_batch_semantics(paths, socket)?;

    let targets: Vec<(
        String,
        descendit::AnalysisReport,
        Option<descendit::SemanticOverlay>,
    )> = paths
        .iter()
        .zip(semantic_overlays)
        .map(|(path, semantic)| {
            let mut report = descendit::analyze_path(path)?;
            if let Some(ref overlay) = semantic {
                report.semantic = Some(descendit::SemanticSummary::from_overlay(overlay));
            }
            Ok((path.display().to_string(), report, semantic))
        })
        .collect::<anyhow::Result<_>>()?;

    let mut builder = descendit::NormalizationContextBuilder::default();
    for (_, report, _) in &targets {
        builder.observe_report(report);
    }
    let norm_ctx = builder.build();

    let mut all_heatmap = Vec::new();
    for (prefix, report, semantic) in &targets {
        let cr = descendit::compute_compliance_with_context(
            report,
            &policy,
            &norm_ctx,
            semantic.as_ref(),
        );
        for mut entry in cr.heatmap {
            entry.file = format!("{prefix}/{}", entry.file);
            all_heatmap.push(entry);
        }
    }

    if all_heatmap.is_empty() {
        println!("No loss hotspots -- all dimensions at 0.0 loss. Nothing to explore.");
        return Ok(());
    }

    let roots = descendit::build_heatmap_tree(&all_heatmap);
    descendit::explore::run_explore_with_tree(roots)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

fn run_policy(dump_default: bool) -> anyhow::Result<()> {
    if dump_default {
        let policy = descendit::CompliancePolicy::default();
        println!("{}", serde_json::to_string_pretty(&policy)?);
    } else {
        println!("Use --dump-default to emit the default compliance policy as JSON.");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn analyze_defaults_parse() {
        let cli = Cli::try_parse_from(["descendit", "analyze", "."]).expect("parse analyze");
        match cli.command {
            Command::Analyze {
                paths,
                semantic_path,
                ..
            } => {
                assert_eq!(paths, vec![PathBuf::from(".")]);
                assert!(semantic_path.is_none());
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn analyze_accepts_semantic_path() {
        let cli = Cli::try_parse_from([
            "descendit",
            "analyze",
            ".",
            "--semantic-path",
            "target/descendit/semantic.json",
        ])
        .expect("parse analyze");

        match cli.command {
            Command::Analyze { semantic_path, .. } => {
                assert_eq!(
                    semantic_path,
                    Some(PathBuf::from("target/descendit/semantic.json"))
                );
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn analyze_rejects_semantic_flag() {
        // --semantic should no longer be a valid flag
        let result = Cli::try_parse_from(["descendit", "analyze", ".", "--semantic", "off"]);
        assert!(result.is_err(), "--semantic should not be accepted");
    }

    // --- CLI flag parse tests ---

    #[test]
    fn analyze_agent_flag_parses() {
        let cli = Cli::try_parse_from(["descendit", "analyze", ".", "--agent"])
            .expect("parse analyze --agent");
        match cli.command {
            Command::Analyze { agent, top, .. } => {
                assert!(agent);
                assert_eq!(top, 10);
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn analyze_agent_with_top_parses() {
        let cli = Cli::try_parse_from(["descendit", "analyze", ".", "--agent", "--top", "5"])
            .expect("parse analyze --agent --top 5");
        match cli.command {
            Command::Analyze { agent, top, .. } => {
                assert!(agent);
                assert_eq!(top, 5);
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn diff_compliance_flag_parses() {
        let cli = Cli::try_parse_from(["descendit", "diff", "a.json", "b.json", "--compliance"])
            .expect("parse diff --compliance");
        match cli.command {
            Command::Diff {
                compliance,
                heatmap,
                ..
            } => {
                assert!(compliance);
                assert!(!heatmap);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_compliance_conflicts_with_loss_vector() {
        let result = Cli::try_parse_from([
            "descendit",
            "diff",
            "a.json",
            "b.json",
            "--compliance",
            "--loss-vector",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn diff_heatmap_flag_parses() {
        let cli = Cli::try_parse_from(["descendit", "diff", "a.json", "b.json", "--heatmap"])
            .expect("parse diff --heatmap");
        match cli.command {
            Command::Diff { heatmap, json, .. } => {
                assert!(heatmap);
                assert!(!json);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_heatmap_and_compliance_coexist() {
        let cli = Cli::try_parse_from([
            "descendit",
            "diff",
            "a.json",
            "b.json",
            "--heatmap",
            "--compliance",
        ])
        .expect("parse diff --heatmap --compliance");
        match cli.command {
            Command::Diff {
                heatmap,
                compliance,
                ..
            } => {
                assert!(heatmap);
                assert!(compliance);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_policy_flag_parses() {
        let cli = Cli::try_parse_from([
            "descendit",
            "diff",
            "a.json",
            "b.json",
            "--compliance",
            "--policy",
            "custom.json",
        ])
        .expect("parse diff --compliance --policy");
        match cli.command {
            Command::Diff { policy, .. } => {
                assert_eq!(policy, Some(PathBuf::from("custom.json")));
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_json_flag_parses() {
        let cli = Cli::try_parse_from([
            "descendit",
            "diff",
            "a.json",
            "b.json",
            "--heatmap",
            "--json",
        ])
        .expect("parse diff --heatmap --json");
        match cli.command {
            Command::Diff { heatmap, json, .. } => {
                assert!(heatmap);
                assert!(json);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn analyze_multi_path_parses() {
        let cli =
            Cli::try_parse_from(["descendit", "analyze", "a", "b"]).expect("parse analyze a b");
        match cli.command {
            Command::Analyze { paths, .. } => {
                assert_eq!(paths, vec![PathBuf::from("a"), PathBuf::from("b")]);
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn heatmap_multi_path_parses() {
        let cli =
            Cli::try_parse_from(["descendit", "heatmap", "a", "b"]).expect("parse heatmap a b");
        match cli.command {
            Command::Heatmap { paths, .. } => {
                assert_eq!(paths, vec![PathBuf::from("a"), PathBuf::from("b")]);
            }
            _ => panic!("expected heatmap command"),
        }
    }
}

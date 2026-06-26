//! CLI for speculative code-design tools built on descendit artifacts.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Parser, Subcommand};

#[cfg(all(unix, feature = "semantic"))]
#[path = "../server_protocol.rs"]
mod server_protocol;

/// Speculative design tools for Rust code.
#[derive(Debug, Parser)]
#[command(
    name = "descendit-design",
    bin_name = "descendit-design",
    version,
    about = "Speculative Rust code-design tools built on descendit artifacts"
)]
struct Cli {
    /// Connect to a running descendit watch server via this Unix socket path.
    #[arg(long, global = true)]
    sock: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Discover type/trait graph simplification candidates.
    TypeTrait {
        #[command(subcommand)]
        command: TypeTraitCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TypeTraitCommand {
    /// Discover rewrite candidates from a JSON query.
    Discover {
        /// Query JSON path, or '-' for stdin.
        #[arg(long)]
        query: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::TypeTrait { command } => dispatch_type_trait(command, cli.sock.as_deref())?,
    }
    Ok(())
}

fn dispatch_type_trait(command: TypeTraitCommand, socket: Option<&Path>) -> anyhow::Result<()> {
    match command {
        TypeTraitCommand::Discover { query } => run_type_trait_discover(&query, socket),
    }
}

fn run_type_trait_discover(query_path: &Path, socket: Option<&Path>) -> anyhow::Result<()> {
    let query = read_type_trait_query(query_path)?;
    let data = load_type_trait_semantic_data(&query, socket)?;
    let report = descendit::type_trait_discover::discover_type_trait_rewrites(&data, &query)?;
    match query.emit {
        descendit::type_trait_discover::TypeTraitEmit::Text => {
            print!("{}", report.render_text());
        }
        descendit::type_trait_discover::TypeTraitEmit::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}

/// Read a type/trait design query from a file or stdin.
///
/// `--query -` is intentionally supported so agents can synthesize a query and
/// pipe it into the design tool without managing temporary files.
fn read_type_trait_query(
    path: &Path,
) -> anyhow::Result<descendit::type_trait_discover::TypeTraitQuery> {
    let mut json = String::new();
    if path == Path::new("-") {
        std::io::stdin().read_to_string(&mut json)?;
    } else {
        json = std::fs::read_to_string(path)?;
    }
    serde_json::from_str(&json).map_err(anyhow::Error::from)
}

#[cfg(feature = "semantic")]
fn load_type_trait_semantic_data(
    query: &descendit::type_trait_discover::TypeTraitQuery,
    socket: Option<&Path>,
) -> anyhow::Result<descendit::SemanticData> {
    if let Some(path) = &query.semantic_path {
        let json = std::fs::read_to_string(path)?;
        return serde_json::from_str(&json).map_err(anyhow::Error::from);
    }

    let data = run_ra_type_trait_data(&query.path, socket)?;
    let json = serde_json::to_string(&data)?;
    serde_json::from_str(&json).map_err(anyhow::Error::from)
}

#[cfg(not(feature = "semantic"))]
fn load_type_trait_semantic_data(
    query: &descendit::type_trait_discover::TypeTraitQuery,
    _socket: Option<&Path>,
) -> anyhow::Result<descendit::SemanticData> {
    if let Some(path) = &query.semantic_path {
        let json = std::fs::read_to_string(path)?;
        return serde_json::from_str(&json).map_err(anyhow::Error::from);
    }
    anyhow::bail!(
        "semantic analysis is required for type-trait discovery unless `semantic_path` is set. \
         Rebuild with `cargo install descendit` (default features)."
    );
}

#[cfg(feature = "semantic")]
fn run_ra_type_trait_data(
    analysis_path: &Path,
    socket: Option<&Path>,
) -> anyhow::Result<descendit_ra::SemanticData> {
    catch_ra_panic(|| run_ra_type_trait_data_unchecked(analysis_path, socket))
}

#[cfg(feature = "semantic")]
fn run_ra_type_trait_data_unchecked(
    analysis_path: &Path,
    socket: Option<&Path>,
) -> anyhow::Result<descendit_ra::SemanticData> {
    let manifest =
        find_nearest_manifest(manifest_search_start(analysis_path)).ok_or_else(|| {
            anyhow::anyhow!("could not find Cargo.toml near {}", analysis_path.display())
        })?;
    let manifest_dir = manifest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml has no parent directory"))?;

    #[cfg(unix)]
    if let Some(socket_path) = socket {
        return analyze_with_domains(
            socket_path,
            manifest_dir,
            descendit_ra::AnalysisDomains::type_traits(),
        )
        .context("server-backed semantic analysis failed");
    }

    #[cfg(not(unix))]
    if socket.is_some() {
        anyhow::bail!("socket-based analysis is only supported on Unix platforms");
    }

    descendit_ra::analyze_with_domains(manifest_dir, descendit_ra::AnalysisDomains::type_traits())
        .with_context(|| {
            format!(
                "rust-analyzer semantic analysis failed for {}.",
                manifest_dir.display()
            )
        })
}

/// Request selected semantic domains from an existing descendit watch server.
///
/// The design binary only needs analysis, not lifecycle control, so it keeps a
/// narrow client here instead of depending on the main binary's reap client.
#[cfg(all(unix, feature = "semantic"))]
fn analyze_with_domains(
    socket_path: &Path,
    manifest_dir: &Path,
    domains: descendit_ra::AnalysisDomains,
) -> anyhow::Result<descendit_ra::SemanticData> {
    use std::io::{BufReader, BufWriter};
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(300)))?;
    let mut reader = BufReader::new(&stream);
    let mut writer = BufWriter::new(&stream);

    server_protocol::write_message(
        &mut writer,
        &server_protocol::Request::Analyze {
            manifest_dir: manifest_dir.to_owned(),
            domains,
        },
    )?;

    let response: server_protocol::Response = server_protocol::read_message(&mut reader)?;
    match response {
        server_protocol::Response::SemanticData(data) => Ok(data),
        server_protocol::Response::Error { message } => anyhow::bail!("server error: {message}"),
        server_protocol::Response::Ok => anyhow::bail!("unexpected Ok response to Analyze request"),
    }
}

#[cfg(feature = "semantic")]
fn manifest_search_start(path: &Path) -> &Path {
    if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    }
}

#[cfg(feature = "semantic")]
fn find_nearest_manifest(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    for _ in 0..32 {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// Convert RA panics into ordinary CLI errors.
///
/// The design tool is exploratory and often points at arbitrary codebases, so
/// rust-analyzer failures should report as input failures rather than aborting
/// the process.
#[cfg(feature = "semantic")]
fn catch_ra_panic<F, T>(f: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(result) => result,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            Err(anyhow::anyhow!("rust-analyzer panicked: {msg}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_trait_discover_stdin_query_parses() {
        let cli =
            Cli::try_parse_from(["descendit-design", "type-trait", "discover", "--query", "-"])
                .expect("parse type-trait discover");
        match cli.command {
            Command::TypeTrait {
                command: TypeTraitCommand::Discover { query },
            } => {
                assert_eq!(query, PathBuf::from("-"));
            }
        }
    }
}

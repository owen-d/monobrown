use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use clap::ValueEnum;

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum SemanticMode {
    #[default]
    Require,
    Auto,
    Off,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Run the RA backend (or load pre-existing data) and return an overlay.
///
/// When `explicit_semantic` is given, that file is loaded directly.
/// Otherwise `semantic_mode` controls whether the RA backend is invoked.
/// When `socket` is `Some`, the analysis is delegated to a running server
/// instead of spawning a local RA session.
pub(crate) fn ensure_semantic_data(
    explicit_semantic: Option<&Path>,
    analysis_path: Option<&Path>,
    semantic_mode: SemanticMode,
    _socket: Option<&Path>,
) -> anyhow::Result<Option<descendit::SemanticOverlay>> {
    if explicit_semantic.is_some() {
        return resolve_semantic(explicit_semantic, analysis_path);
    }

    match semantic_mode {
        SemanticMode::Off => Ok(None),
        SemanticMode::Auto => {
            let Some(path) = analysis_path else {
                return Ok(None);
            };
            #[cfg(feature = "semantic")]
            {
                match run_ra_analysis(path, _socket) {
                    Ok(overlay) => Ok(Some(overlay)),
                    Err(e) => {
                        eprintln!("warning: semantic analysis failed: {e}");
                        resolve_semantic(None, Some(path))
                    }
                }
            }
            #[cfg(not(feature = "semantic"))]
            {
                resolve_semantic(None, Some(path))
            }
        }
        SemanticMode::Require => {
            let path = analysis_path.ok_or_else(|| anyhow!("no analysis path provided"))?;
            #[cfg(feature = "semantic")]
            {
                let overlay = run_ra_analysis(path, _socket)?;
                Ok(Some(overlay))
            }
            #[cfg(not(feature = "semantic"))]
            {
                let _ = path;
                anyhow::bail!(
                    "semantic analysis is required but the `semantic` feature is not enabled. \
                     Rebuild with `cargo install descendit` (default features) or pass --semantic off."
                );
            }
        }
    }
}

/// Resolve pre-existing semantic data without running a backend pipeline.
///
/// Used for saved analysis reports where the semantic JSON was already
/// produced during the original analysis.
pub(crate) fn ensure_saved_semantic_data(
    explicit_semantic: Option<&Path>,
    anchor: Option<&Path>,
    semantic_mode: SemanticMode,
) -> anyhow::Result<Option<descendit::SemanticOverlay>> {
    if explicit_semantic.is_some() {
        return resolve_semantic(explicit_semantic, anchor);
    }

    match semantic_mode {
        SemanticMode::Off => Ok(None),
        SemanticMode::Auto => resolve_semantic(None, anchor),
        SemanticMode::Require => resolve_semantic(None, anchor)?
            .ok_or_else(|| {
                anyhow!(
                    "semantic enrichment is required for saved analysis reports, but no \
                     semantic data was found. Pass --semantic-path <path> or rerun against \
                     source with --semantic=require."
                )
            })
            .map(Some),
    }
}

// ---------------------------------------------------------------------------
// Resolution helpers
// ---------------------------------------------------------------------------

/// Load an explicit semantic file, or search near `anchor` for one.
pub(crate) fn resolve_semantic(
    explicit: Option<&Path>,
    anchor: Option<&Path>,
) -> anyhow::Result<Option<descendit::SemanticOverlay>> {
    if let Some(path) = explicit {
        return load_semantic_overlay(path).map(Some);
    }

    let Some(anchor) = anchor else {
        return Ok(None);
    };

    resolve_near_anchor(anchor)
}

fn resolve_near_anchor(anchor: &Path) -> anyhow::Result<Option<descendit::SemanticOverlay>> {
    let start = if anchor.is_file() {
        match anchor.parent() {
            Some(parent) => parent,
            None => return Ok(None),
        }
    } else {
        anchor
    };

    let mut dir = start;
    let mut depth = 0u32;
    loop {
        if depth >= 32 {
            return Ok(None);
        }
        let candidate = dir.join("target/descendit/semantic.json");
        if candidate.is_file() {
            return match load_semantic_overlay(&candidate) {
                Ok(overlay) => Ok(Some(overlay)),
                Err(e) => {
                    eprintln!("warning: failed to load semantic data: {e:#}");
                    Ok(None)
                }
            };
        }
        dir = match dir.parent() {
            Some(parent) => parent,
            None => return Ok(None),
        };
        depth += 1;
    }
}

fn load_semantic_overlay(path: &Path) -> anyhow::Result<descendit::SemanticOverlay> {
    descendit::SemanticOverlay::load(path)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("failed to load semantic data from {}", path.display()))
}

// ---------------------------------------------------------------------------
// RA backend (requires `semantic` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "semantic")]
fn run_ra_analysis(
    analysis_path: &Path,
    socket: Option<&Path>,
) -> anyhow::Result<descendit::SemanticOverlay> {
    let manifest = find_nearest_manifest(analysis_path.parent().unwrap_or(analysis_path))
        .ok_or_else(|| anyhow!("could not find Cargo.toml near {}", analysis_path.display()))?;
    let manifest_dir = manifest
        .parent()
        .ok_or_else(|| anyhow!("Cargo.toml has no parent directory"))?;

    #[cfg(unix)]
    if let Some(socket_path) = socket {
        let ra_data = crate::client::analyze(socket_path, manifest_dir)
            .context("server-backed semantic analysis failed")?;
        // Roundtrip through JSON to convert descendit_ra::output::SemanticData
        // into descendit::SemanticData (structurally identical, separate types).
        let json = serde_json::to_string(&ra_data)?;
        let data: descendit::SemanticData = serde_json::from_str(&json)?;
        return Ok(descendit::SemanticOverlay::from_data(&data));
    }

    #[cfg(not(unix))]
    if socket.is_some() {
        anyhow::bail!("socket-based analysis is only supported on Unix platforms");
    }

    let json = descendit_ra::analyze_to_json(manifest_dir)
        .context("rust-analyzer semantic analysis failed")?;

    let data: descendit::SemanticData =
        serde_json::from_str(&json).context("failed to parse RA semantic output")?;
    Ok(descendit::SemanticOverlay::from_data(&data))
}

// ---------------------------------------------------------------------------
// Manifest discovery
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "semantic"), allow(dead_code))]
pub(crate) fn find_nearest_manifest(start: &Path) -> Option<PathBuf> {
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

// ---------------------------------------------------------------------------
// Batch RA analysis
// ---------------------------------------------------------------------------

/// Run semantic analysis for multiple crate paths, returning data for each.
///
/// When `socket` is provided, each path is analyzed via the running server.
/// When `socket` is `None`, a single `RaSession` is loaded at the first
/// path's location (which loads the entire workspace), and
/// [`extract_for_subcrate`](descendit_ra::RaSession::extract_for_subcrate) is
/// called for each path.
#[cfg(feature = "semantic")]
pub(crate) fn run_ra_analysis_batch(
    paths: &[PathBuf],
    socket: Option<&Path>,
) -> anyhow::Result<Vec<(PathBuf, descendit_ra::SemanticData)>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    #[cfg(unix)]
    if let Some(socket_path) = socket {
        return paths
            .iter()
            .map(|path| {
                let manifest = find_nearest_manifest(path.parent().unwrap_or(path))
                    .ok_or_else(|| anyhow!("could not find Cargo.toml near {}", path.display()))?;
                let manifest_dir = manifest
                    .parent()
                    .ok_or_else(|| anyhow!("Cargo.toml has no parent directory"))?;
                let data =
                    crate::client::analyze(socket_path, manifest_dir).with_context(|| {
                        format!(
                            "server-backed semantic analysis failed for {}",
                            path.display()
                        )
                    })?;
                Ok((path.clone(), data))
            })
            .collect();
    }

    #[cfg(not(unix))]
    if socket.is_some() {
        anyhow::bail!("socket-based analysis is only supported on Unix platforms");
    }

    // Offline mode: load a single workspace session, extract for each subcrate.
    let first = &paths[0];
    let first_manifest = find_nearest_manifest(first.parent().unwrap_or(first))
        .ok_or_else(|| anyhow!("could not find Cargo.toml near {}", first.display()))?;
    let first_manifest_dir = first_manifest
        .parent()
        .ok_or_else(|| anyhow!("Cargo.toml has no parent directory"))?;

    let mut session = descendit_ra::RaSession::load(first_manifest_dir)
        .context("failed to load RA workspace session")?;

    paths
        .iter()
        .map(|path| {
            let manifest = find_nearest_manifest(path.parent().unwrap_or(path))
                .ok_or_else(|| anyhow!("could not find Cargo.toml near {}", path.display()))?;
            let manifest_dir = manifest
                .parent()
                .ok_or_else(|| anyhow!("Cargo.toml has no parent directory"))?;
            let data = session
                .extract_for_subcrate(manifest_dir)
                .with_context(|| format!("semantic extraction failed for {}", path.display()))?;
            Ok((path.clone(), data))
        })
        .collect()
}

#[cfg(not(feature = "semantic"))]
pub(crate) fn run_ra_analysis_batch(
    _paths: &[PathBuf],
    _socket: Option<&Path>,
) -> anyhow::Result<Vec<(PathBuf, descendit::SemanticData)>> {
    anyhow::bail!(
        "batch semantic analysis requires the `semantic` feature. \
         Rebuild with default features or pass --semantic off."
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn saved_analysis_require_fails_without_semantic_data() {
        let temp = tempdir().expect("tempdir");
        let report_path = temp.path().join("analysis.json");
        std::fs::write(&report_path, "{}").expect("report placeholder");

        let err = ensure_saved_semantic_data(None, Some(&report_path), SemanticMode::Require)
            .expect_err("require should fail");
        assert!(err.to_string().contains("--semantic-path"));
    }

    #[test]
    fn find_nearest_manifest_walks_up() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"\n")
            .expect("write manifest");
        std::fs::create_dir_all(root.join("src/deep")).expect("create nested dirs");

        let found = find_nearest_manifest(&root.join("src/deep"));
        assert_eq!(found, Some(root.join("Cargo.toml")),);
    }

    #[test]
    fn find_nearest_manifest_returns_none_at_root() {
        // Starting from a path with no Cargo.toml anywhere should return None.
        let temp = tempdir().expect("tempdir");
        let empty = temp.path().join("empty");
        std::fs::create_dir_all(&empty).expect("create empty dir");
        assert!(find_nearest_manifest(&empty).is_none());
    }
}

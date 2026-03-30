//! Semantic analysis backend using `ra_ap_*` (rust-analyzer) crates.
//!
//! Extracts cross-module call edges and resolved type/function cardinalities
//! from a Cargo workspace, producing structured `SemanticData` JSON. Runs on
//! stable Rust with no external prerequisites.
//!
//! # Usage
//!
//! ```no_run
//! use std::path::Path;
//! let json = descendit_ra::analyze_to_json(Path::new("crates/my-crate")).unwrap();
//! ```

mod call_graph;
mod cardinality;
mod output;
pub mod session;

use std::path::Path;

use anyhow::Context;
use ra_ap_hir::HasSource;
use ra_ap_hir::db::HirDatabase;
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_syntax::AstNode;

pub use output::{CallEdge, ResolvedFunctionCardinality, ResolvedTypeCardinality, SemanticData};
pub use session::RaSession;

/// Analyze a crate and return semantic data as a JSON string.
///
/// This is the primary entry point. `manifest_dir` should point to the
/// crate directory (containing `Cargo.toml`) or a workspace member.
pub fn analyze_to_json(manifest_dir: &Path) -> anyhow::Result<String> {
    let data = analyze(manifest_dir)?;
    serde_json::to_string_pretty(&data).context("failed to serialize SemanticData")
}

/// Analyze a crate and return structured semantic data.
pub fn analyze(manifest_dir: &Path) -> anyhow::Result<SemanticData> {
    let manifest_dir = std::fs::canonicalize(manifest_dir)
        .with_context(|| format!("failed to canonicalize {}", manifest_dir.display()))?;

    let cargo_config = CargoConfig {
        sysroot: Some(RustLibSource::Discover),
        all_targets: true,
        ..Default::default()
    };

    let load_config = LoadCargoConfig {
        load_out_dirs_from_check: true,
        with_proc_macro_server: ProcMacroServerChoice::None,
        prefill_caches: false,
        proc_macro_processes: 0,
    };

    let no_progress = |_: String| {};
    let (db, vfs, _proc_macro) = load_workspace_at(
        manifest_dir.as_ref(),
        &cargo_config,
        &load_config,
        &no_progress,
    )
    .context("failed to load workspace")?;

    // Attach the database to thread-local storage so the type solver can
    // access it during inference (required by salsa 0.25+ / ra_ap_hir_ty).
    ra_ap_hir::attach_db(&db, || extract_semantic_data(&db, &vfs, &manifest_dir))
}

pub(crate) fn extract_semantic_data(
    db: &dyn HirDatabase,
    vfs: &ra_ap_vfs::Vfs,
    manifest_dir: &Path,
) -> anyhow::Result<SemanticData> {
    let crates = find_target_crates(db, vfs, manifest_dir)?;
    let workspace_root = find_workspace_root(manifest_dir)?;

    let crate_name = crates
        .first()
        .and_then(|k| k.display_name(db))
        .map_or_else(String::new, |n| n.to_string());

    let mut data = SemanticData {
        crate_name,
        type_cardinalities: Vec::new(),
        function_cardinalities: Vec::new(),
        call_edges: Vec::new(),
    };

    // Walk all target crates (lib + bins) in the package.
    for krate in &crates {
        walk_crate(db, vfs, *krate, &workspace_root, &mut data);
    }

    Ok(data)
}

fn walk_crate(
    db: &dyn HirDatabase,
    vfs: &ra_ap_vfs::Vfs,
    krate: ra_ap_hir::Crate,
    workspace_root: &Path,
    data: &mut SemanticData,
) {
    for module in krate.modules(db) {
        let editioned_file_id = match module.definition_source_file_id(db).file_id() {
            Some(id) => id,
            None => continue,
        };
        let file_id = editioned_file_id.file_id(db);

        let file_path = {
            let vfs_path = vfs.file_path(file_id);
            match vfs_path.as_path() {
                Some(abs_path) => normalize_path(abs_path.as_ref(), workspace_root),
                None => continue,
            }
        };

        let module_path = module_path_string(db, module);

        for decl in module.declarations(db) {
            match decl {
                ra_ap_hir::ModuleDef::Adt(adt) => {
                    cardinality::process_adt(db, &adt, &file_path, &module_path, data);
                }
                ra_ap_hir::ModuleDef::Function(func) => {
                    cardinality::process_function(
                        db,
                        vfs,
                        &func,
                        &file_path,
                        &module_path,
                        workspace_root,
                        data,
                    );
                    call_graph::process_function(
                        db,
                        vfs,
                        &func,
                        &file_path,
                        &module_path,
                        workspace_root,
                        data,
                    );
                }
                _ => {}
            }
        }

        // Also walk impl blocks for methods.
        for impl_block in module.impl_defs(db) {
            for item in impl_block.items(db) {
                if let ra_ap_hir::AssocItem::Function(func) = item {
                    cardinality::process_function(
                        db,
                        vfs,
                        &func,
                        &file_path,
                        &module_path,
                        workspace_root,
                        data,
                    );
                    call_graph::process_function(
                        db,
                        vfs,
                        &func,
                        &file_path,
                        &module_path,
                        workspace_root,
                        data,
                    );
                }
            }
        }
    }
}

/// Find all crates (lib + bins) whose root file is inside `manifest_dir`.
fn find_target_crates(
    db: &dyn HirDatabase,
    vfs: &ra_ap_vfs::Vfs,
    manifest_dir: &Path,
) -> anyhow::Result<Vec<ra_ap_hir::Crate>> {
    let all_crates = ra_ap_hir::Crate::all(db);
    let mut targets = Vec::new();

    for krate in &all_crates {
        if let Some(name) = krate.display_name(db) {
            let name_str = name.to_string();
            if matches!(
                name_str.as_str(),
                "std" | "core" | "alloc" | "proc_macro" | "test"
            ) {
                continue;
            }
        }
        let root_file_id = krate.root_file(db);
        let vfs_path = vfs.file_path(root_file_id);
        if let Some(abs_path) = vfs_path.as_path() {
            let root_path: &std::path::Path = abs_path.as_ref();
            if root_path.starts_with(manifest_dir) {
                targets.push(*krate);
            }
        }
    }

    if targets.is_empty() {
        anyhow::bail!("could not find target crate for {}", manifest_dir.display());
    }
    Ok(targets)
}

pub(crate) fn find_workspace_root(manifest_dir: &Path) -> anyhow::Result<std::path::PathBuf> {
    let mut dir = manifest_dir;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.is_file() {
            let content = std::fs::read_to_string(&cargo_toml)?;
            if content.contains("[workspace]") {
                return Ok(dir.to_owned());
            }
        }
        dir = dir
            .parent()
            .with_context(|| "reached filesystem root without finding workspace")?;
    }
}

fn normalize_path(path: &Path, workspace_root: &Path) -> String {
    match path.strip_prefix(workspace_root) {
        Ok(relative) => relative.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

fn module_path_string(db: &dyn HirDatabase, module: ra_ap_hir::Module) -> String {
    let mut parts = Vec::new();
    let mut current = Some(module);
    while let Some(m) = current {
        if let Some(name) = m.name(db) {
            parts.push(name.as_str().to_owned());
        }
        current = m.parent(db);
    }
    parts.reverse();
    parts.join("::")
}

/// Compute the 1-based line number of a function within its source file.
///
/// Uses the `fn` keyword token position (not the start of the syntax node,
/// which includes doc comments and attributes) to match the semantic analysis
/// backend's convention of reporting the `fn` definition line.
pub(crate) fn function_line_number(
    vfs: &ra_ap_vfs::Vfs,
    db: &dyn HirDatabase,
    func: &ra_ap_hir::Function,
) -> usize {
    let Some(src) = func.source(db) else {
        return 0;
    };

    // Find the `fn` keyword token within the function syntax node.
    // The syntax node includes doc comments and attributes before the `fn`,
    // We report the line of the `fn` keyword itself, not preceding doc comments.
    let fn_offset = src
        .value
        .fn_token()
        .map(|tok| usize::from(tok.text_range().start()))
        .unwrap_or_else(|| usize::from(src.value.syntax().text_range().start()));

    // Read the full file to count newlines up to the offset.
    let editioned_file_id = src.file_id.original_file(db);
    let file_id = editioned_file_id.file_id(db);
    let path = vfs.file_path(file_id);
    let Some(abs_path) = path.as_path() else {
        return 0;
    };
    let Ok(file_bytes) = std::fs::read(abs_path.as_ref() as &Path) else {
        return 0;
    };

    file_bytes[..fn_offset.min(file_bytes.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

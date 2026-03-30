//! Persistent RA session for incremental re-analysis.
//!
//! [`RaSession`] wraps the `RootDatabase` and `Vfs` so that repeated
//! analyses of the same crate can benefit from salsa's incremental
//! recomputation instead of paying the full workspace-load cost each time.

use std::path::{Path, PathBuf};

use anyhow::Context;
use ra_ap_ide_db::{ChangeWithProcMacros, RootDatabase};
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_vfs::{self as vfs, Vfs, VfsPath};

use crate::output::SemanticData;

/// A reusable rust-analyzer session.
///
/// Holds onto the salsa database and VFS across multiple analysis runs so that
/// only files that actually changed on disk trigger recomputation.
pub struct RaSession {
    db: RootDatabase,
    vfs: Vfs,
    /// Local source files (under `manifest_dir`) that we re-read on each reload.
    local_file_ids: Vec<(vfs::FileId, VfsPath)>,
    manifest_dir: PathBuf,
    /// Root of the Cargo workspace (contains `[workspace]` in Cargo.toml).
    workspace_root: PathBuf,
    /// `true` until the first `reload_and_analyze` call completes.
    first_run: bool,
}

impl RaSession {
    /// Perform the expensive initial workspace load.
    ///
    /// This mirrors the setup in [`crate::analyze`] but retains the database
    /// and VFS for later incremental reuse.
    pub fn load(manifest_dir: &Path) -> anyhow::Result<Self> {
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

        // Collect all VFS files whose path falls under manifest_dir.
        // These are the "local" source files we will re-read on reload.
        let local_file_ids: Vec<(vfs::FileId, VfsPath)> = vfs
            .iter()
            .filter(|(_id, path)| {
                path.as_path().is_some_and(|abs| {
                    let p: &Path = abs.as_ref();
                    p.starts_with(&manifest_dir)
                })
            })
            .map(|(id, path)| (id, path.clone()))
            .collect();

        let workspace_root =
            crate::find_workspace_root(&manifest_dir).context("failed to find workspace root")?;

        Ok(Self {
            db,
            vfs,
            local_file_ids,
            manifest_dir,
            workspace_root,
            first_run: true,
        })
    }

    /// The canonicalized manifest directory this session was loaded from.
    pub fn manifest_dir(&self) -> &Path {
        &self.manifest_dir
    }

    /// The root of the Cargo workspace this session belongs to.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Re-read changed files from disk and produce fresh [`SemanticData`].
    ///
    /// On the first call after [`load`](Self::load) this just runs analysis
    /// (no reload needed). On subsequent calls it re-reads every local source
    /// file, lets the VFS detect which ones actually changed (content-hash
    /// dedup), and applies the delta to the salsa database before analyzing.
    pub fn reload_and_analyze(&mut self) -> anyhow::Result<SemanticData> {
        if self.first_run {
            self.first_run = false;
        } else {
            self.apply_disk_changes()?;
        }

        ra_ap_hir::attach_db(&self.db, || {
            crate::extract_semantic_data(&self.db, &self.vfs, &self.manifest_dir)
        })
    }

    /// Re-read changed files from disk and extract [`SemanticData`] for a
    /// specific subcrate within the loaded workspace.
    ///
    /// Unlike [`reload_and_analyze`](Self::reload_and_analyze), this filters
    /// to the crates rooted under the provided `manifest_dir` rather than the
    /// session's own manifest directory. The workspace must already be loaded
    /// (via [`load`](Self::load)) at a root that contains the subcrate.
    pub fn extract_for_subcrate(&mut self, manifest_dir: &Path) -> anyhow::Result<SemanticData> {
        let manifest_dir = std::fs::canonicalize(manifest_dir)
            .with_context(|| format!("failed to canonicalize {}", manifest_dir.display()))?;

        if self.first_run {
            self.first_run = false;
        } else {
            self.apply_disk_changes()?;
        }

        ra_ap_hir::attach_db(&self.db, || {
            crate::extract_semantic_data(&self.db, &self.vfs, &manifest_dir)
        })
    }

    /// Re-read local files from disk, push them into the VFS, and apply the
    /// resulting delta to the database.
    fn apply_disk_changes(&mut self) -> anyhow::Result<()> {
        // Re-derive the local file list each cycle so that files added to (or
        // removed from) the VFS since the last run are included.
        //
        // Known limitation: truly new files that were not part of the initial
        // workspace load (i.e. files the VFS has never seen) will not appear
        // here. Picking those up requires a full server restart because the VFS
        // only knows about files discovered during `load_workspace_at`.
        self.local_file_ids = self
            .vfs
            .iter()
            .filter(|(_id, path)| {
                path.as_path().is_some_and(|abs| {
                    let p: &Path = abs.as_ref();
                    p.starts_with(&self.manifest_dir)
                })
            })
            .map(|(id, path)| (id, path.clone()))
            .collect();

        // Re-read every local file. The VFS uses content hashing internally,
        // so unchanged files produce no delta.
        for (_, path) in &self.local_file_ids {
            let contents = path
                .as_path()
                .and_then(|abs| std::fs::read(abs.as_ref() as &Path).ok());
            self.vfs.set_file_contents(path.clone(), contents);
        }

        let changes = self.vfs.take_changes();
        if changes.is_empty() {
            return Ok(());
        }

        // Convert VFS changes into a ChangeWithProcMacros, replicating the
        // pattern from `load_crate_graph_into_db` in ra_ap_load_cargo.
        let mut analysis_change = ChangeWithProcMacros::default();
        for (_, file) in changes {
            match file.change {
                vfs::Change::Create(bytes, _) | vfs::Change::Modify(bytes, _) => {
                    if let Ok(text) = String::from_utf8(bytes) {
                        analysis_change.change_file(file.file_id, Some(text));
                    }
                }
                vfs::Change::Delete => {
                    analysis_change.change_file(file.file_id, None);
                }
            }
        }

        self.db.apply_change(analysis_change);
        Ok(())
    }
}

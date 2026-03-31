use std::io::{BufReader, BufWriter};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use descendit_ra::{RaSession, SemanticData};
use notify::{RecursiveMode, Watcher, recommended_watcher};

use crate::server_protocol::{Request, Response, read_message, write_message};

/// Guard that removes the socket file on drop.
struct SocketCleanup {
    path: PathBuf,
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

enum WatchEvent {
    FileChanged,
    Connection(UnixStream),
}

/// Start a file-watching server on the given Unix socket path.
///
/// Watches `watch_paths` for filesystem changes and re-analyzes on each change.
/// Also accepts connections on the Unix socket for on-demand analysis and reap.
pub(crate) fn run_watch(socket_path: &Path, watch_paths: &[PathBuf]) -> anyhow::Result<()> {
    // Derive manifest_dir: walk up from first watch_path to find Cargo.toml.
    let first_path = watch_paths
        .first()
        .ok_or_else(|| anyhow::anyhow!("at least one watch path is required"))?;
    let canonical_first = std::fs::canonicalize(first_path)?;
    let start = if canonical_first.is_file() {
        canonical_first
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(canonical_first)
    } else {
        canonical_first
    };
    let manifest_dir = find_manifest_dir(&start)?;

    let (mut session, listener, _cleanup) = init_server(socket_path, &manifest_dir)?;

    let data = session.reload_and_analyze()?;
    print_summary(&data);

    let (tx, rx) = mpsc::channel::<WatchEvent>();
    let _watcher = start_file_watcher(watch_paths, tx.clone())?;
    start_accept_thread(listener, tx);

    run_event_loop(rx, &mut session)
}

fn init_server(
    socket_path: &Path,
    manifest_dir: &Path,
) -> anyhow::Result<(RaSession, UnixListener, SocketCleanup)> {
    if socket_path.exists() {
        match UnixStream::connect(socket_path) {
            Ok(_) => anyhow::bail!(
                "another server is already listening on {}",
                socket_path.display()
            ),
            Err(_) => {
                let _ = std::fs::remove_file(socket_path);
            }
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    let cleanup = SocketCleanup {
        path: socket_path.to_owned(),
    };

    eprintln!("[watch] loading workspace at {}...", manifest_dir.display());
    let session = RaSession::load(manifest_dir)?;
    eprintln!("[watch] workspace loaded");

    Ok((session, listener, cleanup))
}

fn start_file_watcher(
    watch_paths: &[PathBuf],
    tx: mpsc::Sender<WatchEvent>,
) -> anyhow::Result<impl Watcher> {
    let mut watcher = recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(WatchEvent::FileChanged);
        }
    })?;
    for path in watch_paths {
        let canonical = std::fs::canonicalize(path)?;
        watcher.watch(&canonical, RecursiveMode::Recursive)?;
    }
    Ok(watcher)
}

fn start_accept_thread(listener: UnixListener, tx: mpsc::Sender<WatchEvent>) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let _ = tx.send(WatchEvent::Connection(s));
                }
                Err(e) => eprintln!("[watch] accept error: {e}"),
            }
        }
    });
}

fn run_event_loop(rx: mpsc::Receiver<WatchEvent>, session: &mut RaSession) -> anyhow::Result<()> {
    let debounce = Duration::from_millis(300);
    loop {
        match rx.recv()? {
            WatchEvent::FileChanged => {
                let deadline = Instant::now() + debounce;
                while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                    match rx.recv_timeout(remaining) {
                        Ok(WatchEvent::FileChanged) => continue,
                        Ok(WatchEvent::Connection(stream)) => {
                            handle_connection(stream, session);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                }
                eprintln!("[watch] files changed, re-analyzing...");
                match session.reload_and_analyze() {
                    Ok(data) => print_summary(&data),
                    Err(e) => eprintln!("[watch] analysis error: {e:#}"),
                }
            }
            WatchEvent::Connection(stream) => match handle_connection(stream, session) {
                ConnectionResult::Continue => {}
                ConnectionResult::Reap => return Ok(()),
                ConnectionResult::Error(e) => eprintln!("[watch] connection error: {e}"),
            },
        }
    }
}

fn find_manifest_dir(start: &Path) -> anyhow::Result<PathBuf> {
    let mut dir = start;
    for _ in 0..32 {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return Ok(dir.to_path_buf());
        }
        dir = match dir.parent() {
            Some(parent) => parent,
            None => break,
        };
    }
    anyhow::bail!(
        "could not find Cargo.toml walking up from {}",
        start.display()
    )
}

enum ConnectionResult {
    Continue,
    Reap,
    Error(anyhow::Error),
}

fn handle_connection(stream: UnixStream, session: &mut RaSession) -> ConnectionResult {
    let mut reader = BufReader::new(&stream);
    let mut writer = BufWriter::new(&stream);

    let request: Request = match read_message(&mut reader) {
        Ok(req) => req,
        Err(e) => return ConnectionResult::Error(e),
    };

    match request {
        Request::Analyze { manifest_dir } => {
            let response = handle_analyze(session, &manifest_dir);
            if let Err(e) = write_message(&mut writer, &response) {
                return ConnectionResult::Error(e.into());
            }
            ConnectionResult::Continue
        }
        Request::Reap => {
            let _ = write_message(&mut writer, &Response::Ok);
            ConnectionResult::Reap
        }
    }
}

fn handle_analyze(session: &mut RaSession, manifest_dir: &Path) -> Response {
    let canonical = match std::fs::canonicalize(manifest_dir) {
        Ok(p) => p,
        Err(e) => {
            return Response::Error {
                message: format!("failed to canonicalize manifest dir: {e}"),
            };
        }
    };

    // Allow any manifest_dir that is the session's own dir or a descendant of
    // the workspace root. When the request targets a different subcrate within
    // the same workspace, use extract_for_subcrate instead of reload_and_analyze.
    if canonical != session.manifest_dir() && !canonical.starts_with(session.workspace_root()) {
        return Response::Error {
            message: format!(
                "requested {} is outside workspace root {}",
                canonical.display(),
                session.workspace_root().display(),
            ),
        };
    }

    let result = if canonical == session.manifest_dir() {
        session.reload_and_analyze()
    } else {
        session.extract_for_subcrate(&canonical)
    };

    match result {
        Ok(data) => Response::SemanticData(data),
        Err(e) => Response::Error {
            message: format!("analysis failed: {e:#}"),
        },
    }
}

fn print_summary(data: &SemanticData) {
    eprintln!(
        "[watch] {} type resolutions, {} function state resolutions, {} call edges",
        data.type_cardinalities.len(),
        data.function_cardinalities.len(),
        data.call_edges.len(),
    );
}

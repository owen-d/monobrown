use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixStream;
use std::path::Path;

use descendit_ra::SemanticData;

use crate::server_protocol::{Request, Response, read_message, write_message};

/// Connect to a running server and request analysis for the given manifest directory.
pub(crate) fn analyze(socket_path: &Path, manifest_dir: &Path) -> anyhow::Result<SemanticData> {
    let stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(300)))?;
    let mut reader = BufReader::new(&stream);
    let mut writer = BufWriter::new(&stream);

    write_message(
        &mut writer,
        &Request::Analyze {
            manifest_dir: manifest_dir.to_owned(),
        },
    )?;

    let response: Response = read_message(&mut reader)?;
    match response {
        Response::SemanticData(data) => Ok(data),
        Response::Error { message } => anyhow::bail!("server error: {message}"),
        Response::Ok => anyhow::bail!("unexpected Ok response to Analyze request"),
    }
}

/// Connect to a running server and request graceful shutdown.
pub(crate) fn reap(socket_path: &Path) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket_path)?;
    let mut reader = BufReader::new(&stream);
    let mut writer = BufWriter::new(&stream);

    write_message(&mut writer, &Request::Reap)?;

    let response: Response = read_message(&mut reader)?;
    match response {
        Response::Ok => Ok(()),
        Response::Error { message } => anyhow::bail!("server error: {message}"),
        Response::SemanticData(_) => anyhow::bail!("unexpected SemanticData response to Reap"),
    }
}

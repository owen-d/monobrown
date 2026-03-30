use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use descendit_ra::SemanticData;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) enum Request {
    Analyze { manifest_dir: PathBuf },
    Reap,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum Response {
    SemanticData(SemanticData),
    Ok,
    Error { message: String },
}

/// Write a JSON message followed by a newline.
pub(crate) fn write_message<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, msg)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// Read a single JSON line message.
pub(crate) fn read_message<R: BufRead, T: serde::de::DeserializeOwned>(
    reader: &mut R,
) -> anyhow::Result<T> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() {
        anyhow::bail!("unexpected end of stream");
    }
    Ok(serde_json::from_str(&line)?)
}

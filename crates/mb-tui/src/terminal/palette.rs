//! Optional terminal-owned OSC 11 palette detection.
//!
//! [`probe_background`] is only called by [`super::TuiSession`], after raw
//! mode is enabled and before the normal crossterm event loop starts. The
//! probe and keyboard input therefore have one reader and cannot race. Bytes
//! that arrive while the terminal replies are buffered as regular events.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::time::Duration;

use crossterm::event::Event;

use super::input::InputDemux;

const QUERY: &[u8] = b"\x1b]11;?\x1b\\";
const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
const PROBE_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

/// Result of the startup probe, including input typed while it was pending.
#[derive(Debug, Default)]
pub(crate) struct ProbeResult {
    pub(crate) rgb: Option<(u8, u8, u8)>,
    pub(crate) palette: Option<crate::theme::palette::Palette>,
    pub(crate) pending_events: Vec<Event>,
}

/// Detect the terminal background through OSC 11 on Unix.
///
/// This compatibility helper returns only the detected RGB value. Interactive
/// sessions should use [`super::TuiSession`], which also preserves startup
/// input through the internal demultiplexer.
pub fn detect_background() -> Option<(u8, u8, u8)> {
    probe_background().rgb
}

/// Parse an OSC 11 response into an RGB tuple.
pub fn parse_response(response: &[u8]) -> Option<(u8, u8, u8)> {
    let text = std::str::from_utf8(response).ok()?;
    let start = text.to_ascii_lowercase().find("rgb:")?;
    let part = text[start + 4..]
        .trim_end_matches('\\')
        .trim_end_matches('\x1b')
        .trim_end_matches('\x07');
    let mut components = part.split('/');
    Some((
        scale_hex_to_u8(components.next()?)?,
        scale_hex_to_u8(components.next()?)?,
        scale_hex_to_u8(components.next()?)?,
    ))
}

fn scale_hex_to_u8(hex: &str) -> Option<u8> {
    let value = u16::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        1 => Some((value as u8) | ((value as u8) << 4)),
        2 => Some(value as u8),
        3 => Some((value >> 4) as u8),
        4 => Some((value >> 8) as u8),
        _ => None,
    }
}

pub(crate) fn probe_background() -> ProbeResult {
    #[cfg(unix)]
    {
        return probe_background_unix();
    }
    #[cfg(not(unix))]
    {
        ProbeResult::default()
    }
}

#[cfg(unix)]
fn probe_background_unix() -> ProbeResult {
    let mut tty = match OpenOptions::new().read(true).write(true).open("/dev/tty") {
        Ok(tty) => tty,
        Err(_) => return ProbeResult::default(),
    };

    if tty.write_all(QUERY).and_then(|_| tty.flush()).is_err() {
        return ProbeResult::default();
    }

    let fd = tty.as_raw_fd();
    let mut demux = InputDemux::new();
    let mut buffer = [0_u8; 256];
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;

    while demux.response().is_none() {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll_fd` points to one valid tty descriptor and the timeout
        // is bounded. No memory is retained by libc after the call returns.
        let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if ready <= 0 || (poll_fd.revents & libc::POLLIN) == 0 {
            break;
        }
        match tty.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => demux.feed(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    // If the terminal began its OSC response just before the main deadline,
    // keep the probe reader for a short bounded grace period. Without this,
    // the response tail can be consumed by crossterm and appear as literal
    // keyboard input (notably a phantom `/2c2c/3434` search query).
    if demux.collecting_probe_response() {
        let deadline = std::time::Instant::now() + PROBE_DRAIN_TIMEOUT;
        while demux.response().is_none() {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            let mut poll_fd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `poll_fd` points to one valid tty descriptor and the
            // timeout is bounded. No memory is retained by libc after call.
            let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
            if ready <= 0 || (poll_fd.revents & libc::POLLIN) == 0 {
                break;
            }
            match tty.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => demux.feed(&buffer[..read]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    demux.finish();
    let rgb = demux.response().and_then(parse_response);
    let palette =
        rgb.map(|(red, green, blue)| crate::theme::palette::Palette::from_rgb(red, green, blue));

    ProbeResult {
        rgb,
        palette,
        pending_events: demux.into_events(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_four_digit_response() {
        assert_eq!(
            parse_response(b"\x1b]11;rgb:0000/2b2b/3636\x1b\\"),
            Some((0, 0x2b, 0x36))
        );
    }

    #[test]
    fn parses_two_digit_response() {
        assert_eq!(
            parse_response(b"\x1b]11;rgb:ff/ff/ff\x1b\\"),
            Some((255, 255, 255))
        );
    }
}

//! Optional terminal-owned OSC 11 palette detection.
//!
//! Termina owns the input stream while the probe is active. Protocol responses
//! are filtered from the same event queue as keyboard input, so keys arriving
//! during startup remain available to the normal event loop.

use std::time::Duration;

use termina::escape::{
    csi::{Csi, Device},
    osc::{ColorOrQuery, DynamicColorNumber, Osc},
};
use termina::event::Event as TerminaEvent;

#[cfg(unix)]
use super::input::InputReader;

const PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Result of the startup probe and the parser that owns the tty afterward.
#[derive(Debug, Default)]
pub(crate) struct ProbeResult {
    pub(crate) rgb: Option<(u8, u8, u8)>,
    pub(crate) palette: Option<crate::theme::palette::Palette>,
    #[cfg(unix)]
    pub(crate) input: Option<InputReader>,
}

/// Detect the terminal background through OSC 11 on Unix.
pub fn detect_background() -> Option<(u8, u8, u8)> {
    probe_background().rgb
}

/// Parse an OSC 11 response into an RGB tuple.
///
/// This remains available for callers that already have a raw response. New
/// terminal code should prefer Termina's typed `ColorOrQuery::Color` value.
pub fn parse_response(response: &[u8]) -> Option<(u8, u8, u8)> {
    let response = response.strip_suffix(&[0x9c]).unwrap_or(response);
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
    let mut input = match InputReader::open() {
        Ok(input) => input,
        Err(_) => return ProbeResult::default(),
    };

    if input.write_all(&super::input::probe_query()).is_err() {
        return ProbeResult {
            input: Some(input),
            ..ProbeResult::default()
        };
    }

    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let mut rgb = None;
    let mut device_attributes = false;

    while !(device_attributes && rgb.is_some()) {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() || !input.poll_probe_event(remaining).unwrap_or(false) {
            break;
        }

        let Ok(event) = input.read_probe_event() else {
            break;
        };
        match event {
            TerminaEvent::Osc(Osc::ChangeDynamicColors(
                DynamicColorNumber::TextBackgroundColor,
                colors,
            )) => {
                if let Some(ColorOrQuery::Color(color)) = colors.first() {
                    rgb = Some((color.red, color.green, color.blue));
                }
            }
            TerminaEvent::Csi(Csi::Device(Device::DeviceAttributes(()))) => {
                device_attributes = true;
            }
            _ => {}
        }
    }

    let palette =
        rgb.map(|(red, green, blue)| crate::theme::palette::Palette::from_rgb(red, green, blue));
    ProbeResult {
        rgb,
        palette,
        input: Some(input),
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

    #[test]
    fn parses_c1_string_terminator_response() {
        assert_eq!(
            parse_response(b"\x1b]11;rgb:2c2c/3434/3c3c\x9c"),
            Some((0x2c, 0x34, 0x3c))
        );
    }
}

//! Terminal palette detection.
//!
//! Queries the terminal background color at startup using the OSC 11
//! escape sequence and caches the result. The detected color is used
//! to determine whether the terminal has a light or dark background.

use std::sync::OnceLock;

/// Cached result of the background color query.
/// `Some((r, g, b))` on success, `None` if the query failed or timed out.
static DETECTED_BG: OnceLock<Option<(u8, u8, u8)>> = OnceLock::new();

/// Returns the detected background RGB, or `None` if detection failed.
pub fn default_bg() -> Option<(u8, u8, u8)> {
    *DETECTED_BG.get_or_init(query_background_color)
}

/// Returns whether the terminal background is light.
///
/// Light is defined as perceived luminance > 128 using the standard
/// formula `Y = 0.299*R + 0.587*G + 0.114*B`.
///
/// If the query fails, assumes dark (most terminals are dark).
pub fn is_light() -> bool {
    match default_bg() {
        Some((r, g, b)) => luminance(r, g, b) > 128.0,
        None => false,
    }
}

/// Perceived luminance using the ITU-R BT.601 luma coefficients.
fn luminance(r: u8, g: u8, b: u8) -> f64 {
    0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
}

/// Query the terminal background color via the OSC 11 escape sequence.
///
/// Sends `ESC ] 11 ; ? ESC \` and parses the response, which looks like:
/// `ESC ] 11 ; rgb:RRRR/GGGG/BBBB ESC \`
///
/// Returns `None` if the terminal does not respond or the response
/// cannot be parsed.
fn query_background_color() -> Option<(u8, u8, u8)> {
    #[cfg(not(unix))]
    {
        None
    }
    #[cfg(unix)]
    {
        query_background_color_unix()
    }
}

/// Unix implementation of the OSC 11 background color query.
///
/// Opens `/dev/tty` directly so that this works even when stdout is
/// redirected. Temporarily enables raw mode on the tty fd so that the
/// terminal's response can be read without waiting for a newline.
#[cfg(unix)]
fn query_background_color_unix() -> Option<(u8, u8, u8)> {
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;

    // OSC 11 query: request current background color.
    const QUERY: &[u8] = b"\x1b]11;?\x1b\\";

    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;

    let fd = tty.as_raw_fd();

    // SAFETY: `fd` is a valid file descriptor from `OpenOptions::open` and
    // `termios` is zero-initialized. `tcgetattr` only reads terminal state.
    let original = unsafe {
        let mut termios = std::mem::zeroed::<libc::termios>();
        if libc::tcgetattr(fd, &mut termios) != 0 {
            return None;
        }
        termios
    };

    let mut raw = original;
    // SAFETY: `raw` is a valid copy of the original termios. We set VMIN=0
    // and VTIME=2 so reads return immediately when data arrives or after
    // 200ms, avoiding a busy-spin loop.
    unsafe {
        libc::cfmakeraw(&mut raw);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 2; // 200ms timeout (in deciseconds)
        if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
            return None;
        }
    }

    // Restore original termios on all exit paths.
    struct RestoreGuard {
        fd: std::os::unix::io::RawFd,
        termios: libc::termios,
    }
    impl Drop for RestoreGuard {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.termios);
            }
        }
    }
    let _guard = RestoreGuard {
        fd,
        termios: original,
    };

    tty.write_all(QUERY).ok()?;
    tty.flush().ok()?;

    // Read the response byte-by-byte until we see the string terminator
    // (`ESC \`) or the VTIME timeout expires (read returns 0 bytes).
    let mut buf = [0u8; 1];
    let mut response = Vec::with_capacity(64);

    loop {
        match tty.read(&mut buf) {
            Ok(1) => {
                response.push(buf[0]);
                // Check for ST (String Terminator): ESC \ (0x1b 0x5c) or BEL (0x07).
                let len = response.len();
                if buf[0] == 0x07 {
                    break;
                }
                if len >= 2 && response[len - 2] == 0x1b && response[len - 1] == b'\\' {
                    break;
                }
                // Safety valve: no valid response is this long.
                if len > 128 {
                    return None;
                }
            }
            Ok(_) => return None, // VTIME expired — no response from terminal.
            Err(_) => return None,
        }
    }

    parse_osc11_response(&response)
}

/// Parse an OSC 11 response into an RGB tuple.
///
/// The response format is:
///   `ESC ] 11 ; rgb:RRRR/GGGG/BBBB ST`
///
/// where each color component is 1-4 hex digits. We scale each
/// component down to 8-bit by taking the high byte of a 16-bit value.
fn parse_osc11_response(response: &[u8]) -> Option<(u8, u8, u8)> {
    let text = std::str::from_utf8(response).ok()?;

    // Find the "rgb:" prefix (case-insensitive).
    let rgb_start = text.to_ascii_lowercase().find("rgb:")?;
    let rgb_part = &text[rgb_start + 4..];

    // Strip any trailing ST (ESC \ or BEL).
    let rgb_part = rgb_part
        .trim_end_matches('\\')
        .trim_end_matches('\x1b')
        .trim_end_matches('\x07');

    let mut components = rgb_part.split('/');
    let r_hex = components.next()?;
    let g_hex = components.next()?;
    let b_hex = components.next()?;

    // Each component is 1-4 hex digits representing a value in [0, 0xFFFF].
    // Scale to 8-bit: if the component has N digits, the high 8 bits are
    // obtained by shifting right by 4*(N-1) bits when N <= 2, or by
    // taking the first two hex digits when N > 2.
    let r = scale_hex_to_u8(r_hex)?;
    let g = scale_hex_to_u8(g_hex)?;
    let b = scale_hex_to_u8(b_hex)?;

    Some((r, g, b))
}

/// Convert a 1-4 digit hex string to an 8-bit value.
///
/// The hex string represents a value in a space proportional to its
/// digit count:
///   - 1 digit: 0-F (4-bit), scale to 8-bit by repeating (0xA -> 0xAA)
///   - 2 digits: 0-FF (8-bit), use directly
///   - 3 digits: 0-FFF (12-bit), take high 8 bits
///   - 4 digits: 0-FFFF (16-bit), take high 8 bits
fn scale_hex_to_u8(hex: &str) -> Option<u8> {
    let val = u16::from_str_radix(hex, 16).ok()?;
    let byte = match hex.len() {
        1 => (val as u8) | ((val as u8) << 4),
        2 => val as u8,
        3 => (val >> 4) as u8,
        4 => (val >> 8) as u8,
        _ => return None,
    };
    Some(byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_black() {
        assert_eq!(luminance(0, 0, 0), 0.0);
    }

    #[test]
    fn luminance_white() {
        let y = luminance(255, 255, 255);
        // 0.299*255 + 0.587*255 + 0.114*255 = 255.0
        assert!((y - 255.0).abs() < 0.01);
    }

    #[test]
    fn luminance_dark_bg() {
        // Typical dark terminal: (30, 30, 30)
        let y = luminance(30, 30, 30);
        assert!(y < 128.0);
    }

    #[test]
    fn luminance_light_bg() {
        // Typical light terminal: (240, 240, 240)
        let y = luminance(240, 240, 240);
        assert!(y > 128.0);
    }

    #[test]
    fn luminance_solarized_dark() {
        // Solarized Dark base03: #002b36 -> (0, 43, 54)
        let y = luminance(0, 43, 54);
        assert!(y < 128.0);
    }

    #[test]
    fn luminance_solarized_light() {
        // Solarized Light base3: #fdf6e3 -> (253, 246, 227)
        let y = luminance(253, 246, 227);
        assert!(y > 128.0);
    }

    #[test]
    fn parse_osc11_4digit_hex() -> Result<(), Box<dyn std::error::Error>> {
        // Typical response: ESC]11;rgb:0000/2b2b/3636 ESC\
        let response = b"\x1b]11;rgb:0000/2b2b/3636\x1b\\";
        let (r, g, b) = parse_osc11_response(response).ok_or("parse failed")?;
        assert_eq!(r, 0x00);
        assert_eq!(g, 0x2b);
        assert_eq!(b, 0x36);
        Ok(())
    }

    #[test]
    fn parse_osc11_2digit_hex() -> Result<(), Box<dyn std::error::Error>> {
        // Some terminals respond with 2-digit hex.
        let response = b"\x1b]11;rgb:ff/ff/ff\x1b\\";
        let (r, g, b) = parse_osc11_response(response).ok_or("parse failed")?;
        assert_eq!(r, 0xff);
        assert_eq!(g, 0xff);
        assert_eq!(b, 0xff);
        Ok(())
    }

    #[test]
    fn parse_osc11_bel_terminator() -> Result<(), Box<dyn std::error::Error>> {
        // BEL (0x07) as string terminator.
        let response = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
        let (r, g, b) = parse_osc11_response(response).ok_or("parse failed")?;
        assert_eq!(r, 0xff);
        assert_eq!(g, 0xff);
        assert_eq!(b, 0xff);
        Ok(())
    }

    #[test]
    fn parse_osc11_1digit_hex() -> Result<(), Box<dyn std::error::Error>> {
        // 1-digit components: 0xA -> 0xAA
        let response = b"\x1b]11;rgb:a/b/c\x1b\\";
        let (r, g, b) = parse_osc11_response(response).ok_or("parse failed")?;
        assert_eq!(r, 0xaa);
        assert_eq!(g, 0xbb);
        assert_eq!(b, 0xcc);
        Ok(())
    }

    #[test]
    fn parse_osc11_3digit_hex() -> Result<(), Box<dyn std::error::Error>> {
        // 3-digit components: 0xFED -> high 8 bits = 0xFE
        let response = b"\x1b]11;rgb:fed/cba/987\x1b\\";
        let (r, g, b) = parse_osc11_response(response).ok_or("parse failed")?;
        assert_eq!(r, 0xfe); // 0xfed >> 4
        assert_eq!(g, 0xcb); // 0xcba >> 4
        assert_eq!(b, 0x98); // 0x987 >> 4
        Ok(())
    }

    #[test]
    fn parse_osc11_garbage_returns_none() {
        let response = b"not a valid response";
        assert!(parse_osc11_response(response).is_none());
    }

    #[test]
    fn scale_hex_to_u8_cases() {
        assert_eq!(scale_hex_to_u8("f"), Some(0xff));
        assert_eq!(scale_hex_to_u8("0"), Some(0x00));
        assert_eq!(scale_hex_to_u8("ab"), Some(0xab));
        assert_eq!(scale_hex_to_u8("abc"), Some(0xab));
        assert_eq!(scale_hex_to_u8("abcd"), Some(0xab));
        assert_eq!(scale_hex_to_u8(""), None);
        assert_eq!(scale_hex_to_u8("abcde"), None);
        assert_eq!(scale_hex_to_u8("zz"), None);
    }
}

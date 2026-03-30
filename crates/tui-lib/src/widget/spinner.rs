use std::time::Duration;

/// Returns the braille-dot spinner character for the given elapsed duration.
///
/// Uses a 10-frame braille cycle at 100ms per frame (1s total rotation).
/// Deterministic: the same elapsed duration always produces the same frame.
pub fn spinner_frame(elapsed: Duration) -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    const FRAME_MS: u128 = 100;
    let idx = (elapsed.as_millis() / FRAME_MS) as usize % FRAMES.len();
    FRAMES[idx]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::spinner_frame;

    #[test]
    fn spinner_starts_on_first_frame() {
        assert_eq!(spinner_frame(Duration::ZERO), '⠋');
    }

    #[test]
    fn spinner_advances_on_frame_boundary() {
        assert_eq!(spinner_frame(Duration::from_millis(99)), '⠋');
        assert_eq!(spinner_frame(Duration::from_millis(100)), '⠙');
    }

    #[test]
    fn spinner_wraps_after_full_cycle() {
        assert_eq!(spinner_frame(Duration::from_millis(1000)), '⠋');
    }
}

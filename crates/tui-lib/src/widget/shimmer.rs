use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static HAS_TRUE_COLOR: OnceLock<bool> = OnceLock::new();

const SHIMMER_PADDING: usize = 10;
const SHIMMER_BAND_HALF_WIDTH: f32 = 5.0;
const SHIMMER_SWEEP_SECONDS: f32 = 2.0;
const SHIMMER_BASE_COLOR: (u8, u8, u8) = (100, 100, 100);
const SHIMMER_HIGHLIGHT_COLOR: (u8, u8, u8) = (0, 255, 255);

fn elapsed_since_start() -> Duration {
    let start = PROCESS_START.get_or_init(Instant::now);
    start.elapsed()
}

fn has_true_color() -> bool {
    *HAS_TRUE_COLOR.get_or_init(|| {
        std::env::var("COLORTERM")
            .map(|v| v == "truecolor" || v == "24bit")
            .unwrap_or(false)
    })
}

fn blend(fg: (u8, u8, u8), bg: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let mix = |a: u8, b: u8| -> u8 {
        let v = (a as f32) * t + (b as f32) * (1.0 - t);
        (v.round() as u8).clamp(0, 255)
    };
    (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}

fn shimmer_position(text_len: usize, elapsed: Duration) -> usize {
    let period = text_len + SHIMMER_PADDING * 2;
    let pos_f =
        (elapsed.as_secs_f32() % SHIMMER_SWEEP_SECONDS) / SHIMMER_SWEEP_SECONDS * (period as f32);
    pos_f as usize
}

fn shimmer_intensity(index: usize, position: usize) -> f32 {
    let i_pos = index as isize + SHIMMER_PADDING as isize;
    let pos = position as isize;
    let dist = (i_pos - pos).abs() as f32;
    if dist > SHIMMER_BAND_HALF_WIDTH {
        return 0.0;
    }
    let x = std::f32::consts::PI * (dist / SHIMMER_BAND_HALF_WIDTH);
    0.5 * (1.0 + x.cos())
}

fn style_for_intensity(intensity: f32, has_true_color: bool) -> Style {
    if has_true_color {
        let highlight = intensity.clamp(0.0, 1.0);
        let (r, g, b) = blend(SHIMMER_HIGHLIGHT_COLOR, SHIMMER_BASE_COLOR, highlight * 0.9);
        return Style::default()
            .fg(Color::Rgb(r, g, b))
            .add_modifier(Modifier::BOLD);
    }
    color_for_level(intensity)
}

pub fn shimmer_spans_with(
    text: &str,
    elapsed: Duration,
    has_true_color: bool,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }

    let position = shimmer_position(chars.len(), elapsed);
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(chars.len());
    for (index, ch) in chars.iter().enumerate() {
        let intensity = shimmer_intensity(index, position);
        let style = style_for_intensity(intensity, has_true_color);
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans
}

pub fn shimmer_spans(text: &str) -> Vec<Span<'static>> {
    shimmer_spans_with(text, elapsed_since_start(), has_true_color())
}

fn color_for_level(intensity: f32) -> Style {
    if intensity < 0.2 {
        Style::default().add_modifier(Modifier::DIM)
    } else if intensity < 0.6 {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::style::{Color, Modifier};

    use super::{shimmer_intensity, shimmer_position, shimmer_spans_with};

    #[test]
    fn shimmer_position_wraps_after_full_cycle() {
        let text_len = 5;
        assert_eq!(shimmer_position(text_len, Duration::ZERO), 0);
        assert_eq!(shimmer_position(text_len, Duration::from_secs(2)), 0);
    }

    #[test]
    fn shimmer_intensity_peaks_at_band_center() {
        let position = 10;
        assert_eq!(shimmer_intensity(0, position), 1.0);
        assert_eq!(shimmer_intensity(10, position), 0.0);
    }

    #[test]
    fn shimmer_spans_without_truecolor_use_modifier_fallback() {
        let spans = shimmer_spans_with("abc", Duration::ZERO, false);
        assert_eq!(spans.len(), 3);
        assert!(spans[0].style.add_modifier.contains(Modifier::DIM));
        assert!(spans[1].style.add_modifier.contains(Modifier::DIM));
        assert!(spans[2].style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn shimmer_spans_with_truecolor_use_rgb_gradient() {
        let spans = shimmer_spans_with("a", Duration::ZERO, true);
        match spans[0].style.fg {
            Some(Color::Rgb(_, _, _)) => {}
            other => panic!("expected rgb color, got {other:?}"),
        }
    }

    #[test]
    fn shimmer_spans_empty_text_is_empty() {
        assert!(shimmer_spans_with("", Duration::ZERO, true).is_empty());
    }
}

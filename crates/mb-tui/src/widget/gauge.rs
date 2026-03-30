use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::display_width;
use crate::theme;

/// Left end cap of the track.
const LEFT_CAP: &str = "\u{2576}"; // ╶
/// Track dash (empty/unfilled).
const TRACK_DASH: &str = "\u{2500}"; // ─
/// Filled track (left of needle).
const FILLED_TRACK: &str = "\u{2501}"; // ━
/// Needle indicator.
const NEEDLE: &str = "\u{25CF}"; // ●
/// Right end cap of the track.
const RIGHT_CAP: &str = "\u{2574}"; // ╴

/// Minimum track width: left cap + 3 dashes + right cap.
const MIN_TRACK_WIDTH: u16 = 5;

/// How the needle color is determined.
enum ColorMode {
    Fixed(Color),
    Gradient,
}

/// A compact text-based gauge showing a value within a range.
///
/// Renders a horizontal track with a needle indicator whose position
/// reflects the current value (0.0 to 1.0). Suitable for bounded scalar
/// values like confidence, alignment, or temperature.
///
/// ```text
/// label: ╶───────●───────╴
///        low              high
/// ```
pub struct Gauge {
    value: f64,
    label: Option<String>,
    low_label: Option<String>,
    high_label: Option<String>,
    color_fn: ColorMode,
}

impl Gauge {
    /// Create a new gauge with the given value (clamped to 0.0..=1.0).
    /// Defaults to gradient color mode.
    pub fn new(value: f64) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            label: None,
            low_label: None,
            high_label: None,
            color_fn: ColorMode::Gradient,
        }
    }

    pub fn label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    pub fn low_label(mut self, label: String) -> Self {
        self.low_label = Some(label);
        self
    }

    pub fn high_label(mut self, label: String) -> Self {
        self.high_label = Some(label);
        self
    }

    /// Use a fixed color for the needle.
    pub fn color(mut self, color: Color) -> Self {
        self.color_fn = ColorMode::Fixed(color);
        self
    }

    /// Use gradient color mode (default): green at low, yellow at mid, red at high.
    pub fn gradient(mut self) -> Self {
        self.color_fn = ColorMode::Gradient;
        self
    }

    /// Update the value (clamped to 0.0..=1.0).
    pub fn set_value(&mut self, value: f64) {
        self.value = value.clamp(0.0, 1.0);
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    /// Resolve the needle color based on the current color mode and value.
    fn needle_color(&self) -> Color {
        match &self.color_fn {
            ColorMode::Fixed(c) => *c,
            ColorMode::Gradient => {
                if self.value <= 0.33 {
                    theme::success()
                } else if self.value <= 0.66 {
                    theme::warning()
                } else {
                    theme::error()
                }
            }
        }
    }

    /// Whether end labels are configured.
    fn has_end_labels(&self) -> bool {
        self.low_label.is_some() || self.high_label.is_some()
    }
}

impl LayoutRenderable for Gauge {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.fill_width();
        let natural_height: u16 = if self.has_end_labels() { 2 } else { 1 };
        let height = match constraints.max_height {
            Some(max_h) => natural_height.min(max_h),
            None => natural_height,
        };
        Size::new(width, height)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut track_x = area.x;
        let mut track_width = area.width;

        // 1. Render optional label at the left of row 0.
        if let Some(ref label_text) = self.label {
            let label_w = display_width(label_text) as u16;
            // label + ": " suffix = label_w + 2
            let label_total = label_w + 2;
            if label_total < area.width {
                let label_style = Style::default().fg(theme::dim());
                let formatted = format!("{label_text}: ");
                buf.set_stringn(
                    area.x,
                    area.y,
                    &formatted,
                    label_total as usize,
                    label_style,
                );
                track_x = area.x + label_total;
                track_width = area.width - label_total;
            }
        }

        // 2. Check minimum track width.
        if track_width < MIN_TRACK_WIDTH {
            return;
        }

        // 3. Compute needle position within the track.
        //    Track layout: [left_cap] [dashes...needle...dashes] [right_cap]
        //    The inner positions (between caps) span (track_width - 2) cells.
        //    Needle can occupy any of those inner positions.
        let inner_width = track_width - 2; // cells between the two caps
        let needle_offset = if inner_width <= 1 {
            0
        } else {
            (self.value * (inner_width - 1) as f64).round() as u16
        };
        // Absolute needle position = track_x + 1 (past left cap) + needle_offset
        let needle_abs = track_x + 1 + needle_offset;

        // 4. Render the track on row 0.
        let dim_style = Style::default().fg(theme::dim());
        let faint_style = Style::default().fg(theme::border());
        let needle_color = self.needle_color();
        let needle_style = Style::default()
            .fg(needle_color)
            .add_modifier(Modifier::BOLD);
        let filled_style = Style::default().fg(needle_color);

        let track_end = track_x + track_width;
        for x in track_x..track_end {
            if x == track_x {
                // Left cap.
                buf.set_stringn(x, area.y, LEFT_CAP, 1, dim_style);
            } else if x == track_end - 1 {
                // Right cap.
                buf.set_stringn(x, area.y, RIGHT_CAP, 1, faint_style);
            } else if x == needle_abs {
                // Needle.
                buf.set_stringn(x, area.y, NEEDLE, 1, needle_style);
            } else if x < needle_abs {
                // Filled track left of needle.
                buf.set_stringn(x, area.y, FILLED_TRACK, 1, filled_style);
            } else {
                // Empty track right of needle.
                buf.set_stringn(x, area.y, TRACK_DASH, 1, faint_style);
            }
        }

        // 5. Render end labels on row 1 if height permits.
        if area.height >= 2 && self.has_end_labels() {
            let label_y = area.y + 1;

            if let Some(ref low) = self.low_label {
                let low_w = display_width(low) as u16;
                let available = track_width.min(low_w);
                buf.set_stringn(track_x, label_y, low, available as usize, dim_style);
            }

            if let Some(ref high) = self.high_label {
                let high_w = display_width(high) as u16;
                // Right-align to the track end.
                let start = track_end.saturating_sub(high_w);
                let start = start.max(track_x);
                let available = (track_end - start) as usize;
                buf.set_stringn(start, label_y, high, available, dim_style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    use super::*;

    /// Extract the symbol string for a row from the buffer.
    fn row_symbols(buf: &Buffer, area: Rect, row: u16) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf[(x, area.y + row)].symbol().to_string())
            .collect()
    }

    #[test]
    fn height_one_without_labels() {
        let gauge = Gauge::new(0.5);
        let size = gauge.measure(Constraints::loose(30, 10));
        assert_eq!(size.height, 1);
    }

    #[test]
    fn height_two_with_labels() {
        let gauge = Gauge::new(0.5)
            .low_label("cold".to_string())
            .high_label("hot".to_string());
        let size = gauge.measure(Constraints::loose(30, 10));
        assert_eq!(size.height, 2);
    }

    #[test]
    fn needle_at_zero() {
        let gauge = Gauge::new(0.0);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        let text = row_symbols(&buf, area, 0);
        // Needle should be at position 1 (just after left cap).
        assert_eq!(buf[(0, 0)].symbol(), LEFT_CAP);
        assert_eq!(buf[(1, 0)].symbol(), NEEDLE);
        assert!(text.contains(NEEDLE));
    }

    #[test]
    fn needle_at_one() {
        let gauge = Gauge::new(1.0);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        // Needle should be at the last inner position (just before right cap).
        assert_eq!(buf[(19, 0)].symbol(), RIGHT_CAP);
        assert_eq!(buf[(18, 0)].symbol(), NEEDLE);
    }

    #[test]
    fn needle_at_half() {
        let gauge = Gauge::new(0.5);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        // Inner width = 18, needle_offset = round(0.5 * 17) = round(8.5) = 9.
        // Needle at track_x + 1 + 9 = 10.
        let text = row_symbols(&buf, area, 0);
        assert!(text.contains(NEEDLE));
        assert_eq!(buf[(10, 0)].symbol(), NEEDLE);
    }

    #[test]
    fn gradient_colors() {
        // Value 0.0 -> success color.
        let gauge_low = Gauge::new(0.0);
        assert_eq!(gauge_low.needle_color(), theme::success());

        // Value 0.5 -> warning color.
        let gauge_mid = Gauge::new(0.5);
        assert_eq!(gauge_mid.needle_color(), theme::warning());

        // Value 1.0 -> error color.
        let gauge_high = Gauge::new(1.0);
        assert_eq!(gauge_high.needle_color(), theme::error());
    }

    #[test]
    fn fixed_color() -> Result<(), Box<dyn std::error::Error>> {
        let gauge = Gauge::new(0.5).color(Color::Magenta);
        assert_eq!(gauge.needle_color(), Color::Magenta);

        // Verify the needle cell gets the fixed color.
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        // Find the needle and check its fg color.
        let needle_x = (0..20)
            .find(|&x| buf[(x, 0)].symbol() == NEEDLE)
            .ok_or("needle not found")?;
        assert_eq!(buf[(needle_x, 0)].fg, Color::Magenta);
        Ok(())
    }

    #[test]
    fn label_rendered() {
        let gauge = Gauge::new(0.5).label("temp".to_string());
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        let text = row_symbols(&buf, area, 0);
        assert!(
            text.starts_with("temp: "),
            "expected label at left, got: {text:?}"
        );
    }

    #[test]
    fn end_labels_rendered() {
        let gauge = Gauge::new(0.5)
            .low_label("cold".to_string())
            .high_label("hot".to_string());
        let area = Rect::new(0, 0, 20, 2);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        let row1 = row_symbols(&buf, area, 1);
        assert!(
            row1.starts_with("cold"),
            "expected low label at left of row 1, got: {row1:?}"
        );
        assert!(
            row1.ends_with("hot"),
            "expected high label at right of row 1, got: {row1:?}"
        );
    }

    #[test]
    fn value_clamped() {
        let gauge = Gauge::new(1.5);
        assert_eq!(gauge.value(), 1.0);

        let gauge = Gauge::new(-0.5);
        assert_eq!(gauge.value(), 0.0);

        let mut gauge = Gauge::new(0.5);
        gauge.set_value(2.0);
        assert_eq!(gauge.value(), 1.0);
        gauge.set_value(-1.0);
        assert_eq!(gauge.value(), 0.0);
    }

    #[test]
    fn track_chars_present() {
        let gauge = Gauge::new(0.5);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        gauge.render(area, &mut buf);

        let text = row_symbols(&buf, area, 0);
        assert!(text.contains(LEFT_CAP), "missing left cap in: {text:?}");
        assert!(text.contains(RIGHT_CAP), "missing right cap in: {text:?}");
        assert!(text.contains(NEEDLE), "missing needle in: {text:?}");
        assert!(text.contains(TRACK_DASH), "missing track dash in: {text:?}");
    }
}

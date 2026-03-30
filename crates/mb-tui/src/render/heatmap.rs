//! Styling utility that maps intensity values to colors.
//!
//! A [`HeatmapRamp`] defines a color ramp — a sorted sequence of (position, RGB)
//! stops — and linearly interpolates between them to produce smooth gradients.
//! This is not a widget; it produces [`Style`] values for other renderables to
//! consume.

use ratatui::style::{Color, Style};

/// A color ramp that maps intensity values in [0.0, 1.0] to colors.
///
/// Stops are `(position, (r, g, b))` pairs sorted by position. Intensities
/// between stops are linearly interpolated per channel. Values outside the
/// stop range clamp to the nearest endpoint.
pub struct HeatmapRamp {
    stops: Vec<(f64, (u8, u8, u8))>,
}

impl HeatmapRamp {
    /// Cool-to-hot: blue, cyan, green, yellow, red.
    pub fn cool_to_hot() -> Self {
        Self {
            stops: vec![
                (0.0, (0, 0, 200)),
                (0.25, (0, 200, 200)),
                (0.5, (0, 200, 0)),
                (0.75, (200, 200, 0)),
                (1.0, (200, 0, 0)),
            ],
        }
    }

    /// Dim-to-bright: dark gray, light gray, white.
    pub fn dim_to_bright() -> Self {
        Self {
            stops: vec![
                (0.0, (80, 80, 80)),
                (0.5, (180, 180, 180)),
                (1.0, (255, 255, 255)),
            ],
        }
    }

    /// Green-to-red: success, warning, error.
    pub fn green_to_red() -> Self {
        Self {
            stops: vec![
                (0.0, (0, 180, 60)),
                (0.5, (200, 180, 0)),
                (1.0, (200, 40, 40)),
            ],
        }
    }

    /// Custom ramp from stops. Stops are (position, RGB) pairs.
    /// Position values should be in [0.0, 1.0]. Sorted internally.
    pub fn custom(mut stops: Vec<(f64, (u8, u8, u8))>) -> Self {
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        Self { stops }
    }

    /// Map an intensity (0.0 to 1.0) to an RGB color.
    /// Values outside [0.0, 1.0] are clamped.
    pub fn color(&self, intensity: f64) -> Color {
        let (r, g, b) = self.interpolate(intensity);
        Color::Rgb(r, g, b)
    }

    /// Map intensity to a [`Style`] with the foreground set.
    pub fn style(&self, intensity: f64) -> Style {
        Style::default().fg(self.color(intensity))
    }

    /// Linearly interpolate between stops to produce an RGB triple.
    fn interpolate(&self, intensity: f64) -> (u8, u8, u8) {
        // Empty ramp: default to white.
        if self.stops.is_empty() {
            return (255, 255, 255);
        }

        // Single stop: always return it.
        if self.stops.len() == 1 {
            return self.stops[0].1;
        }

        let clamped = intensity.clamp(0.0, 1.0);

        // At or before the first stop.
        if clamped <= self.stops[0].0 {
            return self.stops[0].1;
        }

        // At or past the last stop.
        let last = self.stops.len() - 1;
        if clamped >= self.stops[last].0 {
            return self.stops[last].1;
        }

        // Find the surrounding pair of stops.
        for i in 0..last {
            let (p0, rgb0) = self.stops[i];
            let (p1, rgb1) = self.stops[i + 1];

            if clamped >= p0 && clamped <= p1 {
                let t = if (p1 - p0).abs() < f64::EPSILON {
                    0.0
                } else {
                    (clamped - p0) / (p1 - p0)
                };
                return lerp_rgb(rgb0, rgb1, t);
            }
        }

        // Should be unreachable given the bounds checks above.
        self.stops[last].1
    }
}

/// Linearly interpolate between two RGB colors.
fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let r = a.0 as f64 + (b.0 as f64 - a.0 as f64) * t;
    let g = a.1 as f64 + (b.1 as f64 - a.1 as f64) * t;
    let blue = a.2 as f64 + (b.2 as f64 - a.2 as f64) * t;
    (r.round() as u8, g.round() as u8, blue.round() as u8)
}

/// Quick heatmap style using the cool-to-hot ramp.
pub fn heatmap_style(intensity: f64) -> Style {
    HeatmapRamp::cool_to_hot().style(intensity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cool_to_hot_endpoints() {
        let ramp = HeatmapRamp::cool_to_hot();
        // 0.0 should be blue-ish: high blue, zero red.
        assert_eq!(ramp.color(0.0), Color::Rgb(0, 0, 200));
        // 1.0 should be red-ish: high red, zero blue.
        assert_eq!(ramp.color(1.0), Color::Rgb(200, 0, 0));
    }

    #[test]
    fn dim_to_bright_endpoints() {
        let ramp = HeatmapRamp::dim_to_bright();
        // 0.0 should be dark.
        assert_eq!(ramp.color(0.0), Color::Rgb(80, 80, 80));
        // 1.0 should be white.
        assert_eq!(ramp.color(1.0), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn green_to_red_midpoint() {
        let ramp = HeatmapRamp::green_to_red();
        // 0.5 should be the yellow stop.
        assert_eq!(ramp.color(0.5), Color::Rgb(200, 180, 0));
    }

    #[test]
    fn interpolation_midpoint() {
        // Two stops: black at 0.0, white at 1.0. Midpoint should be gray.
        let ramp = HeatmapRamp::custom(vec![(0.0, (0, 0, 0)), (1.0, (200, 100, 50))]);
        assert_eq!(ramp.color(0.5), Color::Rgb(100, 50, 25));
    }

    #[test]
    fn clamps_below_zero() {
        let ramp = HeatmapRamp::cool_to_hot();
        // Negative intensity should clamp to the first stop.
        assert_eq!(ramp.color(-0.5), Color::Rgb(0, 0, 200));
    }

    #[test]
    fn clamps_above_one() {
        let ramp = HeatmapRamp::cool_to_hot();
        // Intensity > 1.0 should clamp to the last stop.
        assert_eq!(ramp.color(1.5), Color::Rgb(200, 0, 0));
    }

    #[test]
    fn single_stop_ramp() {
        let ramp = HeatmapRamp::custom(vec![(0.5, (42, 100, 200))]);
        // Any intensity should return the single stop color.
        assert_eq!(ramp.color(0.0), Color::Rgb(42, 100, 200));
        assert_eq!(ramp.color(0.5), Color::Rgb(42, 100, 200));
        assert_eq!(ramp.color(1.0), Color::Rgb(42, 100, 200));
    }

    #[test]
    fn custom_ramp() {
        // Three stops: interpolation between them should work correctly.
        let ramp = HeatmapRamp::custom(vec![
            (0.0, (0, 0, 0)),
            (0.5, (100, 100, 100)),
            (1.0, (200, 200, 200)),
        ]);
        // At 0.25: halfway between first two stops.
        assert_eq!(ramp.color(0.25), Color::Rgb(50, 50, 50));
        // At 0.75: halfway between last two stops.
        assert_eq!(ramp.color(0.75), Color::Rgb(150, 150, 150));
    }

    #[test]
    fn style_sets_foreground() {
        let ramp = HeatmapRamp::cool_to_hot();
        let style = ramp.style(0.0);
        assert_eq!(style.fg, Some(Color::Rgb(0, 0, 200)));
    }

    #[test]
    fn empty_ramp_returns_white() {
        let ramp = HeatmapRamp::custom(vec![]);
        // Edge case: no stops should return a sensible default (white).
        assert_eq!(ramp.color(0.0), Color::Rgb(255, 255, 255));
        assert_eq!(ramp.color(0.5), Color::Rgb(255, 255, 255));
        assert_eq!(ramp.color(1.0), Color::Rgb(255, 255, 255));
    }
}

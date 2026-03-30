use ratatui::style::Color;

/// Map a ratatui `Color` to its RGB components.
/// Returns `None` for `Color::Reset`, which has no fixed RGB value.
pub fn color_to_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Reset => None,
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((205, 0, 0)),
        Color::Green => Some((0, 205, 0)),
        Color::Yellow => Some((205, 205, 0)),
        Color::Blue => Some((0, 0, 238)),
        Color::Magenta => Some((205, 0, 205)),
        Color::Cyan => Some((0, 205, 205)),
        Color::Gray => Some((229, 229, 229)),
        Color::DarkGray => Some((127, 127, 127)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((92, 92, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(n) => Some(XTERM_256[n as usize]),
    }
}

/// WCAG 2.0 relative luminance of an sRGB color.
///
/// For each channel, linearize by removing the sRGB gamma curve, then
/// combine with the standard luminance coefficients.
pub fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    let linearize = |channel: u8| -> f64 {
        let s = channel as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)
}

/// WCAG contrast ratio between two RGB colors.
///
/// Returns a value in [1.0, 21.0]. Higher means more contrast.
pub fn contrast_ratio(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> f64 {
    let lum_fg = relative_luminance(fg.0, fg.1, fg.2);
    let lum_bg = relative_luminance(bg.0, bg.1, bg.2);
    let l1 = lum_fg.max(lum_bg);
    let l2 = lum_fg.min(lum_bg);
    (l1 + 0.05) / (l2 + 0.05)
}

/// Euclidean distance between two RGB colors, normalized to [0.0, 1.0].
///
/// 0.0 means identical colors, 1.0 means maximally distant (black vs white).
pub fn rgb_distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let dr = a.0 as f64 - b.0 as f64;
    let dg = a.1 as f64 - b.1 as f64;
    let db = a.2 as f64 - b.2 as f64;
    ((dr * dr + dg * dg + db * db) / (3.0 * 255.0 * 255.0)).sqrt()
}

/// Convert HSL color to RGB.
///
/// - `h`: hue in degrees [0, 360)
/// - `s`: saturation [0.0, 1.0]
/// - `l`: lightness [0.0, 1.0]
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

/// Standard xterm-256 color palette.
///
/// - 0-7: standard ANSI colors
/// - 8-15: bright ANSI colors
/// - 16-231: 6x6x6 color cube (index = r*36 + g*6 + b + 16, components map
///   through CUBE_STEPS)
/// - 232-255: grayscale ramp (value = 8 + 10*n for n in 0..24)
pub const XTERM_256: [(u8, u8, u8); 256] = {
    // Cube component values for indices 0..6.
    const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    let mut table = [(0u8, 0u8, 0u8); 256];

    // 0-7: standard ANSI.
    table[0] = (0, 0, 0);
    table[1] = (205, 0, 0);
    table[2] = (0, 205, 0);
    table[3] = (205, 205, 0);
    table[4] = (0, 0, 238);
    table[5] = (205, 0, 205);
    table[6] = (0, 205, 205);
    table[7] = (229, 229, 229);

    // 8-15: bright ANSI.
    table[8] = (127, 127, 127);
    table[9] = (255, 0, 0);
    table[10] = (0, 255, 0);
    table[11] = (255, 255, 0);
    table[12] = (92, 92, 255);
    table[13] = (255, 0, 255);
    table[14] = (0, 255, 255);
    table[15] = (255, 255, 255);

    // 16-231: 6x6x6 color cube.
    let mut i = 16;
    let mut r = 0usize;
    while r < 6 {
        let mut g = 0usize;
        while g < 6 {
            let mut b = 0usize;
            while b < 6 {
                table[i] = (CUBE_STEPS[r], CUBE_STEPS[g], CUBE_STEPS[b]);
                i += 1;
                b += 1;
            }
            g += 1;
        }
        r += 1;
    }

    // 232-255: grayscale ramp.
    let mut n = 0u8;
    while (n as usize) < 24 {
        let v = 8 + 10 * n;
        table[232 + n as usize] = (v, v, v);
        n += 1;
    }

    table
};

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn luminance_black() {
        let lum = relative_luminance(0, 0, 0);
        assert!((lum - 0.0).abs() < 1e-10, "expected 0.0, got {lum}");
    }

    #[test]
    fn luminance_white() {
        let lum = relative_luminance(255, 255, 255);
        assert!((lum - 1.0).abs() < 1e-6, "expected 1.0, got {lum}");
    }

    #[test]
    fn contrast_black_on_white() {
        let ratio = contrast_ratio((0, 0, 0), (255, 255, 255));
        assert!((ratio - 21.0).abs() < 0.01, "expected 21.0, got {ratio}");
    }

    #[test]
    fn contrast_same_color() {
        let ratio = contrast_ratio((128, 64, 32), (128, 64, 32));
        assert!((ratio - 1.0).abs() < 1e-10, "expected 1.0, got {ratio}");
    }

    #[test]
    fn color_to_rgb_named() {
        assert_eq!(color_to_rgb(Color::Black), Some((0, 0, 0)));
        assert_eq!(color_to_rgb(Color::Red), Some((205, 0, 0)));
        assert_eq!(color_to_rgb(Color::White), Some((255, 255, 255)));
        assert_eq!(color_to_rgb(Color::Gray), Some((229, 229, 229)));
    }

    #[test]
    fn color_to_rgb_indexed() {
        // Index 0 = standard black.
        assert_eq!(color_to_rgb(Color::Indexed(0)), Some((0, 0, 0)));
        // Index 15 = bright white.
        assert_eq!(color_to_rgb(Color::Indexed(15)), Some((255, 255, 255)));
        // Index 232 = first grayscale step.
        assert_eq!(color_to_rgb(Color::Indexed(232)), Some((8, 8, 8)));
        // Index 16 = first cube color (0,0,0).
        assert_eq!(color_to_rgb(Color::Indexed(16)), Some((0, 0, 0)));
    }

    #[test]
    fn color_to_rgb_reset_is_none() {
        assert_eq!(color_to_rgb(Color::Reset), None);
    }

    #[test]
    fn rgb_distance_same() {
        let d = rgb_distance((100, 100, 100), (100, 100, 100));
        assert!((d - 0.0).abs() < 1e-10, "expected 0.0, got {d}");
    }

    #[test]
    fn rgb_distance_black_white() {
        let d = rgb_distance((0, 0, 0), (255, 255, 255));
        assert!((d - 1.0).abs() < 1e-10, "expected 1.0, got {d}");
    }

    #[test]
    fn hsl_to_rgb_pure_red() {
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), (255, 0, 0));
    }

    #[test]
    fn hsl_to_rgb_pure_green() {
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
    }

    #[test]
    fn hsl_to_rgb_pure_blue() {
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
    }

    #[test]
    fn hsl_to_rgb_white() {
        assert_eq!(hsl_to_rgb(0.0, 0.0, 1.0), (255, 255, 255));
    }

    #[test]
    fn hsl_to_rgb_black() {
        assert_eq!(hsl_to_rgb(0.0, 0.0, 0.0), (0, 0, 0));
    }
}

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::color::{color_to_rgb, contrast_ratio, rgb_distance};

/// A single rendered frame captured from a TUI widget at a specific state.
pub struct Frame {
    pub label: String,
    buffer: Buffer,
    width: u16,
    height: u16,
}

/// A sequence of frames recording a widget's visual output over time.
///
/// Provides metrics for animation quality (smoothness, coverage, periodicity)
/// and assertions for automated verification by AI agents.
pub struct FrameTape {
    frames: Vec<Frame>,
}

/// A cell whose foreground color changed between two frames.
struct CellChange {
    x: u16,
    y: u16,
    old_fg: (u8, u8, u8),
    new_fg: (u8, u8, u8),
}

/// A cell that fails a minimum contrast ratio threshold.
pub struct ContrastViolation {
    pub x: u16,
    pub y: u16,
    pub fg_rgb: (u8, u8, u8),
    pub bg_rgb: (u8, u8, u8),
    pub ratio: f64,
    pub symbol: String,
}

/// Color-level difference between two consecutive frames.
pub struct FrameDelta {
    pub from_label: String,
    pub to_label: String,
    pub cells_changed: usize,
    pub total_distance: f64,
    pub max_distance: f64,
}

// ---------------------------------------------------------------------------
// Frame: per-frame metrics
// ---------------------------------------------------------------------------

impl Frame {
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Find cells failing the given contrast ratio threshold.
    ///
    /// `terminal_bg` is the assumed background for cells with `Color::Reset` bg.
    /// `terminal_fg` is the assumed foreground for cells with `Color::Reset` fg.
    /// Whitespace-only symbols are skipped because they carry no visible text.
    pub fn contrast_violations(
        &self,
        terminal_bg: (u8, u8, u8),
        terminal_fg: (u8, u8, u8),
        min_ratio: f64,
    ) -> Vec<ContrastViolation> {
        let mut violations = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = &self.buffer[(x, y)];

                // Skip whitespace-only symbols -- no visible glyph to contrast.
                if cell.symbol().trim().is_empty() {
                    continue;
                }

                let fg_rgb = resolve_fg(cell.fg, terminal_fg);
                let bg_rgb = resolve_bg(cell.bg, terminal_bg);
                let ratio = contrast_ratio(fg_rgb, bg_rgb);

                if ratio < min_ratio {
                    violations.push(ContrastViolation {
                        x,
                        y,
                        fg_rgb,
                        bg_rgb,
                        ratio,
                        symbol: cell.symbol().to_string(),
                    });
                }
            }
        }
        violations
    }

    /// All distinct foreground RGB colors used in non-whitespace cells.
    ///
    /// Returns a sorted `Vec` for deterministic output.
    pub fn fg_palette(&self) -> Vec<(u8, u8, u8)> {
        let mut set = HashSet::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = &self.buffer[(x, y)];
                if cell.symbol().trim().is_empty() {
                    continue;
                }
                // Resolve Reset to white (default terminal fg).
                let rgb = resolve_fg(cell.fg, (255, 255, 255));
                set.insert(rgb);
            }
        }
        let mut colors: Vec<(u8, u8, u8)> = set.into_iter().collect();
        colors.sort();
        colors
    }
}

/// Resolve a foreground color, falling back to `default` for `Color::Reset`.
fn resolve_fg(color: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    color_to_rgb(color).unwrap_or(default)
}

/// Resolve a background color, falling back to `default` for `Color::Reset`.
fn resolve_bg(color: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    color_to_rgb(color).unwrap_or(default)
}

// ---------------------------------------------------------------------------
// FrameTape: recording
// ---------------------------------------------------------------------------

impl FrameTape {
    /// Record a tape from an iterator of labeled states.
    ///
    /// Each state is rendered into an off-screen buffer via `render_fn`,
    /// producing one `Frame` per state.
    pub fn record_states<S>(
        states: impl IntoIterator<Item = (String, S)>,
        render_fn: fn(&S, Rect, &mut Buffer),
        width: u16,
        height: u16,
    ) -> Self {
        let area = Rect::new(0, 0, width, height);
        let frames = states
            .into_iter()
            .map(|(label, state)| {
                let mut buffer = Buffer::empty(area);
                render_fn(&state, area, &mut buffer);
                Frame {
                    label,
                    buffer,
                    width,
                    height,
                }
            })
            .collect();
        Self { frames }
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

// ---------------------------------------------------------------------------
// FrameTape: cross-frame metrics
// ---------------------------------------------------------------------------

impl FrameTape {
    /// Cell-level color diffs between consecutive frames.
    ///
    /// For each pair of consecutive frames, computes the number of cells whose
    /// foreground color changed, the total RGB distance across all changed cells,
    /// and the maximum single-cell distance.
    pub fn frame_deltas(&self) -> Vec<FrameDelta> {
        let default_fg = (255, 255, 255);
        self.frames
            .windows(2)
            .map(|pair| {
                let a = &pair[0];
                let b = &pair[1];
                let mut cells_changed = 0usize;
                let mut total_distance = 0.0f64;
                let mut max_distance = 0.0f64;

                for y in 0..a.height {
                    for x in 0..a.width {
                        let fg_a = resolve_fg(a.buffer[(x, y)].fg, default_fg);
                        let fg_b = resolve_fg(b.buffer[(x, y)].fg, default_fg);
                        if fg_a != fg_b {
                            let dist = rgb_distance(fg_a, fg_b);
                            cells_changed += 1;
                            total_distance += dist;
                            if dist > max_distance {
                                max_distance = dist;
                            }
                        }
                    }
                }

                FrameDelta {
                    from_label: a.label.clone(),
                    to_label: b.label.clone(),
                    cells_changed,
                    total_distance,
                    max_distance,
                }
            })
            .collect()
    }

    /// Coefficient of variation of frame delta magnitudes.
    ///
    /// Lower values indicate smoother animation (more uniform frame-to-frame
    /// change). Returns 0.0 if fewer than 2 non-zero deltas exist.
    pub fn smoothness(&self) -> f64 {
        let deltas = self.frame_deltas();
        let nonzero: Vec<f64> = deltas
            .iter()
            .map(|d| d.total_distance)
            .filter(|&m| m > 0.0)
            .collect();
        if nonzero.len() < 2 {
            return 0.0;
        }
        let mean = nonzero.iter().sum::<f64>() / nonzero.len() as f64;
        let variance =
            nonzero.iter().map(|&m| (m - mean).powi(2)).sum::<f64>() / nonzero.len() as f64;
        variance.sqrt() / mean
    }

    /// Fraction of cells whose foreground color changed at least once.
    ///
    /// Returns 0.0 if there are fewer than 2 frames.
    pub fn coverage(&self) -> f64 {
        if self.frames.len() < 2 {
            return 0.0;
        }
        let width = self.frames[0].width as usize;
        let height = self.frames[0].height as usize;
        let total_cells = width * height;
        if total_cells == 0 {
            return 0.0;
        }

        let default_fg = (255, 255, 255);
        let mut changed = vec![false; total_cells];

        for pair in self.frames.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            for y in 0..a.height {
                for x in 0..a.width {
                    let idx = y as usize * width + x as usize;
                    if !changed[idx] {
                        let fg_a = resolve_fg(a.buffer[(x, y)].fg, default_fg);
                        let fg_b = resolve_fg(b.buffer[(x, y)].fg, default_fg);
                        if fg_a != fg_b {
                            changed[idx] = true;
                        }
                    }
                }
            }
        }

        let count_changed = changed.iter().filter(|&&c| c).count();
        count_changed as f64 / total_cells as f64
    }

    /// Distance between first and last frame's foreground colors.
    ///
    /// 0.0 means identical (perfect loop), 1.0 means maximally different.
    /// Returns 0.0 if fewer than 2 frames.
    pub fn periodicity(&self) -> f64 {
        if self.frames.len() < 2 {
            return 0.0;
        }
        let first = &self.frames[0];
        let last = &self.frames[self.frames.len() - 1];
        let default_fg = (255, 255, 255);
        let total_cells = first.width as usize * first.height as usize;
        if total_cells == 0 {
            return 0.0;
        }

        let mut sum = 0.0f64;
        for y in 0..first.height {
            for x in 0..first.width {
                let fg_first = resolve_fg(first.buffer[(x, y)].fg, default_fg);
                let fg_last = resolve_fg(last.buffer[(x, y)].fg, default_fg);
                sum += rgb_distance(fg_first, fg_last);
            }
        }
        sum / total_cells as f64
    }
}

// ---------------------------------------------------------------------------
// FrameTape: assertions
// ---------------------------------------------------------------------------

impl FrameTape {
    /// Panics if any frame has cells below the given contrast ratio.
    pub fn assert_contrast(&self, terminal_bg: (u8, u8, u8), min_ratio: f64) {
        let terminal_fg = (255, 255, 255);
        for frame in &self.frames {
            let violations = frame.contrast_violations(terminal_bg, terminal_fg, min_ratio);
            assert!(
                violations.is_empty(),
                "Contrast violations in frame '{}': {} cells below {:.1}:1 ratio. \
                 First violation: ({},{}) symbol='{}' fg={:?} bg={:?} ratio={:.2}",
                frame.label,
                violations.len(),
                min_ratio,
                violations[0].x,
                violations[0].y,
                violations[0].symbol,
                violations[0].fg_rgb,
                violations[0].bg_rgb,
                violations[0].ratio,
            );
        }
    }

    /// WCAG AA for normal text (4.5:1).
    pub fn assert_contrast_aa(&self, terminal_bg: (u8, u8, u8)) {
        self.assert_contrast(terminal_bg, 4.5);
    }

    /// WCAG AA for large text (3.0:1).
    pub fn assert_contrast_aa_large(&self, terminal_bg: (u8, u8, u8)) {
        self.assert_contrast(terminal_bg, 3.0);
    }

    /// Panics if smoothness (coefficient of variation) exceeds `max_cv`.
    pub fn assert_smooth(&self, max_cv: f64) {
        let cv = self.smoothness();
        assert!(
            cv <= max_cv,
            "Animation not smooth enough: CV={:.3} exceeds threshold {:.3}. \
             Frame deltas: {:?}",
            cv,
            max_cv,
            self.frame_deltas()
                .iter()
                .map(|d| format!("{}: dist={:.3}", d.from_label, d.total_distance))
                .collect::<Vec<_>>(),
        );
    }

    /// Panics if coverage falls below `min_fraction`.
    pub fn assert_coverage(&self, min_fraction: f64) {
        let cov = self.coverage();
        assert!(
            cov >= min_fraction,
            "Animation coverage too low: {:.1}% < {:.1}% required",
            cov * 100.0,
            min_fraction * 100.0,
        );
    }

    /// Panics if periodicity exceeds `max_distance`.
    pub fn assert_periodic(&self, max_distance: f64) {
        let dist = self.periodicity();
        assert!(
            dist <= max_distance,
            "Animation not periodic: distance={dist:.4} exceeds threshold {max_distance:.4}",
        );
    }
}

// ---------------------------------------------------------------------------
// FrameTape: introspection
// ---------------------------------------------------------------------------

impl FrameTape {
    /// Render all frames stacked with labels, using styled text format.
    pub fn to_filmstrip(&self) -> String {
        let mut out = String::new();
        for frame in &self.frames {
            out.push_str(&format!("--- {} ---\n", frame.label));
            out.push_str(&super::text::buffer_to_styled_text(&frame.buffer));
            out.push('\n');
        }
        out
    }

    /// Show cells that changed between consecutive frames with RGB hex values.
    ///
    /// Limited to 20 changed cells per frame pair to avoid flooding output.
    pub fn to_diff_strip(&self) -> String {
        let default_fg = (255, 255, 255);
        let max_cells_per_pair = 20;
        let mut out = String::new();

        for pair in self.frames.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            let mut changed_cells: Vec<CellChange> = Vec::new();

            for y in 0..a.height {
                for x in 0..a.width {
                    let fg_a = resolve_fg(a.buffer[(x, y)].fg, default_fg);
                    let fg_b = resolve_fg(b.buffer[(x, y)].fg, default_fg);
                    if fg_a != fg_b {
                        changed_cells.push(CellChange {
                            x,
                            y,
                            old_fg: fg_a,
                            new_fg: fg_b,
                        });
                    }
                }
            }

            if changed_cells.is_empty() {
                continue;
            }

            out.push_str(&format!(
                "--- {} -> {} ({} cells changed) ---\n",
                a.label,
                b.label,
                changed_cells.len(),
            ));

            for c in changed_cells.iter().take(max_cells_per_pair) {
                let old = c.old_fg;
                let new = c.new_fg;
                out.push_str(&format!(
                    "  [{},{}] fg:#{:02x}{:02x}{:02x} -> #{:02x}{:02x}{:02x}\n",
                    c.x, c.y, old.0, old.1, old.2, new.0, new.1, new.2,
                ));
            }

            let remaining = changed_cells.len().saturating_sub(max_cells_per_pair);
            if remaining > 0 {
                out.push_str(&format!("  ... and {remaining} more\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};

    use super::*;

    /// Helper: render colored text into a buffer.
    fn render_colored(state: &(Color, &str), area: Rect, buf: &mut Buffer) {
        buf.set_string(area.x, area.y, state.1, Style::default().fg(state.0));
    }

    #[test]
    fn record_states_captures_frames() {
        let states = vec![
            ("a".to_string(), (Color::Red, "X")),
            ("b".to_string(), (Color::Blue, "X")),
        ];
        let tape = FrameTape::record_states(states, render_colored, 5, 1);
        assert_eq!(tape.len(), 2);
        assert!(!tape.is_empty());
        assert_eq!(tape.frames()[0].label, "a");
        assert_eq!(tape.frames()[1].label, "b");
    }

    #[test]
    fn contrast_violations_detects_low_contrast() {
        // Red (205,0,0) on dark red (100,0,0) is low contrast.
        let states = vec![("low".to_string(), (Color::Rgb(50, 50, 50), "X"))];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        let frame = &tape.frames()[0];
        let violations = frame.contrast_violations((40, 40, 40), (255, 255, 255), 4.5);
        assert!(!violations.is_empty(), "expected a contrast violation");
        assert!(violations[0].ratio < 4.5);
    }

    #[test]
    fn contrast_violations_passes_high_contrast() {
        // White text on black background is 21:1.
        let states = vec![("high".to_string(), (Color::White, "X"))];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        let frame = &tape.frames()[0];
        let violations = frame.contrast_violations((0, 0, 0), (255, 255, 255), 4.5);
        assert!(violations.is_empty(), "expected no violations");
    }

    #[test]
    fn smoothness_constant_delta_is_zero() {
        // Three frames with identical per-step change should have CV near 0.
        let states = vec![
            ("a".to_string(), (Color::Rgb(0, 0, 0), "X")),
            ("b".to_string(), (Color::Rgb(50, 50, 50), "X")),
            ("c".to_string(), (Color::Rgb(100, 100, 100), "X")),
        ];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        let cv = tape.smoothness();
        assert!(
            cv < 0.01,
            "expected near-zero CV for uniform steps, got {cv}"
        );
    }

    #[test]
    fn coverage_single_frame_is_zero() {
        let states = vec![("only".to_string(), (Color::Red, "X"))];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        assert!((tape.coverage() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn coverage_all_cells_change() {
        let states = vec![
            ("a".to_string(), (Color::Red, "X")),
            ("b".to_string(), (Color::Blue, "X")),
        ];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        // Only 1 cell out of 1 changed, so coverage = 1.0.
        // But cells beyond the text are whitespace and won't register as changed
        // because they both have Reset fg.
        assert!(tape.coverage() > 0.0);
    }

    #[test]
    fn periodicity_identical_endpoints() {
        let states = vec![
            ("start".to_string(), (Color::Rgb(100, 100, 100), "X")),
            ("mid".to_string(), (Color::Rgb(200, 200, 200), "X")),
            ("end".to_string(), (Color::Rgb(100, 100, 100), "X")),
        ];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        let dist = tape.periodicity();
        assert!(
            dist < 0.01,
            "expected near-zero periodicity distance, got {dist}"
        );
    }

    #[test]
    fn fg_palette_collects_unique_colors() {
        let states = vec![("f".to_string(), (Color::Rgb(10, 20, 30), "AB"))];
        let tape = FrameTape::record_states(states, render_colored, 5, 1);
        let palette = tape.frames()[0].fg_palette();
        assert!(palette.contains(&(10, 20, 30)));
    }

    #[test]
    fn to_filmstrip_includes_labels() {
        let states = vec![("frame-0".to_string(), (Color::Red, "X"))];
        let tape = FrameTape::record_states(states, render_colored, 3, 1);
        let filmstrip = tape.to_filmstrip();
        assert!(filmstrip.contains("--- frame-0 ---"));
    }

    #[test]
    fn to_diff_strip_shows_changes() {
        let states = vec![
            ("a".to_string(), (Color::Rgb(0, 0, 0), "X")),
            ("b".to_string(), (Color::Rgb(255, 0, 0), "X")),
        ];
        let tape = FrameTape::record_states(states, render_colored, 1, 1);
        let diff = tape.to_diff_strip();
        assert!(diff.contains("a -> b"));
        assert!(diff.contains("#000000"));
        assert!(diff.contains("#ff0000"));
    }
}

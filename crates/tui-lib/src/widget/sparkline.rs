use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::{display_width, heatmap_style};
use crate::theme;

const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A compact single-row time-series sparkline using Unicode block elements.
///
/// Each column maps one data point to a block character whose height is
/// proportional to the value relative to the series maximum.
#[derive(Clone)]
pub struct Sparkline {
    data: Vec<f64>,
    max: Option<f64>,
    label: Option<String>,
    color: Option<Color>,
}

impl Sparkline {
    pub fn new(data: Vec<f64>) -> Self {
        Self {
            data,
            max: None,
            label: None,
            color: None,
        }
    }

    pub fn max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }

    pub fn label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn data(&self) -> &[f64] {
        &self.data
    }

    pub fn set_data(&mut self, data: Vec<f64>) {
        self.data = data;
    }

    pub fn push(&mut self, value: f64) {
        self.data.push(value);
    }

    /// Resolve the effective maximum for normalization.
    fn effective_max(&self) -> f64 {
        if let Some(m) = self.max {
            return m;
        }
        let auto = self.data.iter().copied().fold(0.0_f64, f64::max);
        if auto == 0.0 { 1.0 } else { auto }
    }
}

impl LayoutRenderable for Sparkline {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.fill_width();
        Size::new(width, 1)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut spark_x = area.x;
        let mut spark_width = area.width;

        // Render optional label at the left.
        if let Some(ref label) = self.label {
            let label_width = display_width(label) as u16;
            let label_with_space = label_width.saturating_add(1);
            if label_with_space < area.width {
                buf.set_stringn(
                    area.x,
                    area.y,
                    label,
                    label_width as usize,
                    Style::default().fg(theme::dim()),
                );
                spark_x = area.x + label_with_space;
                spark_width = area.width - label_with_space;
            }
        }

        if self.data.is_empty() || spark_width == 0 {
            return;
        }

        let max = self.effective_max();
        let cols = spark_width as usize;

        // If more data than columns, show the last N points.
        let visible = if self.data.len() > cols {
            &self.data[self.data.len() - cols..]
        } else {
            &self.data
        };

        // Left-pad if fewer points than columns.
        let pad = cols - visible.len();
        let fixed_style = self.color.map(|c| Style::default().fg(c));

        for (i, &value) in visible.iter().enumerate() {
            let x = spark_x + (pad + i) as u16;
            if value == 0.0 {
                // Zero renders as a space (blank).
                continue;
            }
            let normalized = value / max;
            let idx = (normalized * 7.0).round() as usize;
            let idx = idx.min(7);
            let ch = BLOCKS[idx];
            let style = fixed_style.unwrap_or_else(|| heatmap_style(normalized));
            buf[(x, area.y)]
                .set_symbol(&ch.to_string())
                .set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    #[test]
    fn height_always_one() {
        let spark = Sparkline::new(vec![1.0, 2.0, 3.0]);
        assert_eq!(spark.measure(Constraints::loose(20, 10)).height, 1);
        assert_eq!(spark.measure(Constraints::tight(5, 5)).height, 1);
        assert_eq!(spark.measure(Constraints::tight_width(100)).height, 1);
    }

    #[test]
    fn empty_data_renders_blank() {
        let spark = Sparkline::new(vec![]);
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), " ");
        }
    }

    #[test]
    fn full_bar_at_max() {
        let spark = Sparkline::new(vec![10.0]);
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        // Data point is right-aligned; last cell should be full block.
        assert_eq!(buf[(4, 0)].symbol(), "█");
        // Padding cells should be spaces.
        for x in 0..4 {
            assert_eq!(buf[(x, 0)].symbol(), " ");
        }
    }

    #[test]
    fn zero_renders_space() {
        let spark = Sparkline::new(vec![0.0, 10.0]);
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(buf[(1, 0)].symbol(), "█");
    }

    #[test]
    fn auto_max_from_data() {
        // With data [5.0, 10.0], auto-max is 10.0.
        // 5.0/10.0 = 0.5 -> round(0.5 * 7) = round(3.5) = 4 -> BLOCKS[4] = '▅'
        let spark = Sparkline::new(vec![5.0, 10.0]);
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "▅");
        assert_eq!(buf[(1, 0)].symbol(), "█");
    }

    #[test]
    fn explicit_max() {
        // With explicit max=20, value 10.0 -> 10/20 = 0.5 -> BLOCKS[4] = '▅'
        let spark = Sparkline::new(vec![10.0]).max(20.0);
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "▅");
    }

    #[test]
    fn truncates_to_width() {
        // 6 data points in 3 columns -> last 3 shown.
        let spark = Sparkline::new(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        // Max is 6.0. Last 3 values are [4.0, 5.0, 6.0].
        // 6.0/6.0 = 1.0 -> BLOCKS[7] = '█'
        assert_eq!(buf[(2, 0)].symbol(), "█");
        // All three cells should be non-space (all values > 0).
        for x in 0..3 {
            assert_ne!(buf[(x, 0)].symbol(), " ");
        }
    }

    #[test]
    fn label_rendered() {
        let spark = Sparkline::new(vec![10.0]).label("cpu".to_string());
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        // Label "cpu" occupies columns 0..3, space at 3, data in 4..10.
        assert_eq!(buf[(0, 0)].symbol(), "c");
        assert_eq!(buf[(1, 0)].symbol(), "p");
        assert_eq!(buf[(2, 0)].symbol(), "u");
        assert_eq!(buf[(3, 0)].symbol(), " ");
        // Data point is the last cell (right-aligned within sparkline area).
        assert_eq!(buf[(9, 0)].symbol(), "█");
    }

    #[test]
    fn push_appends() {
        let mut spark = Sparkline::new(vec![1.0, 2.0]);
        spark.push(3.0);
        assert_eq!(spark.data(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn blocks_proportional() {
        let spark = Sparkline::new(vec![0.25, 0.5, 0.75, 1.0]);
        let area = Rect::new(0, 0, 4, 1);
        let mut buf = Buffer::empty(area);
        spark.render(area, &mut buf);
        let symbols: Vec<&str> = (0..4).map(|x| buf[(x, 0)].symbol()).collect();
        // Each successive value should produce a taller (or equal) block.
        for i in 0..3 {
            assert!(
                symbols[i] <= symbols[i + 1],
                "expected non-decreasing block heights, got {symbols:?}"
            );
        }
        // Last value (1.0 = max) must be the full block.
        assert_eq!(symbols[3], "█");
        // First value (0.25) must not be the full block.
        assert_ne!(symbols[0], "█");
    }
}

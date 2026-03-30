use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::display_width;
use crate::theme;

/// Filled portion of the bar.
const FILL_CHAR: &str = "\u{2501}"; // ━
/// Forecast (predicted) portion of the bar.
const FORECAST_CHAR: &str = "\u{254C}"; // ╌
/// Empty portion of the bar.
const EMPTY_CHAR: &str = "\u{2500}"; // ─

/// Percentage suffix width: space + up to 3 digits + '%' = 5 chars.
const PERCENTAGE_WIDTH: u16 = 5;

/// A horizontal progress bar with optional forecast shading.
///
/// Renders as a single row: filled portion + optional forecast portion +
/// empty portion + percentage label.
///
/// ```text
/// [label ] [━━━━━━━╌╌╌─────────] [42%]
///           ^fill   ^forecast ^empty  ^percentage
/// ```
pub struct ProgressBar {
    progress: f64,
    forecast: Option<f64>,
    label: Option<String>,
    show_percentage: bool,
    fill_color: Option<Color>,
    forecast_color: Option<Color>,
    empty_color: Option<Color>,
}

impl ProgressBar {
    pub fn new(progress: f64) -> Self {
        Self {
            progress: progress.clamp(0.0, 1.0),
            forecast: None,
            label: None,
            show_percentage: true,
            fill_color: None,
            forecast_color: None,
            empty_color: None,
        }
    }

    pub fn forecast(mut self, forecast: f64) -> Self {
        self.forecast = Some(forecast.clamp(0.0, 1.0));
        self
    }

    pub fn label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    pub fn show_percentage(mut self, show: bool) -> Self {
        self.show_percentage = show;
        self
    }

    pub fn fill_color(mut self, color: Color) -> Self {
        self.fill_color = Some(color);
        self
    }

    pub fn forecast_color(mut self, color: Color) -> Self {
        self.forecast_color = Some(color);
        self
    }

    /// Effective forecast value: returns `None` if no forecast is set or if
    /// the forecast does not exceed current progress.
    fn effective_forecast(&self) -> Option<f64> {
        self.forecast.filter(|&f| f > self.progress)
    }
}

impl LayoutRenderable for ProgressBar {
    fn measure(&self, constraints: Constraints) -> Size {
        Size::new(constraints.fill_width(), 1)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut x = area.x;
        let right = area.x + area.width;

        // 1. Render label at left if set.
        if let Some(ref label_text) = self.label {
            let label_style = Style::default().fg(theme::dim());
            let label_w = display_width(label_text) as u16;
            let available = right.saturating_sub(x);
            if available == 0 {
                return;
            }
            buf.set_stringn(x, area.y, label_text, available as usize, label_style);
            // Advance past label + trailing space.
            x += label_w.min(available);
            if x < right {
                // Trailing space after label.
                x += 1;
            }
        }

        // 2. Reserve space for percentage at right if enabled.
        let pct_width = if self.show_percentage {
            PERCENTAGE_WIDTH
        } else {
            0
        };

        let bar_width = right.saturating_sub(x).saturating_sub(pct_width);

        // 3. Compute column counts.
        let filled_cols = (self.progress * bar_width as f64).round() as u16;
        let filled_cols = filled_cols.min(bar_width);

        let forecast_cols = match self.effective_forecast() {
            Some(f) => {
                let fc = ((f - self.progress) * bar_width as f64).round() as u16;
                fc.min(bar_width - filled_cols)
            }
            None => 0,
        };

        let empty_cols = bar_width - filled_cols - forecast_cols;

        // 4. Render bar sections.
        let fill_style = Style::default().fg(self.fill_color.unwrap_or_else(theme::focus));
        let forecast_style = Style::default().fg(self.forecast_color.unwrap_or_else(theme::dim));
        let empty_style = Style::default().fg(self.empty_color.unwrap_or_else(theme::border));

        for _ in 0..filled_cols {
            buf.set_stringn(x, area.y, FILL_CHAR, 1, fill_style);
            x += 1;
        }
        for _ in 0..forecast_cols {
            buf.set_stringn(x, area.y, FORECAST_CHAR, 1, forecast_style);
            x += 1;
        }
        for _ in 0..empty_cols {
            buf.set_stringn(x, area.y, EMPTY_CHAR, 1, empty_style);
            x += 1;
        }

        // 5. Render percentage at right.
        if self.show_percentage {
            let pct = (self.progress * 100.0).round() as u32;
            let pct_text = format!("{pct:>3}%");
            let mut pct_style = Style::default().fg(theme::dim());
            if self.progress > 0.0 {
                pct_style = pct_style.add_modifier(Modifier::BOLD);
            }
            // Leading space before percentage.
            x += 1;
            let remaining = right.saturating_sub(x) as usize;
            buf.set_stringn(x, area.y, &pct_text, remaining, pct_style);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    use super::*;

    /// Extract the symbol string for a single row from the buffer.
    fn buf_symbols(buf: &Buffer, area: Rect) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf[(x, area.y)].symbol().to_string())
            .collect()
    }

    /// Count occurrences of a character in the rendered row.
    fn count_char(buf: &Buffer, area: Rect, ch: &str) -> usize {
        (area.x..area.x + area.width)
            .filter(|&x| buf[(x, area.y)].symbol() == ch)
            .count()
    }

    #[test]
    fn height_always_one() {
        let bar = ProgressBar::new(0.5);
        assert_eq!(bar.measure(Constraints::loose(80, 10)).height, 1);
        assert_eq!(bar.measure(Constraints::tight(40, 5)).height, 1);
        assert_eq!(bar.measure(Constraints::unbounded()).height, 1);
    }

    #[test]
    fn zero_progress_all_empty() {
        let bar = ProgressBar::new(0.0).show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        assert_eq!(count_char(&buf, area, FILL_CHAR), 0);
        assert_eq!(count_char(&buf, area, FORECAST_CHAR), 0);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 20);
    }

    #[test]
    fn full_progress_all_filled() {
        let bar = ProgressBar::new(1.0).show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        assert_eq!(count_char(&buf, area, FILL_CHAR), 20);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 0);
    }

    #[test]
    fn half_progress() {
        let bar = ProgressBar::new(0.5).show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        assert_eq!(count_char(&buf, area, FILL_CHAR), 10);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 10);
    }

    #[test]
    fn percentage_label() {
        let bar = ProgressBar::new(0.42);
        // 25 wide: 20 bar + 5 percentage
        let area = Rect::new(0, 0, 25, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        let text = buf_symbols(&buf, area);
        assert!(
            text.ends_with(" 42%"),
            "expected ' 42%' suffix, got: {text:?}"
        );
    }

    #[test]
    fn no_percentage_when_disabled() {
        let bar = ProgressBar::new(0.42).show_percentage(false);
        let area = Rect::new(0, 0, 25, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        let text = buf_symbols(&buf, area);
        assert!(
            !text.contains('%'),
            "should not contain '%' when disabled, got: {text:?}"
        );
    }

    #[test]
    fn forecast_shading() -> Result<(), Box<dyn std::error::Error>> {
        // progress=0.25, forecast=0.75, bar_width=20 (no percentage).
        // filled=5, forecast=10, empty=5
        let bar = ProgressBar::new(0.25).forecast(0.75).show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        assert_eq!(count_char(&buf, area, FILL_CHAR), 5);
        assert_eq!(count_char(&buf, area, FORECAST_CHAR), 10);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 5);

        // Verify ordering: fill comes first, then forecast, then empty.
        let symbols = buf_symbols(&buf, area);
        let first_forecast = symbols
            .find(FORECAST_CHAR)
            .ok_or("forecast char not found")?;
        let last_fill = symbols.rfind(FILL_CHAR).ok_or("fill char not found")?;
        let first_empty = symbols.rfind(EMPTY_CHAR).ok_or("empty char not found")?;
        let last_forecast = symbols
            .rfind(FORECAST_CHAR)
            .ok_or("forecast char not found")?;
        assert!(last_fill < first_forecast, "fill should precede forecast");
        assert!(
            last_forecast < first_empty || count_char(&buf, area, EMPTY_CHAR) == 0,
            "forecast should precede empty"
        );
        Ok(())
    }

    #[test]
    fn label_rendered() {
        let bar = ProgressBar::new(0.5)
            .label("CPU".to_string())
            .show_percentage(false);
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        let text = buf_symbols(&buf, area);
        assert!(
            text.starts_with("CPU"),
            "expected label at left, got: {text:?}"
        );

        // Label should use dim color.
        let dim_color = crate::theme::dim();
        assert_eq!(buf[(0, 0)].fg, dim_color, "label should use dim color");
    }

    #[test]
    fn progress_clamped() {
        // Values > 1.0 clamped to 1.0.
        let bar = ProgressBar::new(1.5).show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);
        assert_eq!(count_char(&buf, area, FILL_CHAR), 20);

        // Values < 0.0 clamped to 0.0.
        let bar = ProgressBar::new(-0.5).show_percentage(false);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 20);
    }

    #[test]
    fn forecast_must_exceed_progress() {
        // forecast < progress should be treated as no forecast.
        let bar = ProgressBar::new(0.5).forecast(0.3).show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        assert_eq!(
            count_char(&buf, area, FORECAST_CHAR),
            0,
            "forecast < progress should produce no forecast chars"
        );
        assert_eq!(count_char(&buf, area, FILL_CHAR), 10);
        assert_eq!(count_char(&buf, area, EMPTY_CHAR), 10);
    }

    #[test]
    fn fill_color_applied() {
        let bar = ProgressBar::new(0.5)
            .fill_color(Color::Red)
            .show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        // First cell (filled) should have the custom fill color.
        assert_eq!(buf[(0, 0)].fg, Color::Red);
    }

    #[test]
    fn forecast_color_applied() {
        let bar = ProgressBar::new(0.25)
            .forecast(0.75)
            .forecast_color(Color::Yellow)
            .show_percentage(false);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        // Cell at index 5 should be forecast with custom color.
        assert_eq!(buf[(5, 0)].symbol(), FORECAST_CHAR);
        assert_eq!(buf[(5, 0)].fg, Color::Yellow);
    }
}

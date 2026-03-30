use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::display_width;
use crate::theme;

pub struct EtaDisplay {
    remaining: Option<Duration>,
    label: Option<String>,
    completed: bool,
}

impl EtaDisplay {
    pub fn new(remaining: Duration) -> Self {
        Self {
            remaining: Some(remaining),
            label: None,
            completed: false,
        }
    }

    pub fn unknown() -> Self {
        Self {
            remaining: None,
            label: None,
            completed: false,
        }
    }

    pub fn done() -> Self {
        Self {
            remaining: None,
            label: None,
            completed: true,
        }
    }

    pub fn label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    pub fn set_remaining(&mut self, remaining: Duration) {
        self.remaining = Some(remaining);
        self.completed = false;
    }

    pub fn set_done(&mut self) {
        self.completed = true;
        self.remaining = None;
    }

    pub fn set_unknown(&mut self) {
        self.remaining = None;
        self.completed = false;
    }

    fn formatted_string(&self) -> String {
        let time_part = if self.completed {
            "\u{2713} done".to_string()
        } else {
            match self.remaining {
                Some(d) => format!("\u{23f3} {}", format_duration(d)),
                None => "\u{23f3} ~:--".to_string(),
            }
        };

        match self.label {
            Some(ref l) => format!("{l} {time_part}"),
            None => time_part,
        }
    }
}

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("~{hours}:{mins:02}:{secs:02}")
    } else if mins > 0 {
        format!("~{mins}:{secs:02}")
    } else {
        format!("~0:{secs:02}")
    }
}

impl LayoutRenderable for EtaDisplay {
    fn measure(&self, constraints: Constraints) -> Size {
        let text = self.formatted_string();
        let width = constraints
            .constrain(Size::new(display_width(&text) as u16, 0))
            .width;
        Size::new(width, 1)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let text = self.formatted_string();
        let style = if self.completed {
            Style::default().fg(theme::success())
        } else if self.remaining.is_some() {
            Style::default()
                .fg(theme::warning())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::dim())
        };

        buf.set_stringn(area.x, area.y, &text, area.width as usize, style);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    fn buf_text(buf: &Buffer, area: Rect) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf[(x, area.y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn render_to_string(widget: &EtaDisplay, width: u16) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        buf_text(&buf, area)
    }

    #[test]
    fn height_always_one() {
        let eta = EtaDisplay::new(Duration::from_secs(42));
        assert_eq!(eta.measure(Constraints::loose(80, 10)).height, 1);
        assert_eq!(eta.measure(Constraints::tight(40, 5)).height, 1);
        assert_eq!(eta.measure(Constraints::unbounded()).height, 1);
    }

    #[test]
    fn renders_seconds() {
        let eta = EtaDisplay::new(Duration::from_secs(42));
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~0:42");
    }

    #[test]
    fn renders_minutes() {
        let eta = EtaDisplay::new(Duration::from_secs(3 * 60 + 5));
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~3:05");
    }

    #[test]
    fn renders_hours() {
        let eta = EtaDisplay::new(Duration::from_secs(2 * 3600 + 15 * 60));
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~2:15:00");
    }

    #[test]
    fn renders_done() {
        let eta = EtaDisplay::done();
        let text = render_to_string(&eta, 20);
        assert!(
            text.contains('\u{2713}'),
            "expected checkmark, got: {text:?}"
        );
        assert!(text.contains("done"), "expected 'done', got: {text:?}");
    }

    #[test]
    fn renders_unknown() {
        let eta = EtaDisplay::unknown();
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~:--");
    }

    #[test]
    fn label_prepended() {
        let eta = EtaDisplay::new(Duration::from_secs(3 * 60 + 42)).label("ETA".to_string());
        assert_eq!(render_to_string(&eta, 20), "ETA \u{23f3}  ~3:42");
    }

    #[test]
    fn set_methods_update_state() {
        let mut eta = EtaDisplay::unknown();
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~:--");

        eta.set_remaining(Duration::from_secs(10));
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~0:10");

        eta.set_done();
        let text = render_to_string(&eta, 20);
        assert!(
            text.contains("done"),
            "expected 'done' after set_done, got: {text:?}"
        );

        eta.set_unknown();
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~:--");
    }

    #[test]
    fn measure_width_matches_content() {
        let cases = [
            EtaDisplay::new(Duration::from_secs(42)),
            EtaDisplay::new(Duration::from_secs(3 * 60 + 5)),
            EtaDisplay::new(Duration::from_secs(2 * 3600 + 15 * 60)),
            EtaDisplay::done(),
            EtaDisplay::unknown(),
            EtaDisplay::new(Duration::from_secs(90)).label("ETA".to_string()),
        ];

        for widget in &cases {
            let measured = widget.measure(Constraints::unbounded());
            let formatted = widget.formatted_string();
            let expected_width = display_width(&formatted) as u16;
            assert_eq!(
                measured.width, expected_width,
                "measured width should match content width for {formatted:?}"
            );
        }
    }

    #[test]
    fn zero_duration() {
        let eta = EtaDisplay::new(Duration::ZERO);
        assert_eq!(render_to_string(&eta, 20), "\u{23f3}  ~0:00");
    }
}

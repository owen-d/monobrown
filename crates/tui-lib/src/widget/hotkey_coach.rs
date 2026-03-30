use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::display_width;
use crate::theme;

/// Separator between shortcuts in horizontal mode.
const SEPARATOR: &str = " \u{2502} ";
const SEPARATOR_WIDTH: u16 = 3;

/// Gap (in columns) between the key column and description column in vertical mode.
const VERTICAL_GAP: u16 = 2;

/// A single keyboard shortcut hint.
pub struct Shortcut {
    pub key: String,
    pub description: String,
}

impl Shortcut {
    pub fn new(key: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            description: description.into(),
        }
    }
}

/// Display orientation for the shortcut list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// All shortcuts on one line, separated by `│`.
    Horizontal,
    /// One shortcut per line, keys right-aligned in a column.
    Vertical,
}

/// Displays a list of keyboard shortcut hints.
///
/// Each key label is rendered with an inverted badge style, and descriptions
/// are rendered in dim text.
pub struct HotkeyCoach {
    shortcuts: Vec<Shortcut>,
    orientation: Orientation,
}

impl HotkeyCoach {
    pub fn new(shortcuts: Vec<Shortcut>) -> Self {
        Self {
            shortcuts,
            orientation: Orientation::Horizontal,
        }
    }

    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn push(&mut self, shortcut: Shortcut) {
        self.shortcuts.push(shortcut);
    }

    pub fn len(&self) -> usize {
        self.shortcuts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }

    /// Style for the key badge: inverted accent, bold.
    fn key_style() -> Style {
        Style::default()
            .fg(theme::text_on_accent())
            .bg(theme::focus())
            .add_modifier(Modifier::BOLD)
    }

    /// Style for the description text.
    fn desc_style() -> Style {
        Style::default().fg(theme::dim())
    }

    /// Max key display width across all shortcuts (including padding spaces).
    fn max_key_width(&self) -> usize {
        self.shortcuts
            .iter()
            .map(|s| display_width(&s.key) + 2)
            .max()
            .unwrap_or(0)
    }

    /// Natural width of a single shortcut entry: `" key " + " " + description`.
    fn shortcut_width(shortcut: &Shortcut) -> usize {
        display_width(&shortcut.key) + 2 + 1 + display_width(&shortcut.description)
    }

    /// Total natural width in horizontal mode (all shortcuts + separators).
    fn horizontal_natural_width(&self) -> usize {
        if self.shortcuts.is_empty() {
            return 0;
        }
        let content: usize = self.shortcuts.iter().map(Self::shortcut_width).sum();
        let separators = (self.shortcuts.len() - 1) * SEPARATOR_WIDTH as usize;
        content + separators
    }
}

impl LayoutRenderable for HotkeyCoach {
    fn measure(&self, constraints: Constraints) -> Size {
        if self.shortcuts.is_empty() {
            return Size::ZERO;
        }

        match self.orientation {
            Orientation::Horizontal => {
                let natural = self.horizontal_natural_width() as u16;
                let width = constraints
                    .constrain(Size::new(natural, 0))
                    .width
                    .max(constraints.fill_width());
                constraints.constrain(Size::new(width, 1))
            }
            Orientation::Vertical => {
                let max_key = self.max_key_width() as u16;
                let max_desc = self
                    .shortcuts
                    .iter()
                    .map(|s| display_width(&s.description))
                    .max()
                    .unwrap_or(0) as u16;
                let natural = max_key + VERTICAL_GAP + max_desc;
                let width = constraints
                    .constrain(Size::new(natural, 0))
                    .width
                    .max(constraints.fill_width());
                let height = self.shortcuts.len() as u16;
                constraints.constrain(Size::new(width, height))
            }
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.shortcuts.is_empty() {
            return;
        }

        match self.orientation {
            Orientation::Horizontal => render_horizontal(self, area, buf),
            Orientation::Vertical => render_vertical(self, area, buf),
        }
    }
}

fn render_horizontal(coach: &HotkeyCoach, area: Rect, buf: &mut Buffer) {
    let key_style = HotkeyCoach::key_style();
    let desc_style = HotkeyCoach::desc_style();
    let sep_style = Style::default().fg(theme::dim());

    let mut x = area.x;
    let right = area.x + area.width;

    for (i, shortcut) in coach.shortcuts.iter().enumerate() {
        // Separator before non-first shortcuts.
        if i > 0 {
            if x + SEPARATOR_WIDTH > right {
                break;
            }
            buf.set_stringn(x, area.y, SEPARATOR, SEPARATOR_WIDTH as usize, sep_style);
            x += SEPARATOR_WIDTH;
        }

        let padded_key = format!(" {} ", shortcut.key);
        let padded_w = display_width(&shortcut.key) as u16 + 2;
        if x + padded_w > right {
            break;
        }

        // Render key badge with padding.
        buf.set_stringn(x, area.y, &padded_key, (right - x) as usize, key_style);
        x += padded_w;

        if x >= right {
            break;
        }

        // Space between key and description.
        buf.set_stringn(x, area.y, " ", 1, Style::default());
        x += 1;

        if x >= right {
            break;
        }

        // Render description.
        let remaining = (right - x) as usize;
        let desc_w = display_width(&shortcut.description) as u16;
        buf.set_stringn(x, area.y, &shortcut.description, remaining, desc_style);
        x += desc_w.min(remaining as u16);
    }
}

fn render_vertical(coach: &HotkeyCoach, area: Rect, buf: &mut Buffer) {
    let key_style = HotkeyCoach::key_style();
    let desc_style = HotkeyCoach::desc_style();
    let max_key = coach.max_key_width() as u16;

    for (i, shortcut) in coach.shortcuts.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }

        let padded_key = format!(" {} ", shortcut.key);
        let padded_w = display_width(&shortcut.key) as u16 + 2;
        // Right-align padded key within the key column.
        let key_x = area.x + max_key.saturating_sub(padded_w);
        if key_x < area.x + area.width {
            let avail = (area.x + area.width - key_x) as usize;
            buf.set_stringn(key_x, y, &padded_key, avail, key_style);
        }

        // Description starts after the key column + gap.
        let desc_x = area.x + max_key + VERTICAL_GAP;
        if desc_x < area.x + area.width {
            let avail = (area.x + area.width - desc_x) as usize;
            buf.set_stringn(desc_x, y, &shortcut.description, avail, desc_style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_line(buf: &Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn sample_shortcuts() -> Vec<Shortcut> {
        vec![
            Shortcut::new("\u{2318}K", "save"),
            Shortcut::new("\u{2318}S", "save file"),
            Shortcut::new("j/k", "navigate"),
        ]
    }

    #[test]
    fn horizontal_height_one() {
        let coach = HotkeyCoach::new(sample_shortcuts());
        let size = coach.measure(Constraints::loose(80, 10));
        assert_eq!(size.height, 1);
    }

    #[test]
    fn vertical_height_matches_count() {
        let coach = HotkeyCoach::new(sample_shortcuts()).orientation(Orientation::Vertical);
        let size = coach.measure(Constraints::loose(80, 10));
        assert_eq!(size.height, 3);
    }

    #[test]
    fn horizontal_renders_all_keys() {
        let coach = HotkeyCoach::new(sample_shortcuts());
        let area = Rect::new(0, 0, 60, 1);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);
        let text = buf_line(&buf, 0, area.width);
        assert!(text.contains("\u{2318}K"), "missing key in {text:?}");
        assert!(text.contains("\u{2318}S"), "missing key in {text:?}");
        assert!(text.contains("j/k"), "missing key in {text:?}");
    }

    #[test]
    fn vertical_renders_all_keys() {
        let coach = HotkeyCoach::new(sample_shortcuts()).orientation(Orientation::Vertical);
        let area = Rect::new(0, 0, 40, 3);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);
        let all: String = (0..3)
            .map(|y| buf_line(&buf, y, area.width))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("\u{2318}K"), "missing key in {all:?}");
        assert!(all.contains("\u{2318}S"), "missing key in {all:?}");
        assert!(all.contains("j/k"), "missing key in {all:?}");
    }

    #[test]
    fn horizontal_separator() {
        let coach = HotkeyCoach::new(sample_shortcuts());
        let area = Rect::new(0, 0, 60, 1);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);
        let text = buf_line(&buf, 0, area.width);
        assert!(
            text.contains("\u{2502}"),
            "separator should appear: {text:?}"
        );
    }

    #[test]
    fn key_badge_style() {
        let coach = HotkeyCoach::new(vec![Shortcut::new("x", "action")]);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);

        // (0,0) is the padding space; (1,0) is the actual key character.
        let cell = &buf[(1, 0)];
        assert_eq!(cell.symbol(), "x");
        assert_eq!(cell.bg, theme::focus(), "key should have badge background");
        assert_eq!(
            cell.fg,
            theme::text_on_accent(),
            "key should have accent text color"
        );
    }

    #[test]
    fn description_rendered() {
        let coach = HotkeyCoach::new(vec![Shortcut::new("x", "do thing")]);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);
        let text = buf_line(&buf, 0, area.width);
        assert!(
            text.contains("do thing"),
            "description should appear: {text:?}"
        );
    }

    #[test]
    fn empty_renders_nothing() {
        let coach = HotkeyCoach::new(vec![]);
        assert_eq!(
            coach.measure(Constraints::loose(80, 10)),
            Size::ZERO,
            "empty coach should measure as ZERO"
        );

        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);
        let text = buf_line(&buf, 0, area.width);
        assert!(text.is_empty(), "empty coach should render nothing");
    }

    #[test]
    fn push_adds_shortcut() {
        let mut coach = HotkeyCoach::new(vec![Shortcut::new("a", "alpha")]);
        assert_eq!(coach.len(), 1);
        coach.push(Shortcut::new("b", "beta"));
        assert_eq!(coach.len(), 2);
        assert!(!coach.is_empty());
    }

    #[test]
    fn horizontal_truncation() {
        let coach = HotkeyCoach::new(vec![
            Shortcut::new("a", "alpha"),
            Shortcut::new("b", "beta"),
            Shortcut::new("c", "gamma"),
        ]);
        // " a " + " " + "alpha" = 9, " | " = 3, " b " + " " + "beta" = 8, etc.
        // Total = 35. Give only 14 columns.
        let area = Rect::new(0, 0, 14, 1);
        let mut buf = Buffer::empty(area);
        coach.render(area, &mut buf);
        let text = buf_line(&buf, 0, area.width);
        // First shortcut should appear.
        assert!(text.contains("a"), "first key should appear: {text:?}");
        assert!(text.contains("alpha"), "first desc should appear: {text:?}");
        // Third shortcut should be omitted entirely.
        assert!(
            !text.contains("gamma"),
            "last desc should be truncated away: {text:?}"
        );
    }
}

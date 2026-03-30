use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::display_width;
use crate::theme;

/// Which face of the card is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardSide {
    Front,
    Back,
}

/// A two-sided card widget with flip interaction.
///
/// Renders as a bordered box displaying either the front or back text,
/// with a `[space]` hint indicating the flip key.
#[derive(Clone)]
pub struct Flashcard {
    front: String,
    back: String,
    side: CardSide,
}

/// Minimum width the card can occupy.
const MIN_WIDTH: u16 = 10;

/// The hint shown inside the card's bottom-right corner.
const HINT: &str = "[space]";

impl Flashcard {
    pub fn new(front: String, back: String) -> Self {
        Self {
            front,
            back,
            side: CardSide::Front,
        }
    }

    pub fn flip(&mut self) {
        self.side = match self.side {
            CardSide::Front => CardSide::Back,
            CardSide::Back => CardSide::Front,
        };
    }

    pub fn side(&self) -> CardSide {
        self.side
    }

    pub fn set_side(&mut self, side: CardSide) {
        self.side = side;
    }

    pub fn front(&self) -> &str {
        &self.front
    }

    pub fn back(&self) -> &str {
        &self.back
    }

    fn active_text(&self) -> &str {
        match self.side {
            CardSide::Front => &self.front,
            CardSide::Back => &self.back,
        }
    }

    fn side_label(&self) -> &str {
        match self.side {
            CardSide::Front => "front",
            CardSide::Back => "back",
        }
    }
}

/// Greedy word-wrap: split `text` into lines that each fit within
/// `max_width` display columns. Words are never broken; a word wider
/// than `max_width` occupies its own line.
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![];
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return vec![String::new()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();
    let mut current_width: usize = 0;

    for word in &words {
        let w = display_width(word);
        if current_line.is_empty() {
            // First word on the line -- always take it.
            current_line.push_str(word);
            current_width = w;
        } else if current_width + 1 + w <= max_width {
            // Fits with a separating space.
            current_line.push(' ');
            current_line.push_str(word);
            current_width += 1 + w;
        } else {
            // Start a new line.
            lines.push(std::mem::take(&mut current_line));
            current_line.push_str(word);
            current_width = w;
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    lines
}

impl LayoutRenderable for Flashcard {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.fill_width().max(MIN_WIDTH);
        // Inner content width: border (1) + padding (1) on each side = 4.
        let inner_width = width.saturating_sub(4) as usize;
        let content_lines = word_wrap(self.active_text(), inner_width);
        // Height = top border + content lines + hint row + bottom border.
        let height = 2 + content_lines.len() as u16 + 1;
        constraints.constrain(Size::new(width, height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 3 {
            return;
        }

        let border_style = Style::default().fg(theme::border());
        let label_style = match self.side {
            CardSide::Front => Style::default().fg(theme::dim()),
            CardSide::Back => Style::default()
                .fg(theme::focus())
                .add_modifier(Modifier::BOLD),
        };
        let content_style = Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD);
        let hint_style = Style::default().fg(theme::dim());

        let x0 = area.x;
        let x1 = area.x + area.width - 1;
        let y0 = area.y;
        let y_bottom = area.y + area.height - 1;

        // --- Top border: ┌─── label ───...─┐ ---
        buf.set_string(x0, y0, "\u{256d}", border_style); // ╭
        let label = self.side_label();
        // "─── label ─" prefix after the corner
        let prefix = "\u{2500}\u{2500}\u{2500} "; // "─── "
        let prefix_width = 4;
        let suffix = " ";
        let label_display_w = display_width(label);
        // Write prefix dashes
        if area.width >= 2 {
            buf.set_string(x0 + 1, y0, prefix, border_style);
        }
        // Write label
        let label_x = x0 + 1 + prefix_width as u16;
        if (label_x as usize) < (x1 as usize) {
            buf.set_string(label_x, y0, label, label_style);
        }
        // Write suffix space + remaining dashes
        let dash_start = label_x + label_display_w as u16;
        if dash_start < x1 {
            buf.set_string(dash_start, y0, suffix, border_style);
            let fill_start = dash_start + 1;
            for x in fill_start..x1 {
                buf.set_string(x, y0, "\u{2500}", border_style);
            }
        }
        buf.set_string(x1, y0, "\u{256e}", border_style); // ╮

        // --- Side borders for all interior rows ---
        for y in (y0 + 1)..y_bottom {
            buf.set_string(x0, y, "\u{2502}", border_style); // │
            buf.set_string(x1, y, "\u{2502}", border_style); // │
        }

        // --- Content rows (word-wrapped) ---
        let inner_width = area.width.saturating_sub(4) as usize; // 1 border + 1 pad each side
        let content_x = x0 + 2; // border + padding
        let content_lines = word_wrap(self.active_text(), inner_width);
        let max_content_rows = area.height.saturating_sub(3) as usize; // top + hint + bottom
        for (i, line) in content_lines.iter().enumerate() {
            if i >= max_content_rows {
                break;
            }
            let y = y0 + 1 + i as u16;
            buf.set_stringn(content_x, y, line, inner_width, content_style);
        }

        // --- Hint row: right-aligned "[space]" inside the border ---
        let hint_y = y_bottom.saturating_sub(1);
        if hint_y > y0 {
            let hint_w = display_width(HINT);
            // Place hint at rightmost position inside border+padding.
            let hint_x = x1.saturating_sub(1 + hint_w as u16);
            if hint_x > x0 {
                buf.set_stringn(hint_x, hint_y, HINT, hint_w, hint_style);
            }
        }

        // --- Bottom border: └───...─┘ ---
        buf.set_string(x0, y_bottom, "\u{2570}", border_style); // ╰
        for x in (x0 + 1)..x1 {
            buf.set_string(x, y_bottom, "\u{2500}", border_style); // ─
        }
        buf.set_string(x1, y_bottom, "\u{256f}", border_style); // ╯
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_lines(buf: &Buffer, area: Rect) -> Vec<String> {
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn render_card(card: &Flashcard, width: u16, height: u16) -> (Buffer, Rect) {
        let size = card.measure(Constraints::loose(width, height));
        let area = Rect::new(0, 0, size.width, size.height);
        let mut buf = Buffer::empty(area);
        card.render(area, &mut buf);
        (buf, area)
    }

    #[test]
    fn starts_on_front() {
        let card = Flashcard::new("Q".into(), "A".into());
        assert_eq!(card.side(), CardSide::Front);
    }

    #[test]
    fn flip_toggles_side() {
        let mut card = Flashcard::new("Q".into(), "A".into());
        assert_eq!(card.side(), CardSide::Front);
        card.flip();
        assert_eq!(card.side(), CardSide::Back);
        card.flip();
        assert_eq!(card.side(), CardSide::Front);
    }

    #[test]
    fn front_content_shown() {
        let card = Flashcard::new("What is Rust?".into(), "A language.".into());
        let (buf, area) = render_card(&card, 30, 20);
        let lines = buf_lines(&buf, area);
        let all_text = lines.join("\n");
        assert!(
            all_text.contains("What is Rust?"),
            "front text should appear: {all_text}"
        );
    }

    #[test]
    fn back_content_shown() {
        let mut card = Flashcard::new("Q".into(), "A language.".into());
        card.flip();
        let (buf, area) = render_card(&card, 30, 20);
        let lines = buf_lines(&buf, area);
        let all_text = lines.join("\n");
        assert!(
            all_text.contains("A language."),
            "back text should appear: {all_text}"
        );
    }

    #[test]
    fn border_present() {
        let card = Flashcard::new("Q".into(), "A".into());
        let (buf, area) = render_card(&card, 20, 20);
        let lines = buf_lines(&buf, area);
        let all_text = lines.join("");
        assert!(all_text.contains('\u{256d}'), "missing top-left corner");
        assert!(all_text.contains('\u{256f}'), "missing bottom-right corner");
        assert!(all_text.contains('\u{2502}'), "missing side border");
    }

    #[test]
    fn side_label_in_border() {
        let card = Flashcard::new("Q".into(), "A".into());
        let (buf, area) = render_card(&card, 30, 20);
        let top_line = &buf_lines(&buf, area)[0];
        assert!(
            top_line.contains("front"),
            "top border should contain 'front': {top_line}"
        );

        let mut card2 = Flashcard::new("Q".into(), "A".into());
        card2.flip();
        let (buf2, area2) = render_card(&card2, 30, 20);
        let top_line2 = &buf_lines(&buf2, area2)[0];
        assert!(
            top_line2.contains("back"),
            "top border should contain 'back': {top_line2}"
        );
    }

    #[test]
    fn hint_present() {
        let card = Flashcard::new("Q".into(), "A".into());
        let (buf, area) = render_card(&card, 30, 20);
        let lines = buf_lines(&buf, area);
        let all_text = lines.join("\n");
        assert!(
            all_text.contains("[space]"),
            "hint should appear: {all_text}"
        );
    }

    #[test]
    fn word_wrap_splits_long_text() {
        let card = Flashcard::new(
            "This is a much longer piece of text that should wrap across multiple lines".into(),
            "A".into(),
        );
        let (buf, area) = render_card(&card, 20, 30);
        let lines = buf_lines(&buf, area);
        // Content area is 16 chars wide (20 - 4). The text should span
        // more than one content row.
        let content_rows: Vec<&String> = lines[1..lines.len() - 1]
            .iter()
            .filter(|l| {
                let trimmed = l.trim();
                !trimmed.is_empty() && trimmed != "\u{2502}" && !trimmed.contains("[space]")
            })
            .collect();
        assert!(
            content_rows.len() > 1,
            "long text should wrap to multiple lines, got {content_rows:?}"
        );
    }

    #[test]
    fn measure_height_includes_borders() {
        let card = Flashcard::new("Hi".into(), "Lo".into());
        let size = card.measure(Constraints::loose(30, 30));
        // top border + at least 1 content line + hint row + bottom border = 4 minimum
        assert!(
            size.height >= 4,
            "height should be at least 4, got {}",
            size.height
        );
    }

    #[test]
    fn narrow_width_still_renders() {
        let card = Flashcard::new("Hello world".into(), "Goodbye".into());
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        // Should not panic.
        card.render(area, &mut buf);
        let lines = buf_lines(&buf, area);
        let all_text = lines.join("");
        assert!(
            all_text.contains('\u{256d}'),
            "should still render border at width=10"
        );
    }
}

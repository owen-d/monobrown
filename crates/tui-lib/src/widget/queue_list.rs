use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::ellipsize_text;
use crate::theme;

/// Selected-row indicator (U+25B8, right-pointing small triangle).
const INDICATOR: &str = "\u{25B8}";
/// Scroll-up indicator shown when items are hidden above the visible window.
const SCROLL_UP: &str = "\u{25B2}";
/// Scroll-down indicator shown when items are hidden below the visible window.
const SCROLL_DOWN: &str = "\u{25BC}";

/// Interactive ordered list with a selection cursor.
///
/// Each item renders as one row: `{indicator} {index}. {label}`.
/// When items exceed the available height, a scrolled window is shown
/// centered on the selected item, with `SCROLL_UP` / `SCROLL_DOWN` indicators.
#[derive(Clone)]
pub struct QueueList {
    items: Vec<String>,
    selected: usize,
}

impl QueueList {
    pub fn new(items: Vec<String>) -> Self {
        Self { items, selected: 0 }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
        }
    }

    pub fn next(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn push(&mut self, item: String) {
        self.items.push(item);
    }

    pub fn insert(&mut self, index: usize, item: String) {
        let index = index.min(self.items.len());
        if !self.items.is_empty() && index <= self.selected {
            self.selected += 1;
        }
        self.items.insert(index, item);
    }

    pub fn remove(&mut self, index: usize) -> Option<String> {
        if index >= self.items.len() {
            return None;
        }
        let item = self.items.remove(index);
        if self.items.is_empty() {
            self.selected = 0;
        } else if index < self.selected {
            self.selected -= 1;
        } else if self.selected >= self.items.len() {
            self.selected = self.items.len() - 1;
        }
        Some(item)
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.items.len() && b < self.items.len() {
            self.items.swap(a, b);
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.items.swap(self.selected, self.selected - 1);
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.items.swap(self.selected, self.selected + 1);
            self.selected += 1;
        }
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Compute the visible window range for the current selection and height.
    ///
    /// Returns `(start, end)` indices into `self.items` such that
    /// `end - start <= height` and `self.selected` is within the range.
    fn visible_window(&self, height: usize) -> (usize, usize) {
        let count = self.items.len();
        if height >= count {
            return (0, count);
        }

        // Center the selected item in the window.
        let half = height / 2;
        let start = if self.selected <= half {
            0
        } else if self.selected + height - half > count {
            count.saturating_sub(height)
        } else {
            self.selected - half
        };
        let end = (start + height).min(count);
        (start, end)
    }

    /// Number of digits needed to display the widest 1-based index.
    fn index_width(&self) -> usize {
        if self.items.is_empty() {
            return 1;
        }
        let max_index = self.items.len();
        // Number of decimal digits in max_index.
        if max_index == 0 {
            1
        } else {
            (max_index as f64).log10().floor() as usize + 1
        }
    }
}

impl LayoutRenderable for QueueList {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.fill_width();
        let item_count = self.items.len() as u16;
        let max_height = constraints.fill_height();
        let height = item_count.min(max_height);
        Size::new(width, height)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.items.is_empty() {
            return;
        }

        let height = area.height as usize;
        let (win_start, win_end) = self.visible_window(height);
        let has_scroll_up = win_start > 0;
        let has_scroll_down = win_end < self.items.len();

        let idx_width = self.index_width();
        // Each row: "{indicator} {index}. {label}"
        // indicator = 1 char, space = 1, index = idx_width, dot = 1, space = 1, label = rest
        let prefix_width = 1 + 1 + idx_width + 1 + 1; // "X NNN. "

        let focus_style = Style::default()
            .fg(theme::focus())
            .add_modifier(Modifier::BOLD);
        let dim_style = Style::default().fg(theme::dim());

        for (row, item_idx) in (win_start..win_end).enumerate() {
            let y = area.y + row as u16;

            // Scroll indicators replace the indicator column on the first/last visible row.
            let is_first_row = row == 0;
            let is_last_row = row == height - 1 || item_idx == win_end - 1;

            if is_first_row && has_scroll_up {
                buf.set_stringn(area.x, y, SCROLL_UP, 1, dim_style);
            } else if is_last_row && has_scroll_down {
                buf.set_stringn(area.x, y, SCROLL_DOWN, 1, dim_style);
            } else if item_idx == self.selected {
                buf.set_stringn(area.x, y, INDICATOR, 1, focus_style);
            }
            // Unselected rows: the indicator column stays blank (space).

            // Index: 1-based, right-aligned within idx_width.
            let index_str = format!("{:>width$}.", item_idx + 1, width = idx_width);
            let index_x = area.x + 2; // after indicator + space
            let index_max = (area.width as usize).saturating_sub(2);
            if index_max > 0 {
                buf.set_stringn(index_x, y, &index_str, index_max, dim_style);
            }

            // Label: truncated with ellipsis if needed.
            let label_x = area.x + prefix_width as u16;
            let label_max = (area.width as usize).saturating_sub(prefix_width);
            if label_max > 0 {
                let label = ellipsize_text(&self.items[item_idx], label_max);
                let label_style = if item_idx == self.selected {
                    Style::default()
                        .fg(theme::text())
                        .add_modifier(Modifier::BOLD)
                } else {
                    dim_style
                };
                buf.set_stringn(label_x, y, &label, label_max, label_style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    /// Extract the text content of a single row from the buffer.
    fn row_text(buf: &Buffer, area: Rect, row: u16) -> String {
        let y = area.y + row;
        (area.x..area.x + area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect::<String>()
    }

    /// Extract all visible text from the buffer area, one string per row.
    fn all_rows(buf: &Buffer, area: Rect) -> Vec<String> {
        (0..area.height)
            .map(|row| row_text(buf, area, row).trim_end().to_string())
            .collect()
    }

    #[test]
    fn height_matches_items() {
        // Fewer items than available height: height = item_count.
        let ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        assert_eq!(ql.measure(Constraints::loose(40, 10)).height, 3);

        // More items than available height: height = available.
        let ql = QueueList::new(vec![
            "A".into(),
            "B".into(),
            "C".into(),
            "D".into(),
            "E".into(),
        ]);
        assert_eq!(ql.measure(Constraints::loose(40, 3)).height, 3);
    }

    #[test]
    fn renders_selected_indicator() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        ql.select(1);
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        ql.render(area, &mut buf);

        let rows = all_rows(&buf, area);
        // Selected row (index 1) should have the indicator.
        assert!(
            rows[1].contains(INDICATOR),
            "expected indicator on selected row, got {:?}",
            rows[1]
        );
        // Other rows should not.
        assert!(
            !rows[0].contains(INDICATOR),
            "unexpected indicator on row 0: {:?}",
            rows[0]
        );
        assert!(
            !rows[2].contains(INDICATOR),
            "unexpected indicator on row 2: {:?}",
            rows[2]
        );
    }

    #[test]
    fn renders_indices() {
        let ql = QueueList::new(vec!["Alpha".into(), "Beta".into(), "Gamma".into()]);
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        ql.render(area, &mut buf);

        let rows = all_rows(&buf, area);
        assert!(rows[0].contains("1."), "expected '1.' in {:?}", rows[0]);
        assert!(rows[1].contains("2."), "expected '2.' in {:?}", rows[1]);
        assert!(rows[2].contains("3."), "expected '3.' in {:?}", rows[2]);
    }

    #[test]
    fn next_clamps() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        ql.select(2);
        ql.next();
        assert_eq!(ql.selected(), 2, "next at end should clamp");
    }

    #[test]
    fn prev_clamps() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        assert_eq!(ql.selected(), 0);
        ql.prev();
        assert_eq!(ql.selected(), 0, "prev at start should clamp");
    }

    #[test]
    fn push_adds_to_end() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into()]);
        ql.push("C".into());
        assert_eq!(ql.items(), &["A", "B", "C"]);
        assert_eq!(ql.len(), 3);
    }

    #[test]
    fn remove_adjusts_selection() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into(), "D".into()]);
        ql.select(2); // selected = "C"
        ql.remove(0); // remove "A", which is before selected
        assert_eq!(
            ql.selected(),
            1,
            "removing before selected should decrement"
        );
        assert_eq!(ql.items(), &["B", "C", "D"]);
    }

    #[test]
    fn remove_at_selected() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        ql.select(2); // selected = "C" (last)
        let removed = ql.remove(2);
        assert_eq!(removed, Some("C".into()));
        assert_eq!(
            ql.selected(),
            1,
            "removing last selected should move to new last"
        );
        assert_eq!(ql.items(), &["A", "B"]);

        // Remove in the middle.
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        ql.select(1); // selected = "B"
        let removed = ql.remove(1);
        assert_eq!(removed, Some("B".into()));
        assert_eq!(ql.selected(), 1, "removing selected in middle keeps index");
        assert_eq!(ql.items()[ql.selected()], "C");
    }

    #[test]
    fn move_up_swaps() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        ql.select(1);
        ql.move_up();
        assert_eq!(ql.items(), &["B", "A", "C"]);
        assert_eq!(ql.selected(), 0);

        // At index 0, move_up is a no-op.
        ql.move_up();
        assert_eq!(ql.items(), &["B", "A", "C"]);
        assert_eq!(ql.selected(), 0);
    }

    #[test]
    fn move_down_swaps() {
        let mut ql = QueueList::new(vec!["A".into(), "B".into(), "C".into()]);
        ql.select(1);
        ql.move_down();
        assert_eq!(ql.items(), &["A", "C", "B"]);
        assert_eq!(ql.selected(), 2);

        // At last index, move_down is a no-op.
        ql.move_down();
        assert_eq!(ql.items(), &["A", "C", "B"]);
        assert_eq!(ql.selected(), 2);
    }

    #[test]
    fn scrolling_window() {
        let items: Vec<String> = (0..8)
            .map(|i| format!("Task {}", (b'A' + i) as char))
            .collect();
        let mut ql = QueueList::new(items);
        ql.select(3); // "Task D"

        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        ql.render(area, &mut buf);

        let rows = all_rows(&buf, area);

        // The visible window should contain items around index 3.
        // With 8 items and height 5, centered on index 3: window = [1..6].
        // Row 0 should show scroll-up indicator since items are hidden above.
        assert!(
            rows[0].contains(SCROLL_UP),
            "expected scroll-up indicator, got {:?}",
            rows[0]
        );
        // Last row should show scroll-down indicator since items are hidden below.
        assert!(
            rows[4].contains(SCROLL_DOWN),
            "expected scroll-down indicator, got {:?}",
            rows[4]
        );
        // The selected item "Task D" (index 4) should appear with the indicator.
        let has_selected = rows
            .iter()
            .any(|r| r.contains("Task D") && r.contains(INDICATOR));
        assert!(
            has_selected,
            "selected item with indicator not found in {rows:?}"
        );
    }

    #[test]
    fn empty_list() {
        let ql = QueueList::new(vec![]);
        assert_eq!(ql.measure(Constraints::loose(40, 10)).height, 0);
        assert!(ql.is_empty());

        // Rendering empty list should not panic.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        ql.render(area, &mut buf);
    }

    #[test]
    fn label_truncation() {
        let long_label = "This is a very long label that should be truncated".to_string();
        let ql = QueueList::new(vec![long_label]);

        // Render into a narrow area. prefix = "X N. " = 5 chars, leaving 5 for label.
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        ql.render(area, &mut buf);

        let text = row_text(&buf, area, 0);
        assert!(
            text.contains('\u{2026}'),
            "expected ellipsis in truncated label, got {text:?}"
        );
    }
}

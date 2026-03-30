use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Constraints, LayoutRenderable, Size};
use crate::render::{display_width, ellipsize_text};
use crate::theme;

const SEPARATOR: &str = " \u{2502} ";
const SEPARATOR_WIDTH: u16 = 3;

/// A horizontal tab strip with a selected tab highlighted.
#[derive(Clone)]
pub struct TabBar {
    labels: Vec<String>,
    selected: usize,
}

impl TabBar {
    pub fn new(labels: Vec<String>) -> Self {
        Self {
            labels,
            selected: 0,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        if index < self.labels.len() {
            self.selected = index;
        }
    }

    pub fn next(&mut self) {
        if !self.labels.is_empty() {
            self.selected = (self.selected + 1) % self.labels.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.labels.is_empty() {
            self.selected = (self.selected + self.labels.len() - 1) % self.labels.len();
        }
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }
}

impl LayoutRenderable for TabBar {
    fn measure(&self, constraints: Constraints) -> Size {
        Size::new(constraints.fill_width(), 1)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.labels.is_empty() {
            return;
        }

        let width = area.width as usize;
        let sep_style = Style::default().fg(theme::dim());
        let selected_style = Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD);
        let unselected_style = Style::default().fg(theme::dim());

        // Determine which labels are visible and how much space they get.
        // Strategy: find a contiguous window of tabs that includes the selected
        // tab and fits within the available width.
        let label_widths: Vec<usize> = self.labels.iter().map(|l| display_width(l)).collect();

        // Try to show as many tabs as possible starting from the left.
        // If the selected tab would not be visible, shift the window right.
        let (_start, visible_labels) = compute_visible_window(&label_widths, self.selected, width);

        let mut x = area.x;
        let right = area.x + area.width;

        for (vi, &original_idx) in visible_labels.iter().enumerate() {
            if x >= right {
                break;
            }

            // Render separator before non-first visible labels.
            if vi > 0 {
                let sep_x = x;
                if sep_x + SEPARATOR_WIDTH <= right {
                    buf.set_stringn(
                        sep_x,
                        area.y,
                        SEPARATOR,
                        SEPARATOR_WIDTH as usize,
                        sep_style,
                    );
                    x += SEPARATOR_WIDTH;
                } else {
                    break;
                }
            }

            let remaining = (right - x) as usize;
            if remaining == 0 {
                break;
            }

            let is_last_visible = vi == visible_labels.len() - 1;
            let label_text = if is_last_visible {
                // Last visible label gets whatever space remains; ellipsize if needed.
                ellipsize_text(&self.labels[original_idx], remaining)
            } else {
                // Non-last labels render at their natural width (they were
                // measured to fit during window computation).
                let natural = label_widths[original_idx];
                if natural <= remaining {
                    self.labels[original_idx].clone()
                } else {
                    ellipsize_text(&self.labels[original_idx], remaining)
                }
            };

            let style = if original_idx == self.selected {
                selected_style
            } else {
                unselected_style
            };

            let rendered_width = display_width(&label_text);
            buf.set_stringn(x, area.y, &label_text, remaining, style);
            x += rendered_width as u16;
        }
    }
}

/// Compute which tab indices are visible and fit within `width`.
///
/// Returns `(start_index, vec_of_original_indices)`.
///
/// The window always includes `selected`. Labels are shown at their natural
/// width; if the window would exceed `width` the last label in the window
/// may be ellipsized by the caller (it still counts as "in the window").
fn compute_visible_window(
    label_widths: &[usize],
    selected: usize,
    width: usize,
) -> (usize, Vec<usize>) {
    let n = label_widths.len();
    if n == 0 {
        return (0, vec![]);
    }

    // Try starting from `start` and pack as many labels as possible.
    // If `selected` is not reachable, advance `start`.
    let mut best_start = 0;

    // First, find a start such that `selected` is included.
    // Start with start=0 and see if selected fits. If not, increment start.
    'outer: for candidate_start in 0..=selected {
        let mut used = 0usize;
        let mut included_selected = false;

        for (offset, &lw) in label_widths[candidate_start..].iter().enumerate() {
            let i = candidate_start + offset;
            let sep = if offset > 0 {
                SEPARATOR_WIDTH as usize
            } else {
                0
            };
            let needed = sep + lw;
            if used + needed > width {
                if i <= selected && !included_selected {
                    continue 'outer;
                }
                break;
            }
            used += needed;
            if i == selected {
                included_selected = true;
            }
        }

        if included_selected || candidate_start == selected {
            best_start = candidate_start;
            break;
        }
    }

    // Now collect visible indices from best_start.
    let mut result = vec![];
    let mut used = 0usize;
    for (offset, &lw) in label_widths[best_start..].iter().enumerate() {
        let i = best_start + offset;
        let sep = if offset > 0 {
            SEPARATOR_WIDTH as usize
        } else {
            0
        };
        let needed = sep + lw;
        if used + needed > width && i != selected {
            break;
        }
        // If this is the selected tab and it does not fully fit, include it
        // anyway (it will be ellipsized during rendering).
        if used + needed > width && i == selected {
            result.push(i);
            break;
        }
        used += needed;
        result.push(i);
    }

    (best_start, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_text(buf: &Buffer, area: Rect) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf[(x, area.y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn renders_all_labels() {
        let tab = TabBar::new(vec!["Alpha".into(), "Beta".into(), "Gamma".into()]);
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        tab.render(area, &mut buf);
        let text = buf_text(&buf, area);
        assert!(text.contains("Alpha"), "missing Alpha in {text:?}");
        assert!(text.contains("Beta"), "missing Beta in {text:?}");
        assert!(text.contains("Gamma"), "missing Gamma in {text:?}");
    }

    #[test]
    fn selected_label_distinct() -> Result<(), Box<dyn std::error::Error>> {
        let mut tab = TabBar::new(vec!["Alpha".into(), "Beta".into(), "Gamma".into()]);
        tab.select(1);
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        tab.render(area, &mut buf);

        // Find the "B" of "Beta" and check its foreground is theme::text().
        let text_color = theme::text();
        let beta_x = (area.x..area.x + area.width)
            .find(|&x| buf[(x, area.y)].symbol() == "B")
            .ok_or("Beta should be rendered")?;
        assert_eq!(
            buf[(beta_x, area.y)].fg,
            text_color,
            "selected label should use text color"
        );

        // "Alpha" should use dim color.
        let dim_color = theme::dim();
        let alpha_x = area.x; // "Alpha" starts at x=0
        assert_eq!(
            buf[(alpha_x, area.y)].fg,
            dim_color,
            "unselected label should use dim color"
        );
        Ok(())
    }

    #[test]
    fn overflow_truncates() {
        let tab = TabBar::new(vec![
            "Alpha".into(),
            "Beta".into(),
            "Gamma".into(),
            "Delta".into(),
        ]);
        // Width too small for all four labels + separators.
        // "Alpha" + " | " + "Beta" + " | " + "Gamma" + " | " + "Delta" = 5+3+4+3+5+3+5 = 28
        let area = Rect::new(0, 0, 15, 1);
        let mut buf = Buffer::empty(area);
        tab.render(area, &mut buf);
        let text = buf_text(&buf, area);

        // First labels should appear; later ones should be truncated or missing.
        assert!(
            text.contains("Alpha"),
            "first label should appear: {text:?}"
        );
        // "Delta" should not fully appear.
        assert!(
            !text.contains("Delta"),
            "last label should be truncated away: {text:?}"
        );
    }

    #[test]
    fn next_prev_wrap() {
        let mut tab = TabBar::new(vec!["A".into(), "B".into(), "C".into()]);
        assert_eq!(tab.selected(), 0);

        tab.next();
        assert_eq!(tab.selected(), 1);
        tab.next();
        assert_eq!(tab.selected(), 2);
        tab.next(); // wraps
        assert_eq!(tab.selected(), 0);

        tab.prev(); // wraps back
        assert_eq!(tab.selected(), 2);
        tab.prev();
        assert_eq!(tab.selected(), 1);
    }

    #[test]
    fn height_always_one() {
        let tab = TabBar::new(vec!["Alpha".into(), "Beta".into()]);
        assert_eq!(tab.measure(Constraints::loose(80, 10)).height, 1);
        assert_eq!(tab.measure(Constraints::tight(40, 5)).height, 1);
        assert_eq!(tab.measure(Constraints::unbounded()).height, 1);
    }

    #[test]
    fn select_out_of_bounds_noop() {
        let mut tab = TabBar::new(vec!["A".into(), "B".into(), "C".into()]);
        tab.select(1);
        assert_eq!(tab.selected(), 1);
        tab.select(99);
        assert_eq!(tab.selected(), 1); // unchanged
    }

    #[test]
    fn separator_rendered() {
        let tab = TabBar::new(vec!["A".into(), "B".into(), "C".into()]);
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        tab.render(area, &mut buf);
        let text = buf_text(&buf, area);
        assert!(
            text.contains("\u{2502}"),
            "separator pipe should appear: {text:?}"
        );
    }
}

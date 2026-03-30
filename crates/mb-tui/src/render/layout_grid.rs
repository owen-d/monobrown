use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::layout_renderable::{Constraints, LayoutRenderable, LayoutRenderableItem, Size};

/// Responsive 2D tile layout that flows children into a grid of equal-width
/// columns. Column count is derived from available width and `min_col_width`.
pub struct GridRenderable<'a> {
    children: Vec<LayoutRenderableItem<'a>>,
    min_col_width: u16,
    gap: (u16, u16), // (horizontal, vertical)
}

struct GridPlan {
    cols: u16,
    cell_width: u16,
    row_heights: Vec<u16>,
}

impl<'a> GridRenderable<'a> {
    pub fn new(min_col_width: u16) -> Self {
        Self {
            children: Vec::new(),
            min_col_width,
            gap: (0, 0),
        }
    }

    pub fn gap(mut self, h: u16, v: u16) -> Self {
        self.gap = (h, v);
        self
    }

    pub fn push<R: LayoutRenderable + 'a>(&mut self, child: R) {
        self.children
            .push(LayoutRenderableItem::Owned(Box::new(child)));
    }

    pub fn push_ref<R: LayoutRenderable + 'a>(&mut self, child: &'a R) {
        self.children.push(LayoutRenderableItem::Borrowed(child));
    }

    fn plan(&self, available_width: u16) -> GridPlan {
        if available_width == 0 || self.children.is_empty() {
            return GridPlan {
                cols: 0,
                cell_width: 0,
                row_heights: Vec::new(),
            };
        }

        let slot = self.min_col_width.saturating_add(self.gap.0);
        let cols = available_width
            .saturating_add(self.gap.0)
            .checked_div(slot)
            .map_or(1, |v| 1.max(v));

        let total_gap = cols.saturating_sub(1).saturating_mul(self.gap.0);
        let cell_width = available_width.saturating_sub(total_gap) / cols;

        let child_constraints = Constraints::tight_width(cell_width);
        let mut row_heights: Vec<u16> = Vec::new();

        for (i, child) in self.children.iter().enumerate() {
            let row = i / cols as usize;
            let measured = child.measure(child_constraints);
            if row >= row_heights.len() {
                row_heights.push(measured.height);
            } else {
                row_heights[row] = row_heights[row].max(measured.height);
            }
        }

        GridPlan {
            cols,
            cell_width,
            row_heights,
        }
    }
}

impl LayoutRenderable for GridRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        let available_width = constraints.fill_width();
        if available_width == 0 || self.children.is_empty() {
            return Size::ZERO;
        }

        let plan = self.plan(available_width);
        if plan.row_heights.is_empty() {
            return Size::ZERO;
        }

        let total_row_height: u16 = plan
            .row_heights
            .iter()
            .fold(0u16, |acc, h| acc.saturating_add(*h));
        let total_gap = (plan.row_heights.len() as u16)
            .saturating_sub(1)
            .saturating_mul(self.gap.1);
        let total_height = total_row_height.saturating_add(total_gap);

        Size::new(available_width, total_height)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.children.is_empty() {
            return;
        }

        let plan = self.plan(area.width);
        if plan.cols == 0 {
            return;
        }

        let mut y = area.y;
        for (i, child) in self.children.iter().enumerate() {
            let col = (i % plan.cols as usize) as u16;
            let row = i / plan.cols as usize;

            if col == 0 && row > 0 {
                y = y
                    .saturating_add(plan.row_heights[row - 1])
                    .saturating_add(self.gap.1);
            }

            let x = area
                .x
                .saturating_add(col.saturating_mul(plan.cell_width.saturating_add(self.gap.0)));

            let cell_rect = Rect::new(x, y, plan.cell_width, plan.row_heights[row]);
            if !cell_rect.is_empty() {
                child.render(cell_rect, buf);
            }
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.width == 0 || area.height == 0 || self.children.is_empty() {
            return None;
        }

        let plan = self.plan(area.width);
        if plan.cols == 0 {
            return None;
        }

        let mut y = area.y;
        for (i, child) in self.children.iter().enumerate() {
            let col = (i % plan.cols as usize) as u16;
            let row = i / plan.cols as usize;

            if col == 0 && row > 0 {
                y = y
                    .saturating_add(plan.row_heights[row - 1])
                    .saturating_add(self.gap.1);
            }

            let x = area
                .x
                .saturating_add(col.saturating_mul(plan.cell_width.saturating_add(self.gap.0)));

            let cell_rect = Rect::new(x, y, plan.cell_width, plan.row_heights[row]);
            if let Some(pos) = child.cursor_pos(cell_rect) {
                return Some(pos);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: fills its area with a character, measures to constrained
    /// (w, h).
    struct FillChar {
        ch: char,
        w: u16,
        h: u16,
    }

    impl LayoutRenderable for FillChar {
        fn measure(&self, constraints: Constraints) -> Size {
            constraints.constrain(Size::new(self.w, self.h))
        }

        fn render(&self, area: Rect, buf: &mut Buffer) {
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(self.ch);
                    }
                }
            }
        }
    }

    #[test]
    fn single_column_narrow() {
        let mut grid = GridRenderable::new(25);
        grid.push(FillChar {
            ch: 'A',
            w: 25,
            h: 3,
        });
        grid.push(FillChar {
            ch: 'B',
            w: 25,
            h: 2,
        });
        grid.push(FillChar {
            ch: 'C',
            w: 25,
            h: 4,
        });

        // Width 30, min_col_width=25: (30+0)/(25+0) = 1 column
        let size = grid.measure(Constraints::tight_width(30));
        assert_eq!(size.width, 30);
        assert_eq!(size.height, 3 + 2 + 4); // sum of all child heights
    }

    #[test]
    fn multiple_columns_wide() -> Result<(), Box<dyn std::error::Error>> {
        let mut grid = GridRenderable::new(25);
        for ch in ['A', 'B', 'C', 'D', 'E', 'F'] {
            grid.push(FillChar { ch, w: 25, h: 2 });
        }

        // Width 80, min_col_width=25: (80+0)/(25+0) = 3 columns
        let size = grid.measure(Constraints::tight_width(80));
        assert_eq!(size.width, 80);
        // 6 items / 3 cols = 2 rows, each height 2
        assert_eq!(size.height, 4);

        // cell_width = 80 / 3 = 26
        let area = Rect::new(0, 0, 80, 4);
        let mut buf = Buffer::empty(area);
        grid.render(area, &mut buf);

        // Row 0: A at col 0, B at col 1, C at col 2
        assert_eq!(buf.cell((0, 0)).ok_or("cell not found")?.symbol(), "A");
        assert_eq!(buf.cell((26, 0)).ok_or("cell not found")?.symbol(), "B");
        assert_eq!(buf.cell((52, 0)).ok_or("cell not found")?.symbol(), "C");
        // Row 1: D at col 0, E at col 1, F at col 2
        assert_eq!(buf.cell((0, 2)).ok_or("cell not found")?.symbol(), "D");
        assert_eq!(buf.cell((26, 2)).ok_or("cell not found")?.symbol(), "E");
        assert_eq!(buf.cell((52, 2)).ok_or("cell not found")?.symbol(), "F");
        Ok(())
    }

    #[test]
    fn last_row_fewer_items() -> Result<(), Box<dyn std::error::Error>> {
        let mut grid = GridRenderable::new(25);
        for ch in ['A', 'B', 'C', 'D', 'E'] {
            grid.push(FillChar { ch, w: 25, h: 2 });
        }

        // 5 items in 3-column grid: 2 full rows, last row has 2 items
        let size = grid.measure(Constraints::tight_width(80));
        assert_eq!(size.height, 4); // 2 rows * height 2

        let area = Rect::new(0, 0, 80, 4);
        let mut buf = Buffer::empty(area);
        grid.render(area, &mut buf);

        // Last row: D at col 0, E at col 1
        assert_eq!(buf.cell((0, 2)).ok_or("cell not found")?.symbol(), "D");
        assert_eq!(buf.cell((26, 2)).ok_or("cell not found")?.symbol(), "E");
        // Col 2 of last row should be empty (space)
        assert_eq!(buf.cell((52, 2)).ok_or("cell not found")?.symbol(), " ");
        Ok(())
    }

    #[test]
    fn gap_spacing() -> Result<(), Box<dyn std::error::Error>> {
        let mut grid = GridRenderable::new(10).gap(2, 1);
        for ch in ['A', 'B', 'C', 'D'] {
            grid.push(FillChar { ch, w: 10, h: 2 });
        }

        // Width 30, min_col_width=10, gap_h=2:
        // cols = (30+2)/(10+2) = 2
        // cell_width = (30 - 1*2) / 2 = 14
        let size = grid.measure(Constraints::tight_width(30));
        assert_eq!(size.width, 30);
        // 2 rows of height 2 + 1 gap row = 5
        assert_eq!(size.height, 5);

        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        grid.render(area, &mut buf);

        // Row 0: A at x=0, B at x=14+2=16
        assert_eq!(buf.cell((0, 0)).ok_or("cell not found")?.symbol(), "A");
        assert_eq!(buf.cell((16, 0)).ok_or("cell not found")?.symbol(), "B");

        // Gap column (x=14..16) should be empty
        assert_eq!(buf.cell((14, 0)).ok_or("cell not found")?.symbol(), " ");
        assert_eq!(buf.cell((15, 0)).ok_or("cell not found")?.symbol(), " ");

        // Gap row (y=2) should be empty
        assert_eq!(buf.cell((0, 2)).ok_or("cell not found")?.symbol(), " ");

        // Row 1: C at y=3, D at y=3
        assert_eq!(buf.cell((0, 3)).ok_or("cell not found")?.symbol(), "C");
        assert_eq!(buf.cell((16, 3)).ok_or("cell not found")?.symbol(), "D");
        Ok(())
    }

    #[test]
    fn row_height_from_tallest() -> Result<(), Box<dyn std::error::Error>> {
        let mut grid = GridRenderable::new(25);
        grid.push(FillChar {
            ch: 'A',
            w: 25,
            h: 2,
        });
        grid.push(FillChar {
            ch: 'B',
            w: 25,
            h: 5,
        });
        grid.push(FillChar {
            ch: 'C',
            w: 25,
            h: 3,
        });
        // Second row
        grid.push(FillChar {
            ch: 'D',
            w: 25,
            h: 1,
        });

        // 3 columns at width 80
        let size = grid.measure(Constraints::tight_width(80));
        // Row 0 height = max(2, 5, 3) = 5, Row 1 height = 1
        assert_eq!(size.height, 6);

        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        grid.render(area, &mut buf);

        // B is 5 tall in first row -- verify it occupies y=0..5
        assert_eq!(buf.cell((26, 4)).ok_or("cell not found")?.symbol(), "B");
        // D starts at y=5 (after row 0's height of 5)
        assert_eq!(buf.cell((0, 5)).ok_or("cell not found")?.symbol(), "D");
        Ok(())
    }

    #[test]
    fn zero_children() {
        let grid = GridRenderable::new(25);
        let size = grid.measure(Constraints::tight_width(80));
        assert_eq!(size, Size::ZERO);
    }

    #[test]
    fn zero_width() {
        let mut grid = GridRenderable::new(25);
        grid.push(FillChar {
            ch: 'A',
            w: 25,
            h: 3,
        });

        let size = grid.measure(Constraints::tight_width(0));
        assert_eq!(size, Size::ZERO);
    }
}

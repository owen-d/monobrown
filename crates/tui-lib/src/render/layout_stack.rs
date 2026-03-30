use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::layout_renderable::{Constraints, LayoutRenderable, LayoutRenderableItem, Size};

/// Anchor point for positioning an overlay within a stack.
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

/// An overlay child with its anchor and offset from that anchor.
pub struct StackChild<'a> {
    child: LayoutRenderableItem<'a>,
    anchor: Anchor,
    offset: (u16, u16),
}

/// Z-layer overlay layout: a base child with overlays painted on top.
///
/// Overlays float above the base without affecting measured size (like
/// CSS `position: absolute`). Each overlay is anchored to a corner or
/// center of the base area and shifted by an offset.
pub struct StackRenderable<'a> {
    base: LayoutRenderableItem<'a>,
    overlays: Vec<StackChild<'a>>,
}

impl<'a> StackRenderable<'a> {
    pub fn new<R: LayoutRenderable + 'a>(base: R) -> Self {
        Self {
            base: LayoutRenderableItem::Owned(Box::new(base)),
            overlays: Vec::new(),
        }
    }

    pub fn new_ref<R: LayoutRenderable + 'a>(base: &'a R) -> Self {
        Self {
            base: LayoutRenderableItem::Borrowed(base),
            overlays: Vec::new(),
        }
    }

    pub fn overlay<R: LayoutRenderable + 'a>(
        &mut self,
        child: R,
        anchor: Anchor,
        offset: (u16, u16),
    ) {
        self.overlays.push(StackChild {
            child: LayoutRenderableItem::Owned(Box::new(child)),
            anchor,
            offset,
        });
    }

    pub fn overlay_ref<R: LayoutRenderable + 'a>(
        &mut self,
        child: &'a R,
        anchor: Anchor,
        offset: (u16, u16),
    ) {
        self.overlays.push(StackChild {
            child: LayoutRenderableItem::Borrowed(child),
            anchor,
            offset,
        });
    }
}

/// Compute the overlay rect, clamped to `area`.
fn overlay_rect(area: Rect, size: Size, anchor: &Anchor, offset: (u16, u16)) -> Rect {
    let (x, y) = match anchor {
        Anchor::TopLeft => (
            area.x.saturating_add(offset.0),
            area.y.saturating_add(offset.1),
        ),
        Anchor::TopRight => (
            area.x
                .saturating_add(area.width)
                .saturating_sub(size.width)
                .saturating_sub(offset.0),
            area.y.saturating_add(offset.1),
        ),
        Anchor::BottomLeft => (
            area.x.saturating_add(offset.0),
            area.y
                .saturating_add(area.height)
                .saturating_sub(size.height)
                .saturating_sub(offset.1),
        ),
        Anchor::BottomRight => (
            area.x
                .saturating_add(area.width)
                .saturating_sub(size.width)
                .saturating_sub(offset.0),
            area.y
                .saturating_add(area.height)
                .saturating_sub(size.height)
                .saturating_sub(offset.1),
        ),
        Anchor::Center => (
            area.x
                .saturating_add(area.width.saturating_sub(size.width) / 2)
                .saturating_add(offset.0),
            area.y
                .saturating_add(area.height.saturating_sub(size.height) / 2)
                .saturating_add(offset.1),
        ),
    };

    // Clamp to the intersection with `area` so nothing renders outside bounds.
    let right = (x.saturating_add(size.width)).min(area.x.saturating_add(area.width));
    let bottom = (y.saturating_add(size.height)).min(area.y.saturating_add(area.height));
    let clamped_x = x.max(area.x);
    let clamped_y = y.max(area.y);
    Rect::new(
        clamped_x,
        clamped_y,
        right.saturating_sub(clamped_x),
        bottom.saturating_sub(clamped_y),
    )
}

impl LayoutRenderable for StackRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.base.measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.base.render(area, buf);

        for overlay in &self.overlays {
            let size = overlay
                .child
                .measure(Constraints::loose(area.width, area.height));
            let rect = overlay_rect(area, size, &overlay.anchor, overlay.offset);
            if !rect.is_empty() {
                overlay.child.render(rect, buf);
            }
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        // Check overlays in reverse order (topmost first).
        for overlay in self.overlays.iter().rev() {
            let size = overlay
                .child
                .measure(Constraints::loose(area.width, area.height));
            let rect = overlay_rect(area, size, &overlay.anchor, overlay.offset);
            if !rect.is_empty()
                && let Some(pos) = overlay.child.cursor_pos(rect)
            {
                return Some(pos);
            }
        }
        self.base.cursor_pos(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fills its area with a single character; measures to a fixed (w, h).
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
                    buf[(x, y)].set_char(self.ch);
                }
            }
        }
    }

    /// Returns a fixed cursor position, measures to a fixed (w, h).
    struct CursorWidget {
        cursor: (u16, u16),
        w: u16,
        h: u16,
    }

    impl LayoutRenderable for CursorWidget {
        fn measure(&self, constraints: Constraints) -> Size {
            constraints.constrain(Size::new(self.w, self.h))
        }

        fn render(&self, _area: Rect, _buf: &mut Buffer) {}

        fn cursor_pos(&self, _area: Rect) -> Option<(u16, u16)> {
            Some(self.cursor)
        }
    }

    #[test]
    fn overlay_renders_on_top() {
        let base = FillChar {
            ch: 'B',
            w: 5,
            h: 3,
        };
        let overlay = FillChar {
            ch: 'O',
            w: 2,
            h: 1,
        };
        let mut stack = StackRenderable::new(base);
        stack.overlay(overlay, Anchor::TopLeft, (1, 1));

        let area = Rect::new(0, 0, 5, 3);
        let mut buf = Buffer::empty(area);
        stack.render(area, &mut buf);

        // Base fills everything with 'B'.
        assert_eq!(buf[(0, 0)].symbol(), "B");
        assert_eq!(buf[(4, 2)].symbol(), "B");
        // Overlay writes 'O' at (1,1) and (2,1).
        assert_eq!(buf[(1, 1)].symbol(), "O");
        assert_eq!(buf[(2, 1)].symbol(), "O");
        // Adjacent cells remain 'B'.
        assert_eq!(buf[(0, 1)].symbol(), "B");
        assert_eq!(buf[(3, 1)].symbol(), "B");
    }

    #[test]
    fn anchor_positions() {
        let area = Rect::new(0, 0, 10, 8);
        let size = Size::new(3, 2);
        let no_offset = (0, 0);

        let r = overlay_rect(area, size, &Anchor::TopLeft, no_offset);
        assert_eq!((r.x, r.y), (0, 0));

        let r = overlay_rect(area, size, &Anchor::TopRight, no_offset);
        assert_eq!((r.x, r.y), (7, 0));

        let r = overlay_rect(area, size, &Anchor::BottomLeft, no_offset);
        assert_eq!((r.x, r.y), (0, 6));

        let r = overlay_rect(area, size, &Anchor::BottomRight, no_offset);
        assert_eq!((r.x, r.y), (7, 6));

        let r = overlay_rect(area, size, &Anchor::Center, no_offset);
        // (10-3)/2 = 3, (8-2)/2 = 3
        assert_eq!((r.x, r.y), (3, 3));
    }

    #[test]
    fn offset_shifts_overlay() {
        let base = FillChar {
            ch: 'B',
            w: 10,
            h: 8,
        };
        let overlay = FillChar {
            ch: 'X',
            w: 1,
            h: 1,
        };
        let mut stack = StackRenderable::new(base);
        stack.overlay(overlay, Anchor::TopLeft, (2, 1));

        let area = Rect::new(0, 0, 10, 8);
        let mut buf = Buffer::empty(area);
        stack.render(area, &mut buf);

        assert_eq!(buf[(2, 1)].symbol(), "X");
        // Surrounding cells are base.
        assert_eq!(buf[(1, 1)].symbol(), "B");
        assert_eq!(buf[(3, 1)].symbol(), "B");
        assert_eq!(buf[(2, 0)].symbol(), "B");
        assert_eq!(buf[(2, 2)].symbol(), "B");
    }

    #[test]
    fn overlay_clipped_to_area() {
        let area = Rect::new(0, 0, 5, 5);
        // Overlay would extend from (3,0) to (7,2) but should be clipped to (3,0)-(4,2).
        let size = Size::new(4, 3);
        let r = overlay_rect(area, size, &Anchor::TopLeft, (3, 0));
        assert_eq!(r.x, 3);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 2); // clipped from 4 to 2
        assert_eq!(r.height, 3);

        // Verify rendering: overlay chars only appear within area.
        let base = FillChar {
            ch: '.',
            w: 5,
            h: 5,
        };
        let overlay = FillChar {
            ch: '#',
            w: 4,
            h: 3,
        };
        let mut stack = StackRenderable::new(base);
        stack.overlay(overlay, Anchor::TopLeft, (3, 0));

        let mut buf = Buffer::empty(area);
        stack.render(area, &mut buf);

        assert_eq!(buf[(3, 0)].symbol(), "#");
        assert_eq!(buf[(4, 0)].symbol(), "#");
        assert_eq!(buf[(2, 0)].symbol(), "."); // before overlay
    }

    #[test]
    fn measure_equals_base() {
        let base = FillChar {
            ch: 'B',
            w: 10,
            h: 5,
        };
        let overlay = FillChar {
            ch: 'O',
            w: 20,
            h: 20,
        };
        let mut stack = StackRenderable::new(base);
        stack.overlay(overlay, Anchor::TopLeft, (0, 0));

        let size = stack.measure(Constraints::loose(100, 100));
        assert_eq!(size, Size::new(10, 5));
    }

    #[test]
    fn cursor_pos_topmost_wins() {
        let base = CursorWidget {
            cursor: (0, 0),
            w: 10,
            h: 10,
        };
        let bottom_overlay = CursorWidget {
            cursor: (1, 1),
            w: 5,
            h: 5,
        };
        let top_overlay = CursorWidget {
            cursor: (3, 3),
            w: 5,
            h: 5,
        };
        let mut stack = StackRenderable::new(base);
        stack.overlay(bottom_overlay, Anchor::TopLeft, (0, 0));
        stack.overlay(top_overlay, Anchor::TopLeft, (0, 0));

        let area = Rect::new(0, 0, 10, 10);
        let pos = stack.cursor_pos(area);
        // Topmost overlay (last added) wins.
        assert_eq!(pos, Some((3, 3)));
    }

    #[test]
    fn cursor_pos_falls_back_to_base() {
        let base = CursorWidget {
            cursor: (5, 5),
            w: 10,
            h: 10,
        };
        // Overlay with no cursor.
        let overlay = FillChar {
            ch: 'X',
            w: 3,
            h: 3,
        };
        let mut stack = StackRenderable::new(base);
        stack.overlay(overlay, Anchor::TopLeft, (0, 0));

        let area = Rect::new(0, 0, 10, 10);
        let pos = stack.cursor_pos(area);
        assert_eq!(pos, Some((5, 5)));
    }
}

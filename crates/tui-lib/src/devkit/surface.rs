use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::{Constraints, LayoutRenderable};

/// An off-screen rendering surface for snapshot testing `LayoutRenderable` components.
///
/// Wraps a ratatui [`Buffer`] and provides plain-text and styled-text
/// extraction for use with snapshot assertions.
pub struct Surface {
    buf: Buffer,
    width: u16,
    height: u16,
}

impl Surface {
    /// Create an empty surface of the given dimensions.
    pub fn new(width: u16, height: u16) -> Self {
        let area = Rect::new(0, 0, width, height);
        Self {
            buf: Buffer::empty(area),
            width,
            height,
        }
    }

    /// Create a surface sized from tight-width measurement.
    pub fn auto(width: u16, renderable: &dyn LayoutRenderable) -> Self {
        let size = renderable.measure(Constraints::tight_width(width));
        Self::with_area(size.width, size.height, renderable)
    }

    /// Create a surface of exact dimensions and render into it.
    pub fn with_area(width: u16, height: u16, renderable: &dyn LayoutRenderable) -> Self {
        let mut surface = Self::new(width, height);
        surface.render(renderable);
        surface
    }

    /// Create a surface sized from constrained measurement, then render into it.
    pub fn auto_layout(constraints: Constraints, renderable: &dyn LayoutRenderable) -> Self {
        let size = renderable.measure(constraints);
        Self::with_layout_area(size.width, size.height, renderable)
    }

    /// Create a surface of exact dimensions and render a [`LayoutRenderable`] into it.
    pub fn with_layout_area(width: u16, height: u16, renderable: &dyn LayoutRenderable) -> Self {
        let mut surface = Self::new(width, height);
        surface.render_layout(renderable);
        surface
    }

    /// Render a [`LayoutRenderable`] into the full surface area.
    pub fn render_layout(&mut self, renderable: &dyn LayoutRenderable) {
        let area = Rect::new(0, 0, self.width, self.height);
        renderable.render(area, &mut self.buf);
    }

    /// Render a layout renderable into the full surface area.
    pub fn render(&mut self, renderable: &dyn LayoutRenderable) {
        self.render_layout(renderable);
    }

    /// The width of this surface in columns.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The height of this surface in rows.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Plain text content, trailing whitespace stripped per line.
    pub fn to_text(&self) -> String {
        super::text::buffer_to_text(&self.buf)
    }

    /// Styled text with inline annotations like `<fg:red,bold>text</>`.
    pub fn to_styled_text(&self) -> String {
        super::text::buffer_to_styled_text(&self.buf)
    }

    /// Text with ANSI escape codes for terminal rendering.
    pub fn to_ansi(&self) -> String {
        super::text::buffer_to_ansi(&self.buf)
    }

    /// Unified diff between this surface and another (plain text).
    pub fn diff(&self, other: &Surface) -> String {
        use similar::TextDiff;
        let a = self.to_text();
        let b = other.to_text();
        let diff = TextDiff::from_lines(&a, &b);
        diff.unified_diff().header("expected", "actual").to_string()
    }

    /// Direct buffer access for fine-grained assertions.
    pub fn buffer(&self) -> &Buffer {
        &self.buf
    }

    /// Mutable buffer access for direct rendering.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buf
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    use super::*;
    use crate::render::Size;
    /// Minimal test renderable that writes a single line of text.
    struct TestRenderable {
        text: &'static str,
        size: Size,
    }

    impl LayoutRenderable for TestRenderable {
        fn measure(&self, constraints: Constraints) -> Size {
            constraints.constrain(self.size)
        }

        fn render(&self, area: Rect, buf: &mut Buffer) {
            buf.set_stringn(
                area.x,
                area.y,
                self.text,
                area.width as usize,
                Style::default(),
            );
        }
    }

    struct TestLayoutRenderable {
        text: &'static str,
        size: Size,
    }

    impl LayoutRenderable for TestLayoutRenderable {
        fn measure(&self, constraints: Constraints) -> Size {
            constraints.constrain(self.size)
        }

        fn render(&self, area: Rect, buf: &mut Buffer) {
            buf.set_stringn(
                area.x,
                area.y,
                self.text,
                area.width as usize,
                Style::default(),
            );
        }
    }

    #[test]
    fn new_creates_correct_dimensions() {
        let s = Surface::new(40, 10);
        assert_eq!(s.width(), 40);
        assert_eq!(s.height(), 10);
    }

    #[test]
    fn auto_uses_tight_width_measurement() {
        let r = TestRenderable {
            text: "hello",
            size: Size::new(20, 3),
        };
        let s = Surface::auto(20, &r);
        assert_eq!(s.width(), 20);
        assert_eq!(s.height(), 3);
        assert_eq!(s.to_text(), "hello");
    }

    #[test]
    fn with_area_renders_content() {
        let r = TestRenderable {
            text: "test",
            size: Size::new(10, 1),
        };
        let s = Surface::with_area(10, 1, &r);
        assert_eq!(s.to_text(), "test");
    }

    #[test]
    fn auto_layout_uses_measured_size() {
        let r = TestLayoutRenderable {
            text: "layout",
            size: Size::new(6, 2),
        };
        let s = Surface::auto_layout(Constraints::loose(20, 10), &r);
        assert_eq!(s.width(), 6);
        assert_eq!(s.height(), 2);
        assert_eq!(s.to_text(), "layout");
    }

    #[test]
    fn to_text_strips_trailing_whitespace() {
        let r = TestRenderable {
            text: "hi",
            size: Size::new(20, 1),
        };
        let s = Surface::auto(20, &r);
        // "hi" followed by spaces should be trimmed.
        assert_eq!(s.to_text(), "hi");
    }

    #[test]
    fn diff_shows_differences() {
        let a = TestRenderable {
            text: "alpha",
            size: Size::new(10, 1),
        };
        let b = TestRenderable {
            text: "bravo",
            size: Size::new(10, 1),
        };
        let sa = Surface::auto(10, &a);
        let sb = Surface::auto(10, &b);
        let d = sa.diff(&sb);

        assert!(d.contains("expected"));
        assert!(d.contains("actual"));
        assert!(d.contains("alpha"));
        assert!(d.contains("bravo"));
    }

    #[test]
    fn diff_empty_when_identical() {
        let r = TestRenderable {
            text: "same",
            size: Size::new(10, 1),
        };
        let s1 = Surface::auto(10, &r);
        let s2 = Surface::auto(10, &r);
        let d = s1.diff(&s2);

        // similar's unified diff is empty (or just header) when inputs match.
        assert!(!d.contains('-'));
        assert!(!d.contains('+'));
    }

    #[test]
    fn empty_surface_produces_empty_text() {
        let s = Surface::new(0, 0);
        assert_eq!(s.to_text(), "");
        assert_eq!(s.to_styled_text(), "");
    }

    #[test]
    fn buffer_access() {
        let r = TestRenderable {
            text: "X",
            size: Size::new(5, 1),
        };
        let s = Surface::auto(5, &r);
        let cell = &s.buffer()[(0, 0)];
        assert_eq!(cell.symbol(), "X");
    }
}

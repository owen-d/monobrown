//! Layout-native rendering primitives and combinators.
//!
//! This module preserves the familiar container names used across the repo,
//! but all sizing now flows through [`LayoutRenderable`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use super::Insets;
use super::RectExt as _;
use super::layout_flex::{FlexFit, LayoutFlexColumn, LayoutFlexRow};
use super::layout_renderable::{
    ConstrainedRenderable, Constraints, LayoutRenderable, LayoutRenderableItem, Size,
};
use super::overflow::display_width;

pub use super::layout_renderable::LayoutRenderableItem as RenderableItem;

// ---------------------------------------------------------------------------
// Primitive impls
// ---------------------------------------------------------------------------

impl LayoutRenderable for () {
    fn measure(&self, constraints: Constraints) -> Size {
        constraints.constrain(Size::ZERO)
    }

    fn render(&self, _area: Rect, _buf: &mut Buffer) {}
}

impl LayoutRenderable for &str {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = display_width(self).min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, 1))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        buf.set_stringn(area.x, area.y, self, area.width as usize, Style::new());
    }
}

impl LayoutRenderable for String {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = display_width(self).min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, 1))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        buf.set_stringn(area.x, area.y, self, area.width as usize, Style::new());
    }
}

impl LayoutRenderable for Span<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = self.width().min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, 1))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        Widget::render(self, area, buf);
    }
}

impl LayoutRenderable for Line<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = self.width().min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, 1))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        Widget::render(self, area, buf);
    }
}

impl LayoutRenderable for Paragraph<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        let preferred_width = self.line_width().min(u16::MAX as usize) as u16;
        let width = constraints.constrain(Size::new(preferred_width, 0)).width;
        if width == 0 {
            return constraints.constrain(Size::ZERO);
        }

        let height = self.line_count(width).min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        Widget::render(self, area, buf);
    }
}

// ---------------------------------------------------------------------------
// ColumnRenderable -- vertical stack
// ---------------------------------------------------------------------------

/// Vertical stack of children measured with shared width constraints.
#[derive(Default)]
pub struct ColumnRenderable<'a> {
    inner: LayoutFlexColumn<'a>,
}

impl<'a> ColumnRenderable<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<I, T>(children: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<LayoutRenderableItem<'a>>,
    {
        let mut column = Self::new();
        for child in children {
            column.push_item(child.into());
        }
        column
    }

    pub fn push<R>(&mut self, child: R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push(0, FlexFit::Loose, child);
    }

    pub fn push_ref<R>(&mut self, child: &'a R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push_ref(0, FlexFit::Loose, child);
    }

    pub fn push_item(&mut self, child: LayoutRenderableItem<'a>) {
        self.inner.push_item(0, FlexFit::Loose, child);
    }
}

impl LayoutRenderable for ColumnRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.inner.measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.inner.render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos(area)
    }
}

// ---------------------------------------------------------------------------
// FlexRenderable -- vertical flex stack
// ---------------------------------------------------------------------------

/// Vertical flex layout preserving the historical container name.
#[derive(Default)]
pub struct FlexRenderable<'a> {
    inner: LayoutFlexColumn<'a>,
}

impl<'a> FlexRenderable<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<R>(&mut self, flex: i32, child: R)
    where
        R: LayoutRenderable + 'a,
    {
        let fit = if flex > 0 {
            FlexFit::Tight
        } else {
            FlexFit::Loose
        };
        self.inner.push(flex.max(0) as u16, fit, child);
    }

    pub fn push_ref<R>(&mut self, flex: i32, child: &'a R)
    where
        R: LayoutRenderable + 'a,
    {
        let fit = if flex > 0 {
            FlexFit::Tight
        } else {
            FlexFit::Loose
        };
        self.inner.push_ref(flex.max(0) as u16, fit, child);
    }
}

impl LayoutRenderable for FlexRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.inner.measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.inner.render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos(area)
    }
}

// ---------------------------------------------------------------------------
// RowRenderable -- horizontal layout with explicit widths
// ---------------------------------------------------------------------------

/// Horizontal layout where each child is given an explicit width.
#[derive(Default)]
pub struct RowRenderable<'a> {
    inner: LayoutFlexRow<'a>,
}

impl<'a> RowRenderable<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<R>(&mut self, width: u16, child: R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push(
            0,
            FlexFit::Loose,
            ConstrainedRenderable::new(child, Constraints::new(width, Some(width), 0, None)),
        );
    }

    pub fn push_ref<R>(&mut self, width: u16, child: &'a R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push(
            0,
            FlexFit::Loose,
            ConstrainedRenderable::new_ref(child, Constraints::new(width, Some(width), 0, None)),
        );
    }
}

impl LayoutRenderable for RowRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.inner.measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.inner.render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos(area)
    }
}

// ---------------------------------------------------------------------------
// InsetRenderable -- padding wrapper
// ---------------------------------------------------------------------------

/// Wraps a child with padding on each edge.
pub struct InsetRenderable<'a> {
    child: LayoutRenderableItem<'a>,
    insets: Insets,
}

impl<'a> InsetRenderable<'a> {
    pub fn new(child: impl Into<LayoutRenderableItem<'a>>, insets: Insets) -> Self {
        Self {
            child: child.into(),
            insets,
        }
    }

    fn child_constraints(&self, constraints: Constraints) -> Constraints {
        let horizontal = self.insets.left.saturating_add(self.insets.right);
        let vertical = self.insets.top.saturating_add(self.insets.bottom);
        Constraints::new(
            constraints.min_width.saturating_sub(horizontal),
            constraints
                .max_width
                .map(|width| width.saturating_sub(horizontal)),
            constraints.min_height.saturating_sub(vertical),
            constraints
                .max_height
                .map(|height| height.saturating_sub(vertical)),
        )
    }
}

impl LayoutRenderable for InsetRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        let child = self.child.measure(self.child_constraints(constraints));
        constraints.constrain(Size::new(
            child
                .width
                .saturating_add(self.insets.left)
                .saturating_add(self.insets.right),
            child
                .height
                .saturating_add(self.insets.top)
                .saturating_add(self.insets.bottom),
        ))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let inner = area.inset(self.insets);
        if !inner.is_empty() {
            self.child.render(inner, buf);
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.child.cursor_pos(area.inset(self.insets))
    }
}

// ---------------------------------------------------------------------------
// RenderableExt -- convenience `.inset()`
// ---------------------------------------------------------------------------

/// Convenience extension: `.inset(insets)` wraps any [`LayoutRenderable`].
pub trait RenderableExt<'a> {
    fn inset(self, insets: Insets) -> LayoutRenderableItem<'a>;
}

impl<'a, R> RenderableExt<'a> for R
where
    R: LayoutRenderable + 'a,
{
    fn inset(self, insets: Insets) -> LayoutRenderableItem<'a> {
        let child = LayoutRenderableItem::Owned(Box::new(self) as Box<dyn LayoutRenderable + 'a>);
        LayoutRenderableItem::Owned(Box::new(InsetRenderable::new(child, insets)))
    }
}

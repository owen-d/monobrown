use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Two-dimensional size used by constraint-based measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
    };

    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

/// Constraint range for width and height.
///
/// `None` for a max axis means the axis is explicitly unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Constraints {
    pub min_width: u16,
    pub max_width: Option<u16>,
    pub min_height: u16,
    pub max_height: Option<u16>,
}

impl Constraints {
    /// Create a new constraint set.
    pub fn new(
        min_width: u16,
        max_width: Option<u16>,
        min_height: u16,
        max_height: Option<u16>,
    ) -> Self {
        let mut constraints = Self {
            min_width,
            max_width,
            min_height,
            max_height,
        };
        constraints.normalize();
        constraints
    }

    /// Exact width and height.
    pub fn tight(width: u16, height: u16) -> Self {
        Self::new(width, Some(width), height, Some(height))
    }

    /// Exact width with explicit unbounded height.
    pub fn tight_width(width: u16) -> Self {
        Self::new(width, Some(width), 0, None)
    }

    /// Bounded width and height, both with zero minimum.
    pub fn loose(max_width: u16, max_height: u16) -> Self {
        Self::new(0, Some(max_width), 0, Some(max_height))
    }

    /// Fully unbounded constraints.
    pub fn unbounded() -> Self {
        Self::new(0, None, 0, None)
    }

    /// Intersect this constraint set with another.
    pub fn intersect(self, other: Self) -> Self {
        let max_width = min_optional(self.max_width, other.max_width);
        let max_height = min_optional(self.max_height, other.max_height);
        let mut constraints = Self {
            min_width: self.min_width.max(other.min_width),
            max_width,
            min_height: self.min_height.max(other.min_height),
            max_height,
        };
        constraints.normalize();
        constraints
    }

    /// Clamp a measured size into this constraint set.
    pub fn constrain(self, size: Size) -> Size {
        Size {
            width: clamp_axis(size.width, self.min_width, self.max_width),
            height: clamp_axis(size.height, self.min_height, self.max_height),
        }
    }

    /// Choose a width for "fill the offered width" widgets.
    pub fn fill_width(self) -> u16 {
        self.max_width.unwrap_or(self.min_width)
    }

    /// Choose a height for "fill the offered height" widgets.
    pub fn fill_height(self) -> u16 {
        self.max_height.unwrap_or(self.min_height)
    }

    fn normalize(&mut self) {
        if let Some(max_width) = self.max_width
            && self.min_width > max_width
        {
            self.min_width = max_width;
        }
        if let Some(max_height) = self.max_height
            && self.min_height > max_height
        {
            self.min_height = max_height;
        }
    }
}

fn min_optional(a: Option<u16>, b: Option<u16>) -> Option<u16> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn clamp_axis(value: u16, min: u16, max: Option<u16>) -> u16 {
    match max {
        Some(max) => value.max(min).min(max),
        None => value.max(min),
    }
}

/// A measurable, paintable render object using explicit constraints.
pub trait LayoutRenderable {
    /// Measure this content under the given constraints.
    fn measure(&self, constraints: Constraints) -> Size;

    /// Paint this content into the given area of the buffer.
    fn render(&self, area: Rect, buf: &mut Buffer);

    /// Optional cursor position within the given area.
    fn cursor_pos(&self, _area: Rect) -> Option<(u16, u16)> {
        None
    }
}

/// An owned or borrowed [`LayoutRenderable`].
pub enum LayoutRenderableItem<'a> {
    Owned(Box<dyn LayoutRenderable + 'a>),
    Borrowed(&'a dyn LayoutRenderable),
}

impl<'a> LayoutRenderableItem<'a> {
    fn as_dyn(&self) -> &dyn LayoutRenderable {
        match self {
            Self::Owned(child) => child.as_ref(),
            Self::Borrowed(child) => *child,
        }
    }
}

impl LayoutRenderable for LayoutRenderableItem<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.as_dyn().measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_dyn().render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.as_dyn().cursor_pos(area)
    }
}

impl<'a> From<Box<dyn LayoutRenderable + 'a>> for LayoutRenderableItem<'a> {
    fn from(value: Box<dyn LayoutRenderable + 'a>) -> Self {
        Self::Owned(value)
    }
}

impl<R> LayoutRenderable for Box<R>
where
    R: LayoutRenderable + ?Sized,
{
    fn measure(&self, constraints: Constraints) -> Size {
        self.as_ref().measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_ref().render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.as_ref().cursor_pos(area)
    }
}

impl<R: LayoutRenderable> LayoutRenderable for Option<R> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.as_ref()
            .map(|child| child.measure(constraints))
            .unwrap_or_else(|| constraints.constrain(Size::ZERO))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if let Some(child) = self {
            child.render(area, buf);
        }
    }
}

impl<R: LayoutRenderable> LayoutRenderable for Arc<R> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.as_ref().measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_ref().render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.as_ref().cursor_pos(area)
    }
}

/// Intersects incoming constraints with an additional constraint set.
pub struct ConstrainedRenderable<'a> {
    child: LayoutRenderableItem<'a>,
    constraints: Constraints,
}

impl<'a> ConstrainedRenderable<'a> {
    pub fn new<R>(child: R, constraints: Constraints) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            child: LayoutRenderableItem::Owned(Box::new(child)),
            constraints,
        }
    }

    pub fn new_ref<R>(child: &'a R, constraints: Constraints) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            child: LayoutRenderableItem::Borrowed(child),
            constraints,
        }
    }

    pub fn from_item(child: LayoutRenderableItem<'a>, constraints: Constraints) -> Self {
        Self { child, constraints }
    }
}

impl LayoutRenderable for ConstrainedRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.child.measure(constraints.intersect(self.constraints))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let size =
            self.measure(Constraints::loose(area.width, area.height).intersect(self.constraints));
        let child_area = Rect::new(area.x, area.y, size.width, size.height);
        if !child_area.is_empty() {
            self.child.render(child_area, buf);
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let size =
            self.measure(Constraints::loose(area.width, area.height).intersect(self.constraints));
        let child_area = Rect::new(area.x, area.y, size.width, size.height);
        self.child.cursor_pos(child_area)
    }
}

/// Cap a child's measured width without changing its placement policy.
pub struct MaxWidthRenderable<'a> {
    inner: ConstrainedRenderable<'a>,
}

impl<'a> MaxWidthRenderable<'a> {
    pub fn new<R>(child: R, max_width: u16) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            inner: ConstrainedRenderable::new(child, Constraints::new(0, Some(max_width), 0, None)),
        }
    }

    pub fn new_ref<R>(child: &'a R, max_width: u16) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            inner: ConstrainedRenderable::new_ref(
                child,
                Constraints::new(0, Some(max_width), 0, None),
            ),
        }
    }
}

impl LayoutRenderable for MaxWidthRenderable<'_> {
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

/// Cap a child's measured height without changing its placement policy.
pub struct MaxHeightRenderable<'a> {
    inner: ConstrainedRenderable<'a>,
}

impl<'a> MaxHeightRenderable<'a> {
    pub fn new<R>(child: R, max_height: u16) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            inner: ConstrainedRenderable::new(
                child,
                Constraints::new(0, None, 0, Some(max_height)),
            ),
        }
    }

    pub fn new_ref<R>(child: &'a R, max_height: u16) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            inner: ConstrainedRenderable::new_ref(
                child,
                Constraints::new(0, None, 0, Some(max_height)),
            ),
        }
    }
}

impl LayoutRenderable for MaxHeightRenderable<'_> {
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

/// Horizontal alignment inside an allocated rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizontalAlign {
    Start,
    Center,
    End,
}

/// Vertical alignment inside an allocated rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalAlign {
    Start,
    Center,
    End,
}

/// Align a child within the allocated render area.
pub struct AlignRenderable<'a> {
    child: LayoutRenderableItem<'a>,
    horizontal: HorizontalAlign,
    vertical: VerticalAlign,
}

impl<'a> AlignRenderable<'a> {
    pub fn new<R>(child: R, horizontal: HorizontalAlign, vertical: VerticalAlign) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            child: LayoutRenderableItem::Owned(Box::new(child)),
            horizontal,
            vertical,
        }
    }

    pub fn new_ref<R>(child: &'a R, horizontal: HorizontalAlign, vertical: VerticalAlign) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            child: LayoutRenderableItem::Borrowed(child),
            horizontal,
            vertical,
        }
    }

    pub fn from_item(
        child: LayoutRenderableItem<'a>,
        horizontal: HorizontalAlign,
        vertical: VerticalAlign,
    ) -> Self {
        Self {
            child,
            horizontal,
            vertical,
        }
    }

    fn child_area(&self, area: Rect) -> Rect {
        let size = self
            .child
            .measure(Constraints::loose(area.width, area.height));
        let x = match self.horizontal {
            HorizontalAlign::Start => area.x,
            HorizontalAlign::Center => area.x + area.width.saturating_sub(size.width) / 2,
            HorizontalAlign::End => area.x + area.width.saturating_sub(size.width),
        };
        let y = match self.vertical {
            VerticalAlign::Start => area.y,
            VerticalAlign::Center => area.y + area.height.saturating_sub(size.height) / 2,
            VerticalAlign::End => area.y + area.height.saturating_sub(size.height),
        };
        Rect::new(
            x,
            y,
            size.width.min(area.width),
            size.height.min(area.height),
        )
    }
}

impl LayoutRenderable for AlignRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.child.measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let child_area = self.child_area(area);
        if !child_area.is_empty() {
            self.child.render(child_area, buf);
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.child.cursor_pos(self.child_area(area))
    }
}

/// Center a child within the allocated render area.
pub struct CenteredRenderable<'a> {
    child: LayoutRenderableItem<'a>,
}

impl<'a> CenteredRenderable<'a> {
    pub fn new<R>(child: R) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            child: LayoutRenderableItem::Owned(Box::new(child)),
        }
    }

    pub fn new_ref<R>(child: &'a R) -> Self
    where
        R: LayoutRenderable + 'a,
    {
        Self {
            child: LayoutRenderableItem::Borrowed(child),
        }
    }

    pub fn from_item(child: LayoutRenderableItem<'a>) -> Self {
        Self { child }
    }
}

impl LayoutRenderable for CenteredRenderable<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.child.measure(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let size = self
            .child
            .measure(Constraints::loose(area.width, area.height));
        let x = area.x + area.width.saturating_sub(size.width) / 2;
        let y = area.y + area.height.saturating_sub(size.height) / 2;
        let child_area = Rect::new(
            x,
            y,
            size.width.min(area.width),
            size.height.min(area.height),
        );
        if !child_area.is_empty() {
            self.child.render(child_area, buf);
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let size = self
            .child
            .measure(Constraints::loose(area.width, area.height));
        let x = area.x + area.width.saturating_sub(size.width) / 2;
        let y = area.y + area.height.saturating_sub(size.height) / 2;
        let child_area = Rect::new(
            x,
            y,
            size.width.min(area.width),
            size.height.min(area.height),
        );
        self.child.cursor_pos(child_area)
    }
}

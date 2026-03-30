//! Composable rendering primitives for the TUI.
//!
//! # Design
//!
//! The layout system follows a **"constraints down, sizes up"** protocol
//! inspired by Flutter's layout algorithm:
//!
//! 1. A parent passes constraints down to each child.
//! 2. Each child reports its size requirement upward.
//! 3. The parent allocates rectangles and calls `render` to paint.
//!
//! `mb-tui` now standardizes on the explicit [`LayoutRenderable`] contract
//! using [`Constraints`] and [`Size`].
//!
//! # Layout combinators
//!
//! Composable containers build complex layouts from simple pieces:
//!
//! - [`ColumnRenderable`] — vertical stack; each child gets the column width.
//! - [`FlexRenderable`] — vertical stack with flex factors (Flutter-inspired);
//!   non-flex children are measured first, then remaining space is distributed
//!   proportionally among flex children.
//! - [`RowRenderable`] — horizontal layout with explicit widths per child.
//! - [`InsetRenderable`] — padding wrapper that shrinks the available area by
//!   the specified [`Insets`] before delegating to its child.
//!
//! Together with the scrollable [`LayoutPagerView`] viewport, these primitives cover
//! the full range of TUI layouts needed by the application.
//!
//! # Authoring widgets
//!
//! When writing a widget in this style, keep the contract simple:
//!
//! 1. Implement [`LayoutRenderable`] for widgets.
//! 2. Put layout-sensitive decisions in `measure`, not only in `render`.
//! 3. Let parents decide placement; children should not assume absolute
//!    coordinates or screen ownership.
//! 4. Add compact fallbacks for narrow widths instead of clipping away all
//!    meaning.
//!
//! A typical widget has two presentation modes:
//!
//! - full mode when width is sufficient
//! - summary mode when width is tight
//!
//! ```rust,ignore
//! use ratatui::{buffer::Buffer, layout::Rect};
//! use mb_tui::render::{Constraints, LayoutRenderable, Size};
//!
//! struct StatusPill {
//!     label: String,
//!     count: usize,
//! }
//!
//! impl LayoutRenderable for StatusPill {
//!     fn measure(&self, constraints: Constraints) -> Size {
//!         let width = constraints.constrain(Size::new(12, 0)).width;
//!         let height = if width < 12 { 1 } else { 3 };
//!         constraints.constrain(Size::new(width, height))
//!     }
//!
//!     fn render(&self, area: Rect, buf: &mut Buffer) {
//!         if area.width < 12 || area.height < 3 {
//!             // Compact summary: keep the most important information visible.
//!             buf.set_stringn(area.x, area.y, format!("{} {}", self.count, self.label), area.width as usize, Default::default());
//!         } else {
//!             // Full presentation.
//!             // Draw chrome, body, labels, etc.
//!         }
//!     }
//! }
//! ```
//!
//! # Which primitive to reach for
//!
//! - Use [`ColumnRenderable`] when children stack vertically and each child
//!   picks its own height.
//! - Use [`FlexRenderable`] when some children are fixed-height and others
//!   should share remaining vertical space.
//! - Use [`RowRenderable`] when horizontal widths are decided by the parent.
//! - Use [`InsetRenderable`] or [`RenderableExt::inset`] when a child should
//!   render inside padding without owning the padding logic itself.
//! - Use [`LayoutPagerView`] when the measured content height can exceed the
//!   viewport.
//!
//! # Scope
//!
//! This is Flutter-inspired, not a full clone of Flutter's box-constraint
//! system. In practice, `mb-tui` currently standardizes on:
//!
//! - constraints broadcast downward
//! - sizes reported upward
//! - parent-owned placement
//!
//! That is enough for most terminal layouts while keeping widget APIs small.

pub mod cached;
pub mod heatmap;
pub mod layout_flex;
pub mod layout_grid;
pub mod layout_pager;
pub mod layout_renderable;
pub mod layout_stack;
pub mod line_utils;
pub mod overflow;
pub mod renderable;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Widget},
};

use crate::theme;

pub use cached::CachedLayoutRenderable;
pub use heatmap::{HeatmapRamp, heatmap_style};
pub use layout_flex::{FlexFit, LayoutFlexColumn, LayoutFlexRow};
pub use layout_grid::GridRenderable;
pub use layout_pager::LayoutPagerView;
pub use layout_renderable::{
    AlignRenderable, CenteredRenderable, ConstrainedRenderable, Constraints, HorizontalAlign,
    LayoutRenderable, LayoutRenderableItem, MaxHeightRenderable, MaxWidthRenderable, Size,
    VerticalAlign,
};
pub use layout_stack::{Anchor, StackRenderable};
pub use overflow::{
    InlineOverflow, OverflowBehavior, TextOverflow, clip_text, display_width, ellipsize_text,
    fit_text, overflow_text, summarize_text, text_width,
};
pub use renderable::{
    ColumnRenderable, FlexRenderable, InsetRenderable, RenderableExt, RenderableItem, RowRenderable,
};

/// Padding offsets for each edge of a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insets {
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

impl Insets {
    /// Create insets from explicit top, left, bottom, right values.
    pub fn tlbr(top: u16, left: u16, bottom: u16, right: u16) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
        }
    }

    /// Create symmetric insets: `v` for top/bottom, `h` for left/right.
    pub fn vh(v: u16, h: u16) -> Self {
        Self {
            top: v,
            left: h,
            bottom: v,
            right: h,
        }
    }
}

/// Extension trait adding `.inset()` to [`Rect`].
pub trait RectExt {
    /// Shrink the rectangle inward by the given insets.
    fn inset(&self, insets: Insets) -> Rect;
}

impl RectExt for Rect {
    fn inset(&self, insets: Insets) -> Rect {
        let horizontal = insets.left.saturating_add(insets.right);
        let vertical = insets.top.saturating_add(insets.bottom);
        Rect {
            x: self.x.saturating_add(insets.left),
            y: self.y.saturating_add(insets.top),
            width: self.width.saturating_sub(horizontal),
            height: self.height.saturating_sub(vertical),
        }
    }
}

/// Return a centered sub-rect of the given percentages within `area`.
///
/// Used for overlay positioning (help, command palette, detail pane, graph).
pub fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    debug_assert!(percent_x <= 100 && percent_y <= 100);
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

/// Render a bordered pane frame into a buffer with focus-dependent styling.
///
/// Returns the inner `Rect` if it has nonzero dimensions, or `None` if the
/// pane is too small to render content (caller should early-return).
pub fn render_pane_frame(area: Rect, buf: &mut Buffer, title: &str, focused: bool) -> Option<Rect> {
    let border_color = if focused {
        theme::focus()
    } else {
        theme::border()
    };
    let display_title = if focused {
        format!("{title}*")
    } else {
        title.to_string()
    };
    let block = Block::default()
        .title(display_title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    Widget::render(block, area, buf);
    (inner.width > 0 && inner.height > 0).then_some(inner)
}

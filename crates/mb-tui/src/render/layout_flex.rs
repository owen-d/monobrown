use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::layout_renderable::{Constraints, LayoutRenderable, LayoutRenderableItem, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn main_max(self, constraints: Constraints) -> Option<u16> {
        match self {
            Self::Horizontal => constraints.max_width,
            Self::Vertical => constraints.max_height,
        }
    }

    fn cross_min(self, constraints: Constraints) -> u16 {
        match self {
            Self::Horizontal => constraints.min_height,
            Self::Vertical => constraints.min_width,
        }
    }

    fn cross_max(self, constraints: Constraints) -> Option<u16> {
        match self {
            Self::Horizontal => constraints.max_height,
            Self::Vertical => constraints.max_width,
        }
    }

    fn child_constraints(
        self,
        main_min: u16,
        main_max: Option<u16>,
        cross_min: u16,
        cross_max: Option<u16>,
    ) -> Constraints {
        match self {
            Self::Horizontal => Constraints::new(main_min, main_max, cross_min, cross_max),
            Self::Vertical => Constraints::new(cross_min, cross_max, main_min, main_max),
        }
    }

    fn main_size(self, size: Size) -> u16 {
        match self {
            Self::Horizontal => size.width,
            Self::Vertical => size.height,
        }
    }

    fn cross_size(self, size: Size) -> u16 {
        match self {
            Self::Horizontal => size.height,
            Self::Vertical => size.width,
        }
    }

    fn size(self, main: u16, cross: u16) -> Size {
        match self {
            Self::Horizontal => Size::new(main, cross),
            Self::Vertical => Size::new(cross, main),
        }
    }

    fn rect(self, origin: Rect, main_offset: u16, size: Size) -> Rect {
        match self {
            Self::Horizontal => Rect::new(
                origin.x.saturating_add(main_offset),
                origin.y,
                size.width,
                size.height,
            ),
            Self::Vertical => Rect::new(
                origin.x,
                origin.y.saturating_add(main_offset),
                size.width,
                size.height,
            ),
        }
    }
}

/// Whether a flex child fills its allocated share or may remain smaller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexFit {
    Tight,
    Loose,
}

struct FlexChild<'a> {
    flex: u16,
    fit: FlexFit,
    child: LayoutRenderableItem<'a>,
}

struct FlexPlan {
    size: Size,
    child_rects: Vec<Rect>,
}

struct LayoutFlexRenderable<'a> {
    axis: Axis,
    children: Vec<FlexChild<'a>>,
}

impl<'a> LayoutFlexRenderable<'a> {
    fn new(axis: Axis) -> Self {
        Self {
            axis,
            children: Vec::new(),
        }
    }

    fn push<R>(&mut self, flex: u16, fit: FlexFit, child: R)
    where
        R: LayoutRenderable + 'a,
    {
        self.children.push(FlexChild {
            flex,
            fit,
            child: LayoutRenderableItem::Owned(Box::new(child)),
        });
    }

    fn push_item(&mut self, flex: u16, fit: FlexFit, child: LayoutRenderableItem<'a>) {
        self.children.push(FlexChild { flex, fit, child });
    }

    fn push_ref<R>(&mut self, flex: u16, fit: FlexFit, child: &'a R)
    where
        R: LayoutRenderable + 'a,
    {
        self.children.push(FlexChild {
            flex,
            fit,
            child: LayoutRenderableItem::Borrowed(child),
        });
    }

    #[allow(clippy::too_many_lines, clippy::manual_checked_ops)]
    fn plan(&self, constraints: Constraints, origin: Rect) -> FlexPlan {
        let axis = self.axis;
        let cross_min = axis.cross_min(constraints);
        let cross_max = axis.cross_max(constraints);
        let mut child_sizes = vec![Size::ZERO; self.children.len()];

        if let Some(max_main) = axis.main_max(constraints) {
            let mut used_main = 0;
            let mut total_flex = 0u32;
            let mut last_flex_idx = None;

            for (i, child) in self.children.iter().enumerate() {
                if child.flex == 0 {
                    let remaining_main = max_main.saturating_sub(used_main);
                    let size = child.child.measure(axis.child_constraints(
                        0,
                        Some(remaining_main),
                        cross_min,
                        cross_max,
                    ));
                    used_main = used_main.saturating_add(axis.main_size(size));
                    child_sizes[i] = size;
                } else {
                    total_flex += child.flex as u32;
                    last_flex_idx = Some(i);
                }
            }

            let free_space = max_main.saturating_sub(used_main);
            let mut allocated_share = 0u16;
            for (i, child) in self.children.iter().enumerate() {
                if child.flex == 0 {
                    continue;
                }

                let share = if Some(i) == last_flex_idx {
                    free_space.saturating_sub(allocated_share)
                } else if total_flex == 0 {
                    0
                } else {
                    ((free_space as u32 * child.flex as u32) / total_flex) as u16
                };
                allocated_share = allocated_share.saturating_add(share);

                let main_min = match child.fit {
                    FlexFit::Tight => share,
                    FlexFit::Loose => 0,
                };
                let size = child.child.measure(axis.child_constraints(
                    main_min,
                    Some(share),
                    cross_min,
                    cross_max,
                ));
                child_sizes[i] = size;
            }
        } else {
            for (i, child) in self.children.iter().enumerate() {
                child_sizes[i] = child
                    .child
                    .measure(axis.child_constraints(0, None, cross_min, cross_max));
            }
        }

        let main = child_sizes
            .iter()
            .fold(0u16, |acc, size| acc.saturating_add(axis.main_size(*size)));
        let cross = child_sizes
            .iter()
            .fold(0u16, |acc, size| acc.max(axis.cross_size(*size)));
        let size = constraints.constrain(axis.size(main, cross));

        let mut main_offset = 0u16;
        let child_rects = child_sizes
            .into_iter()
            .map(|child_size| {
                let rect = axis.rect(origin, main_offset, child_size);
                main_offset = main_offset.saturating_add(axis.main_size(child_size));
                rect
            })
            .collect();

        FlexPlan { size, child_rects }
    }

    fn measure_inner(&self, constraints: Constraints) -> Size {
        self.plan(constraints, Rect::default()).size
    }

    fn render_inner(&self, area: Rect, buf: &mut Buffer) {
        let plan = self.plan(Constraints::loose(area.width, area.height), area);
        for (child, rect) in self.children.iter().zip(plan.child_rects) {
            if !rect.is_empty() {
                child.child.render(rect, buf);
            }
        }
    }

    fn cursor_pos_inner(&self, area: Rect) -> Option<(u16, u16)> {
        let plan = self.plan(Constraints::loose(area.width, area.height), area);
        self.children
            .iter()
            .zip(plan.child_rects)
            .find_map(|(child, rect)| {
                (!rect.is_empty())
                    .then(|| child.child.cursor_pos(rect))
                    .flatten()
            })
    }
}

/// Horizontal flex layout using the constrained layout API.
pub struct LayoutFlexRow<'a> {
    inner: LayoutFlexRenderable<'a>,
}

impl<'a> LayoutFlexRow<'a> {
    pub fn new() -> Self {
        Self {
            inner: LayoutFlexRenderable::new(Axis::Horizontal),
        }
    }

    pub fn push<R>(&mut self, flex: u16, fit: FlexFit, child: R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push(flex, fit, child);
    }

    pub fn push_item(&mut self, flex: u16, fit: FlexFit, child: LayoutRenderableItem<'a>) {
        self.inner.push_item(flex, fit, child);
    }

    pub fn push_ref<R>(&mut self, flex: u16, fit: FlexFit, child: &'a R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push_ref(flex, fit, child);
    }
}

impl Default for LayoutFlexRow<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutRenderable for LayoutFlexRow<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.inner.measure_inner(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.inner.render_inner(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos_inner(area)
    }
}

/// Vertical flex layout using the constrained layout API.
pub struct LayoutFlexColumn<'a> {
    inner: LayoutFlexRenderable<'a>,
}

impl<'a> LayoutFlexColumn<'a> {
    pub fn new() -> Self {
        Self {
            inner: LayoutFlexRenderable::new(Axis::Vertical),
        }
    }

    pub fn push<R>(&mut self, flex: u16, fit: FlexFit, child: R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push(flex, fit, child);
    }

    pub fn push_item(&mut self, flex: u16, fit: FlexFit, child: LayoutRenderableItem<'a>) {
        self.inner.push_item(flex, fit, child);
    }

    pub fn push_ref<R>(&mut self, flex: u16, fit: FlexFit, child: &'a R)
    where
        R: LayoutRenderable + 'a,
    {
        self.inner.push_ref(flex, fit, child);
    }
}

impl Default for LayoutFlexColumn<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutRenderable for LayoutFlexColumn<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        self.inner.measure_inner(constraints)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.inner.render_inner(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos_inner(area)
    }
}

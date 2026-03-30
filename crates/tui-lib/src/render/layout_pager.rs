use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

use super::layout_renderable::{Constraints, LayoutRenderable, LayoutRenderableItem};

/// Scrollable viewport over a list of constraint-based renderable chunks.
pub struct LayoutPagerView<'a> {
    pub renderables: Vec<LayoutRenderableItem<'a>>,
    pub scroll_offset: usize,
    last_content_height: Option<usize>,
    last_rendered_height: Option<usize>,
    pending_scroll_chunk: Option<usize>,
}

impl<'a> LayoutPagerView<'a> {
    pub fn new(renderables: Vec<LayoutRenderableItem<'a>>, scroll_offset: usize) -> Self {
        Self {
            renderables,
            scroll_offset,
            last_content_height: None,
            last_rendered_height: None,
            pending_scroll_chunk: None,
        }
    }

    pub fn content_height(&self, width: u16) -> usize {
        self.renderables
            .iter()
            .map(|child| child.measure(Constraints::tight_width(width)).height as usize)
            .sum()
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.last_content_height = Some(area.height as usize);
        let content_height = self.content_height(area.width);
        self.last_rendered_height = Some(content_height);

        if let Some(idx) = self.pending_scroll_chunk.take() {
            self.ensure_chunk_visible(idx, area);
        }

        self.scroll_offset = self
            .scroll_offset
            .min(content_height.saturating_sub(area.height as usize));

        self.render_content(area, buf);
    }

    pub fn scroll_chunk_into_view(&mut self, chunk_index: usize) {
        self.pending_scroll_chunk = Some(chunk_index);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = usize::MAX;
    }

    pub fn page_height(&self, viewport_area: Rect) -> usize {
        self.last_content_height
            .unwrap_or(viewport_area.height as usize)
    }

    pub fn is_scrolled_to_bottom(&self) -> bool {
        if self.scroll_offset == usize::MAX {
            return true;
        }
        let Some(viewport_h) = self.last_content_height else {
            return false;
        };
        if self.renderables.is_empty() {
            return true;
        }
        let Some(total_h) = self.last_rendered_height else {
            return false;
        };
        if total_h <= viewport_h {
            return true;
        }
        self.scroll_offset >= total_h.saturating_sub(viewport_h)
    }
}

impl LayoutPagerView<'_> {
    fn ensure_chunk_visible(&mut self, idx: usize, area: Rect) {
        if area.height == 0 || idx >= self.renderables.len() {
            return;
        }
        let first: usize = self
            .renderables
            .iter()
            .take(idx)
            .map(|child| child.measure(Constraints::tight_width(area.width)).height as usize)
            .sum();
        let chunk_h = self.renderables[idx]
            .measure(Constraints::tight_width(area.width))
            .height as usize;
        let last = first + chunk_h;
        let viewport_h = area.height as usize;
        let current_top = self.scroll_offset;
        let current_bottom = current_top.saturating_add(viewport_h.saturating_sub(1));

        if chunk_h > viewport_h {
            if last <= current_top || first > current_bottom {
                self.scroll_offset = first;
            }
        } else if first < current_top {
            self.scroll_offset = first;
        } else if last > current_bottom {
            self.scroll_offset = last.saturating_sub(viewport_h.saturating_sub(1));
        }
    }

    fn render_content(&self, area: Rect, buf: &mut Buffer) {
        let mut y = -(self.scroll_offset as isize);
        let mut drawn_bottom = area.y;
        for renderable in &self.renderables {
            let top = y;
            let height = renderable
                .measure(Constraints::tight_width(area.width))
                .height as isize;
            y += height;
            let bottom = y;
            if bottom <= 0 {
                continue;
            }
            if top >= area.height as isize {
                break;
            }

            drawn_bottom = if top < 0 {
                let drawn = render_offset_layout_content(area, buf, renderable, (-top) as u16);
                drawn_bottom.max(area.y + drawn)
            } else {
                debug_assert!(top >= 0 && top <= u16::MAX as isize, "top out of u16 range");
                let draw_h = (height as u16).min(area.height.saturating_sub(top as u16));
                let draw_area = Rect::new(area.x, area.y + top as u16, area.width, draw_h);
                renderable.render(draw_area, buf);
                drawn_bottom.max(draw_area.y.saturating_add(draw_area.height))
            };
        }

        fill_empty_rows(area, buf, drawn_bottom);
    }
}

fn fill_empty_rows(area: Rect, buf: &mut Buffer, from_y: u16) {
    for y in from_y..area.bottom() {
        if area.width == 0 {
            break;
        }
        buf[(area.x, y)] = Cell::from('~');
        for x in area.x + 1..area.right() {
            buf[(x, y)] = Cell::from(' ');
        }
    }
}

pub fn render_offset_layout_content(
    area: Rect,
    buf: &mut Buffer,
    renderable: &dyn LayoutRenderable,
    scroll_offset: u16,
) -> u16 {
    let height = renderable
        .measure(Constraints::tight_width(area.width))
        .height;
    let mut tall_buf = Buffer::empty(Rect::new(
        0,
        0,
        area.width,
        height.min(area.height + scroll_offset),
    ));
    renderable.render(*tall_buf.area(), &mut tall_buf);
    let copy_height = area
        .height
        .min(tall_buf.area().height.saturating_sub(scroll_offset));
    for y in 0..copy_height {
        let src_y = y + scroll_offset;
        for x in 0..area.width {
            buf[(area.x + x, area.y + y)] = tall_buf[(x, src_y)].clone();
        }
    }
    copy_height
}

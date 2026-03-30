//! Constraint-aware measurement caching for [`LayoutRenderable`].

use std::cell::Cell;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::layout_renderable::{Constraints, LayoutRenderable, Size};

/// A cache for [`LayoutRenderable::measure`].
pub struct CachedLayoutRenderable {
    renderable: Box<dyn LayoutRenderable>,
    size: Cell<Option<Size>>,
    last_constraints: Cell<Option<Constraints>>,
}

impl CachedLayoutRenderable {
    pub fn new<R>(renderable: R) -> Self
    where
        R: LayoutRenderable + 'static,
    {
        Self {
            renderable: Box::new(renderable),
            size: Cell::new(None),
            last_constraints: Cell::new(None),
        }
    }
}

impl LayoutRenderable for CachedLayoutRenderable {
    fn measure(&self, constraints: Constraints) -> Size {
        if self.last_constraints.get() != Some(constraints) {
            let size = self.renderable.measure(constraints);
            self.size.set(Some(size));
            self.last_constraints.set(Some(constraints));
        }
        self.size.get().unwrap_or(Size::ZERO)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.renderable.render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.renderable.cursor_pos(area)
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use ratatui::buffer::Buffer;

    use super::*;

    struct CountingLayoutRenderable {
        calls: Rc<Cell<u16>>,
    }

    impl LayoutRenderable for CountingLayoutRenderable {
        fn measure(&self, constraints: Constraints) -> Size {
            self.calls.set(self.calls.get() + 1);
            constraints.constrain(Size::new(5, 3))
        }

        fn render(&self, _area: Rect, _buf: &mut Buffer) {}
    }

    #[test]
    fn cached_layout_renderable_reuses_measure_for_identical_constraints() {
        let calls = Rc::new(Cell::new(0));
        let cached = CachedLayoutRenderable::new(CountingLayoutRenderable {
            calls: Rc::clone(&calls),
        });

        assert_eq!(cached.measure(Constraints::loose(10, 10)), Size::new(5, 3));
        assert_eq!(cached.measure(Constraints::loose(10, 10)), Size::new(5, 3));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn cached_layout_renderable_invalidates_when_constraints_change() {
        let calls = Rc::new(Cell::new(0));
        let cached = CachedLayoutRenderable::new(CountingLayoutRenderable {
            calls: Rc::clone(&calls),
        });

        cached.measure(Constraints::loose(10, 10));
        cached.measure(Constraints::loose(12, 10));
        assert_eq!(calls.get(), 2);
    }
}

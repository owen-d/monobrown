use std::cell::Cell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use mb_tui::devkit::Surface;
use mb_tui::devkit::command_palette::demo_palette;
use mb_tui::devkit::flame_graph::test_flame_graph;
use mb_tui::render::{
    AlignRenderable, CenteredRenderable, ConstrainedRenderable, Constraints, FlexFit,
    HorizontalAlign, LayoutFlexColumn, LayoutFlexRow, LayoutPagerView, LayoutRenderable,
    OverflowBehavior, Size, TextOverflow, VerticalAlign, fit_text, text_width,
};
use mb_tui::widget::VimEditor;
use mb_tui::widget::bar_selector::BarSelector;

#[derive(Clone)]
struct FixedLayoutBox {
    size: Size,
    fill: char,
}

impl LayoutRenderable for FixedLayoutBox {
    fn measure(&self, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(self.size.width, self.size.height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].set_symbol(&self.fill.to_string());
            }
        }
    }
}

#[derive(Clone)]
struct RecordingLayoutBox {
    size: Size,
    seen: Rc<Cell<Option<Constraints>>>,
}

impl LayoutRenderable for RecordingLayoutBox {
    fn measure(&self, constraints: Constraints) -> Size {
        self.seen.set(Some(constraints));
        constraints.constrain(self.size)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].set_symbol("R");
            }
        }
    }
}

#[test]
fn constraints_intersection_clamps_sizes() {
    let outer = Constraints::loose(12, 8);
    let inner = Constraints::new(4, Some(7), 2, Some(5));

    assert_eq!(
        outer.intersect(inner),
        Constraints::new(4, Some(7), 2, Some(5))
    );
    assert_eq!(inner.constrain(Size::new(20, 1)), Size::new(7, 2));
}

#[test]
fn shared_overflow_helpers_clip_and_ellipsize_text() {
    assert_eq!(text_width("hello"), 5);
    assert_eq!(fit_text("hello", 4, TextOverflow::Clip), "hell");
    assert_eq!(fit_text("hello", 4, TextOverflow::Ellipsis), "hel…");
    assert_eq!(
        OverflowBehavior::Summary.text_overflow(),
        TextOverflow::Ellipsis
    );
}

#[test]
fn primitive_text_renderables_measure_to_intrinsic_width() {
    assert_eq!("hello".measure(Constraints::loose(20, 3)), Size::new(5, 1));
    assert_eq!(
        String::from("status").measure(Constraints::loose(20, 3)),
        Size::new(6, 1)
    );
}

#[test]
fn surface_auto_measures_with_tight_width() {
    let seen = Rc::new(Cell::new(None));
    let layout = RecordingLayoutBox {
        size: Size::new(5, 3),
        seen: Rc::clone(&seen),
    };
    let surface = Surface::auto(9, &layout);

    assert_eq!(surface.width(), 9);
    assert_eq!(surface.height(), 3);
    assert_eq!(seen.get(), Some(Constraints::tight_width(9)));
}

#[test]
fn centered_wrapper_renders_child_in_the_middle_of_the_area() {
    let child = FixedLayoutBox {
        size: Size::new(2, 1),
        fill: 'X',
    };
    let centered = CenteredRenderable::new(child);
    let surface = Surface::with_area(6, 3, &centered);

    assert_eq!(surface.buffer()[(2, 1)].symbol(), "X");
    assert_eq!(surface.buffer()[(3, 1)].symbol(), "X");
    assert_eq!(surface.buffer()[(1, 1)].symbol(), " ");
    assert_eq!(surface.buffer()[(4, 1)].symbol(), " ");
}

#[test]
fn constrained_wrapper_limits_measured_size_and_painted_area() {
    let child = FixedLayoutBox {
        size: Size::new(5, 4),
        fill: '#',
    };
    let constrained = ConstrainedRenderable::new(child, Constraints::new(0, Some(3), 0, Some(2)));
    let surface = Surface::with_area(6, 4, &constrained);

    assert_eq!(constrained.measure(Constraints::tight_width(6)).height, 2);
    assert_eq!(surface.buffer()[(0, 0)].symbol(), "#");
    assert_eq!(surface.buffer()[(2, 1)].symbol(), "#");
    assert_eq!(surface.buffer()[(3, 1)].symbol(), " ");
    assert_eq!(surface.buffer()[(0, 2)].symbol(), " ");
}

#[test]
fn command_palette_measure_compacts_under_bounded_height() {
    let palette = demo_palette();

    assert_eq!(
        palette.measure(Constraints::new(20, Some(20), 0, Some(1))),
        Size::new(20, 1)
    );
    assert_eq!(
        palette.measure(Constraints::new(20, Some(20), 0, Some(20))),
        Size::new(20, 15)
    );
}

#[test]
fn command_palette_reports_intrinsic_framed_width() {
    let palette = demo_palette();
    let measured = palette.measure(Constraints::unbounded());
    let surface = Surface::auto_layout(Constraints::unbounded(), &palette);

    assert!(measured.width > 12);
    assert!(measured.height > 1);
    assert!(surface.to_text().contains("Command Palette"));
}

#[test]
fn bar_selector_reports_intrinsic_detailed_width() {
    let selector = BarSelector::new(&["Alpha", "Beta"]);
    let measured = selector.measure(Constraints::unbounded());
    let surface = Surface::auto_layout(Constraints::unbounded(), &selector);

    assert_eq!(measured, Size::new(10, 4));
    assert!(surface.to_text().contains("Alpha"));
    assert!(surface.to_text().contains("Beta"));
}

#[test]
fn vim_editor_reports_intrinsic_framed_width() {
    let mut editor = VimEditor::new();
    for ch in "hello".chars() {
        editor.step(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }

    let measured = editor.measure(Constraints::unbounded());
    let surface = Surface::auto_layout(Constraints::unbounded(), &editor);

    assert_eq!(measured.height, 4);
    assert!(measured.width > 10);
    assert!(surface.to_text().contains("mode:INSERT"));
    assert!(surface.to_text().contains("hello"));
}

#[test]
fn flame_graph_reports_intrinsic_detailed_width() {
    let flame_graph = test_flame_graph();
    let measured = flame_graph.measure(Constraints::unbounded());
    let surface = Surface::auto_layout(Constraints::unbounded(), &flame_graph);

    assert!(measured.width >= 11);
    assert_eq!(measured.height as usize, flame_graph.visible_rows().len());
    let text = surface.to_text();
    assert!(
        text.contains("request"),
        "expected flame graph content in render\n{text}"
    );
}

#[test]
fn flex_row_tight_children_fill_available_width() {
    let mut row = LayoutFlexRow::new();
    row.push(
        1,
        FlexFit::Tight,
        FixedLayoutBox {
            size: Size::new(1, 1),
            fill: 'A',
        },
    );
    row.push(
        1,
        FlexFit::Tight,
        FixedLayoutBox {
            size: Size::new(1, 1),
            fill: 'B',
        },
    );

    let surface = Surface::with_layout_area(6, 1, &row);
    assert_eq!(surface.to_text(), "AAABBB");
}

#[test]
fn flex_row_loose_children_keep_intrinsic_width() {
    let mut row = LayoutFlexRow::new();
    row.push(
        1,
        FlexFit::Loose,
        FixedLayoutBox {
            size: Size::new(1, 1),
            fill: 'A',
        },
    );
    row.push(
        1,
        FlexFit::Loose,
        FixedLayoutBox {
            size: Size::new(2, 1),
            fill: 'B',
        },
    );

    let measured = row.measure(Constraints::loose(6, 2));
    let surface = Surface::with_layout_area(6, 1, &row);

    assert_eq!(measured, Size::new(3, 1));
    assert_eq!(surface.to_text(), "ABB");
}

#[test]
fn flex_column_tight_children_fill_available_height() {
    let mut col = LayoutFlexColumn::new();
    col.push(
        1,
        FlexFit::Tight,
        FixedLayoutBox {
            size: Size::new(2, 1),
            fill: 'A',
        },
    );
    col.push(
        1,
        FlexFit::Tight,
        FixedLayoutBox {
            size: Size::new(2, 1),
            fill: 'B',
        },
    );

    let surface = Surface::with_layout_area(2, 4, &col);
    assert_eq!(surface.to_text(), "AA\nAA\nBB\nBB");
}

#[test]
fn layout_pager_renders_scrolled_content() {
    let pager_items: Vec<Box<dyn LayoutRenderable>> = vec![
        Box::new(FixedLayoutBox {
            size: Size::new(4, 2),
            fill: 'A',
        }),
        Box::new(FixedLayoutBox {
            size: Size::new(4, 2),
            fill: 'B',
        }),
    ];
    let mut pager = LayoutPagerView::new(pager_items.into_iter().map(Into::into).collect(), 1);
    let mut surface = Surface::new(4, 3);

    pager.render(Rect::new(0, 0, 4, 3), surface.buffer_mut());

    assert_eq!(surface.to_text(), "AAAA\nBBBB\nBBBB");
}

#[test]
fn align_wrapper_positions_child_at_bottom_right() {
    let aligned = AlignRenderable::new(
        FixedLayoutBox {
            size: Size::new(2, 1),
            fill: 'Z',
        },
        HorizontalAlign::End,
        VerticalAlign::End,
    );
    let surface = Surface::with_layout_area(5, 3, &aligned);

    assert_eq!(surface.buffer()[(3, 2)].symbol(), "Z");
    assert_eq!(surface.buffer()[(4, 2)].symbol(), "Z");
    assert_eq!(surface.buffer()[(2, 2)].symbol(), " ");
    assert_eq!(surface.buffer()[(3, 1)].symbol(), " ");
}

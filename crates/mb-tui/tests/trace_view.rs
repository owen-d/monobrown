//! Public trace hierarchy behavior across admission, navigation, and rendering.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mb_tui::input::KeyResult;
use mb_tui::widget::trace_view::{
    CategoryId, GroupId, ItemId, TraceCategory, TraceData, TraceError, TraceGroup, TraceItem,
    TraceTimeUnit, TraceTiming, TraceTrack, TraceView, TraceVisualRole, TrackId,
    render_trace_view_mut,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

fn item(id: u64, label: &str, timing: TraceTiming) -> TraceItem {
    TraceItem {
        id: ItemId(id),
        category: CategoryId(0),
        label: label.into(),
        timing,
        details: vec![
            ("source".into(), "fixture".into()),
            ("payload".into(), r#"{"ok":true,"count":2}"#.into()),
        ],
        children: vec![],
    }
}

fn data(items: Vec<TraceItem>) -> TraceData {
    TraceData {
        time_unit: TraceTimeUnit::Nanoseconds,
        categories: vec![TraceCategory {
            id: CategoryId(0),
            label: "activity".into(),
            role: TraceVisualRole::Neutral,
        }],
        groups: vec![TraceGroup {
            id: GroupId(10),
            label: "query 2e01ddde".into(),
        }],
        tracks: vec![TraceTrack {
            id: TrackId(0),
            group: Some(GroupId(10)),
            label: "workspace".into(),
            details: vec![],
            items,
        }],
    }
}

fn fixture() -> TraceView {
    TraceView::new(data(vec![
        item(1, "query admitted", TraceTiming::Instant(0)),
        item(
            2,
            "planning expansion scheduled",
            TraceTiming::Interval { start: 2, end: 8 },
        ),
        item(3, "query completed", TraceTiming::Instant(10)),
    ]))
    .unwrap()
}

fn frame(view: &mut TraceView, width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    render_trace_view_mut(view, area, &mut buffer);
    buffer
}

fn text(buffer: &Buffer) -> String {
    (buffer.area.top()..buffer.area.bottom())
        .map(|y| {
            let mut row = String::new();
            let mut x = buffer.area.left();
            while x < buffer.area.right() {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                x += symbol.width().max(1) as u16;
            }
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn finish(view: &mut TraceView) {
    for _ in 0..96 {
        view.tick(Duration::from_millis(16));
    }
}

#[test]
fn selected_row_expands_observed_time_and_details() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(2)));
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    finish(&mut view);
    let rendered = text(&frame(&mut view, 140, 20));
    assert!(rendered.contains("Observed UTC"));
    assert!(rendered.contains("00:00:00"));
    assert!(rendered.contains("observed UTC:"));
    assert!(rendered.contains("source: fixture"));
    assert!(rendered.contains("payload:"));
    assert!(rendered.contains("true"));
}

#[test]
fn span_expands_to_endpoint_children() {
    let mut span = item(
        10,
        "tool: inspect_result",
        TraceTiming::Interval { start: 10, end: 20 },
    );
    span.children = vec![
        item(11, "inspect_result call", TraceTiming::Instant(10)),
        item(12, "tool output", TraceTiming::Instant(20)),
    ];
    let mut view = TraceView::new(data(vec![span])).unwrap();
    assert!(view.select_item(ItemId(10)));
    assert_eq!(view.visible_row_count(), 4);
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(view.selected_item().unwrap().id, ItemId(11));
    assert_eq!(view.visible_row_count(), 6);

    // Rendering the selected child exercises recursive item lookup; before
    // the hierarchy fix this path panicked because only top-level items were
    // searched.
    let rendered = text(&frame(&mut view, 100, 12));
    assert!(rendered.contains("inspect_result c"));

    assert!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            == KeyResult::Consumed
    );
    assert_eq!(view.visible_row_count(), 4);
}

#[test]
fn trace_disclosure_preserves_sibling_order_and_focus_parent() {
    let mut reserve = item(
        3,
        "tool: reserve_wakeup",
        TraceTiming::Interval { start: 5, end: 20 },
    );
    reserve.children = vec![
        item(4, "reserve_wakeup call", TraceTiming::Instant(5)),
        item(5, "tool output", TraceTiming::Instant(20)),
    ];
    let mut view = TraceView::new(data(vec![
        item(1, "tool: unknown", TraceTiming::Instant(0)),
        item(2, "reasoning", TraceTiming::Instant(2)),
        reserve,
    ]))
    .unwrap();
    assert!(view.select_item(ItemId(3)));

    let rendered = text(&frame(&mut view, 140, 20));
    let unknown = rendered.find("tool: unknown").unwrap();
    let reasoning = rendered.find("reasoning").unwrap();
    let reserve = rendered.find("tool: reserve_wakeup").unwrap();
    assert!(unknown < reasoning && reasoning < reserve);

    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    let rendered = text(&frame(&mut view, 140, 20));
    let unknown = rendered.find("tool: unknown").unwrap();
    let reasoning = rendered.find("reasoning").unwrap();
    let reserve = rendered.find("tool: reserve_wakeup").unwrap();
    let call = rendered.find("reserve_wakeup call").unwrap();
    assert!(unknown < reasoning && reasoning < reserve && reserve < call);
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );

    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );

    let rendered = text(&frame(&mut view, 140, 20));
    let unknown = rendered.find("tool: unknown").unwrap();
    let reasoning = rendered.find("reasoning").unwrap();
    let reserve = rendered.find("tool: reserve_wakeup").unwrap();
    assert!(unknown < reasoning && reasoning < reserve);
    assert!(!rendered.contains("reserve_wakeup call"));
}

#[test]
fn selected_event_reveals_clean_hierarchy_and_labeled_bars() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(2)));
    finish(&mut view);
    let rendered = text(&frame(&mut view, 80, 12));
    assert!(!rendered.lines().any(|line| line.contains("▾ trace")));
    assert!(rendered.contains("▾ query 2e01ddde"));
    assert!(rendered.contains("▾ workspace"));
    assert!(rendered.contains("query adm"));
    assert!(rendered.contains("planning"));
    assert!(rendered.contains("query com"));
    assert!(rendered.contains('━'));
    assert!(!rendered.contains('│'));
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));
}

#[test]
fn root_track_is_not_rendered_twice_as_group_and_track() {
    let mut input = data(vec![item(1, "observed", TraceTiming::Instant(0))]);
    input.groups[0].label = "workspace".into();
    input.tracks[0].label = "workspace".into();
    let mut view = TraceView::new(input).unwrap();
    finish(&mut view);
    let rendered = text(&frame(&mut view, 100, 10));
    assert_eq!(
        rendered
            .lines()
            .skip(2)
            .filter(|line| line.contains("workspace"))
            .count(),
        1,
        "the source track should appear once under its structural group"
    );
}

#[test]
fn trace_vertical_navigation_stays_within_the_selected_track() {
    let mut input = data(vec![
        item(1, "first", TraceTiming::Instant(0)),
        item(2, "last", TraceTiming::Instant(1)),
    ]);
    input.tracks.push(TraceTrack {
        id: TrackId(1),
        group: Some(GroupId(10)),
        label: "stage 0".into(),
        details: vec![],
        items: vec![item(3, "aunt", TraceTiming::Instant(2))],
    });
    let mut view = TraceView::new(input).unwrap();
    assert!(view.select_item(ItemId(2)));
    finish(&mut view);
    view.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));

    let ctrl_j = KeyEvent {
        code: KeyCode::Char('j'),
        modifiers: KeyModifiers::CONTROL,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    };
    view.handle_key(&ctrl_j);
    assert_eq!(view.selected_track().unwrap().id, TrackId(1));
    let ctrl_k = KeyEvent {
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::CONTROL,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    };
    view.handle_key(&ctrl_k);
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));
}

#[test]
fn trace_view_navigation_uses_immediate_disclosure() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(1)));
    assert!(!view.needs_idle_render());
    finish(&mut view);
    assert!(!view.is_group_collapsed(GroupId(10)));
    assert!(!view.needs_idle_render());
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));
    assert!(!view.is_group_collapsed(GroupId(10)));
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));
}

#[test]
fn hidden_trace_root_is_not_navigable() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(1)));
    for _ in 0..2 {
        assert_eq!(
            view.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
            KeyResult::Consumed
        );
    }
    assert_eq!(view.selected_group(), Some(GroupId(10)));
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(view.selected_group(), Some(GroupId(10)));
}

#[test]
fn collapsing_and_reselecting_preserve_structural_identity() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(3)));
    assert!(view.set_group_collapsed(GroupId(10), true));
    assert!(view.is_group_collapsed(GroupId(10)));
    assert!(view.selected_item().is_none());
    assert_eq!(view.selected_group(), Some(GroupId(10)));
    assert!(view.select_item(ItemId(1)));
    assert!(!view.is_group_collapsed(GroupId(10)));
    assert_eq!(view.selected_track().unwrap().id, TrackId(0));
}

#[test]
fn exact_time_range_remains_available_to_non_terminal_consumers() {
    let mut view = fixture();
    assert_eq!(view.time_window(), (0, 10));
    view.set_time_window(2, 8).unwrap();
    assert_eq!(view.time_window(), (2, 8));
    view.fit_time();
    assert_eq!(view.time_window(), (0, 10));
    assert_eq!(
        view.set_time_window(8, 8),
        Err(TraceError::InvalidTimeWindow)
    );
}

#[test]
fn duplicate_ids_and_unknown_references_are_rejected() {
    let observation = item(0, "one", TraceTiming::Untimed);
    assert!(matches!(
        TraceView::new(data(vec![observation.clone(), observation])),
        Err(TraceError::DuplicateItem(ItemId(0)))
    ));
    let mut input = data(vec![]);
    input.tracks.push(input.tracks[0].clone());
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::DuplicateTrack(TrackId(0)))
    ));
    let mut input = data(vec![]);
    input.tracks[0].group = Some(GroupId(9));
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::UnknownGroup(GroupId(9)))
    ));
    let mut input = data(vec![]);
    input.groups.push(input.groups[0].clone());
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::DuplicateGroup(GroupId(10)))
    ));
    let mut input = data(vec![item(1, "one", TraceTiming::Instant(0))]);
    input.categories.push(input.categories[0].clone());
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::DuplicateCategory(CategoryId(0)))
    ));
    let mut input = data(vec![item(1, "one", TraceTiming::Instant(0))]);
    input.tracks[0].items[0].category = CategoryId(9);
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::UnknownCategory(CategoryId(9)))
    ));
}

#[test]
fn admission_enforces_collection_and_text_limits() {
    let observation = item(0, "", TraceTiming::Untimed);
    let mut input = data(vec![]);
    input.categories = (0..257)
        .map(|id| TraceCategory {
            id: CategoryId(id),
            label: String::new(),
            role: TraceVisualRole::Neutral,
        })
        .collect();
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::LimitExceeded("categories (256)"))
    ));
    assert!(matches!(
        TraceView::new(data(vec![observation.clone(); 65537])),
        Err(TraceError::LimitExceeded("items (65536)"))
    ));
    let mut detailed = observation.clone();
    detailed.details = vec![(String::new(), String::new()); 33];
    assert!(matches!(
        TraceView::new(data(vec![detailed])),
        Err(TraceError::LimitExceeded("detail pairs per item (32)"))
    ));
    let mut long = observation;
    long.label = "x".repeat(4097);
    assert!(matches!(
        TraceView::new(data(vec![long])),
        Err(TraceError::LimitExceeded("bytes per field (4096)"))
    ));
}

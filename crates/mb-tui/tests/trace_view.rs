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
        details: vec![],
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
fn selected_event_reveals_clean_hierarchy_and_labeled_bars() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(2)));
    finish(&mut view);
    let rendered = text(&frame(&mut view, 80, 12));
    assert!(rendered.contains("▾ trace"));
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
fn flame_graph_navigation_and_animation_are_reused() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(1)));
    assert!(view.needs_idle_render());
    finish(&mut view);
    assert!(!view.needs_idle_render());
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
        KeyResult::Consumed
    );
    assert_eq!(view.selected_item().unwrap().id, ItemId(2));
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

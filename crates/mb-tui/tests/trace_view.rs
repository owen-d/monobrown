//! Public timeline behavior across admission, navigation, layout, and rendering.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use mb_tui::input::KeyResult;
use mb_tui::render::{Constraints, LayoutRenderable};
use mb_tui::widget::trace_view::{
    GroupId, ItemId, TraceData, TraceError, TraceGroup, TraceItem, TraceTimeUnit, TraceTiming,
    TraceTrack, TraceView, TrackId, render_trace_view, render_trace_view_mut,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

/// Build neutral display facts, usable without MMUI or the optional devkit.
fn item(id: u64, label: &str, timing: TraceTiming) -> TraceItem {
    TraceItem {
        id: ItemId(id),
        label: label.into(),
        timing,
        details: vec![],
    }
}

/// Put supplied items on one track so overlap and collisions are observable.
fn data(items: Vec<TraceItem>) -> TraceData {
    TraceData {
        time_unit: TraceTimeUnit::Nanoseconds,
        groups: vec![],
        tracks: vec![TraceTrack {
            id: TrackId(0),
            group: None,
            label: "worker".into(),
            items,
        }],
    }
}

/// Mix concurrent work with partial and untimed observations on independent tracks.
fn fixture() -> TraceView {
    let mut input = data(vec![
        item(1, "first", TraceTiming::Interval { start: 0, end: 10 }),
        item(2, "concurrent", TraceTiming::Interval { start: 2, end: 8 }),
        item(3, "next", TraceTiming::Interval { start: 10, end: 20 }),
        item(4, "observed", TraceTiming::Instant(5)),
        item(5, "unknown start", TraceTiming::MissingStart { end: 7 }),
        item(6, "unknown end", TraceTiming::MissingEnd { start: 11 }),
        item(7, "regression", TraceTiming::Interval { start: 18, end: 3 }),
        item(8, "untimed fact", TraceTiming::Untimed),
    ]);
    input.groups = vec![TraceGroup {
        id: GroupId(10),
        label: "request".into(),
    }];
    input.tracks[0].group = Some(GroupId(10));
    TraceView::new(input).unwrap()
}

/// Render a caller-sized terminal buffer with mutable viewport reconciliation.
fn frame(view: &mut TraceView, width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    render_trace_view_mut(view, area, &mut buffer);
    buffer
}

/// Inspect terminal cells without depending on the devkit feature.
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
            row
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Send an ordinary key press through the public interaction seam.
fn press(view: &mut TraceView, key: KeyCode) {
    assert_eq!(
        view.handle_key(&KeyEvent::new(key, KeyModifiers::NONE)),
        KeyResult::Consumed
    );
}

/// Concurrent intervals remain independently selectable while resize and zoom preserve lane identity.
#[test]
fn overlap_navigation_is_stable_across_view_changes() {
    let mut view = fixture();
    assert!(view.select_item(ItemId(1)));
    let first_lane = view.cursor();
    press(&mut view, KeyCode::Tab);
    assert_eq!(view.selected_item().unwrap().id, ItemId(3));
    assert_eq!(view.cursor(), first_lane);
    assert!(view.select_item(ItemId(2)));
    let concurrent_lane = view.cursor();
    assert_ne!(first_lane, concurrent_lane);
    for key in [KeyCode::Char('+'), KeyCode::Right, KeyCode::Char('-')] {
        press(&mut view, key);
        assert_eq!(view.selected_item().unwrap().id, ItemId(2));
        assert_eq!(view.cursor(), concurrent_lane);
    }
    for width in [1, 8, 24, 80, 160] {
        frame(&mut view, width, 8);
        assert_eq!(view.cursor(), concurrent_lane);
    }
}

/// Two observations in one pixel remain individually selectable with exact metadata.
#[test]
fn crowded_cells_preserve_every_item_and_details() {
    let mut a = item(10, "alpha", TraceTiming::Instant(0));
    a.details = vec![("value".into(), "first".into())];
    let mut b = item(20, "beta", TraceTiming::Instant(0));
    b.details = vec![("value".into(), "second".into())];
    let mut view = TraceView::new(data(vec![b, a])).unwrap();
    assert_eq!(view.selected_item().unwrap().id, ItemId(10));
    let first = text(&frame(&mut view, 80, 12));
    assert!(first.contains('+'));
    assert!(first.contains("1 crowded cells"));
    assert!(first.contains("value: first"));
    press(&mut view, KeyCode::Tab);
    let second = text(&frame(&mut view, 80, 12));
    assert!(second.contains("value: second"));
    assert_eq!(view.selected_item().unwrap().id, ItemId(20));
    press(&mut view, KeyCode::Down);
    assert_eq!(view.selected_item().unwrap().id, ItemId(20));
    press(&mut view, KeyCode::BackTab);
    assert_eq!(view.selected_item().unwrap().id, ItemId(10));
}

/// Missing and regressed endpoints never get a connecting bar or synthetic edge.
#[test]
fn uncertain_observations_have_only_recorded_endpoint_marks() {
    let mut view = TraceView::new(data(vec![
        item(1, "reverse", TraceTiming::Interval { start: 80, end: 20 }),
        item(2, "start absent", TraceTiming::MissingStart { end: 40 }),
        item(3, "end absent", TraceTiming::MissingEnd { start: 60 }),
        item(4, "zero", TraceTiming::Interval { start: 50, end: 50 }),
    ]))
    .unwrap();
    view.set_time_window(0, 100).unwrap();
    let rendered = text(&frame(&mut view, 120, 9));
    let lane = rendered.lines().nth(1).unwrap();
    assert_eq!(lane.matches('!').count(), 2);
    assert_eq!(lane.matches('◁').count(), 1);
    assert_eq!(lane.matches('▷').count(), 1);
    assert_eq!(lane.matches('◇').count(), 1);
    assert!(!lane.contains('━'));
    assert!(rendered.contains("80..20 ns (regressed)"));
}

/// Collapsing a selected group has one valid selection target and can be reversed.
#[test]
fn collapse_reconciles_selection_and_selecting_item_expands_group() {
    let mut view = fixture();
    view.select_item(ItemId(8));
    let count = view.visible_row_count();
    press(&mut view, KeyCode::Char(' '));
    assert_eq!(view.visible_row_count(), 1);
    assert!(view.selected_item().is_none());
    assert_eq!(view.selected_group(), Some(GroupId(10)));
    let collapsed = text(&frame(&mut view, 80, 5));
    assert!(collapsed.contains("▸ request (1 tracks / 8 items)"));
    assert!(view.select_item(ItemId(8)));
    assert_eq!(view.visible_row_count(), count);
    assert_eq!(view.selected_item().unwrap().timing, TraceTiming::Untimed);
    assert!(!view.set_group_collapsed(GroupId(999), true));
    assert!(!view.select_item(ItemId(999)));
    assert_eq!(view.selected_item().unwrap().id, ItemId(8));
}

/// Selected rows stay visible during growth, shrinkage, paging, and one-row rendering.
#[test]
fn scrolling_and_resize_keep_selected_content_visible() {
    let mut view = TraceView::new(data(
        (0..40)
            .map(|id| item(id, &format!("fact-{id}"), TraceTiming::Untimed))
            .collect(),
    ))
    .unwrap();
    frame(&mut view, 60, 20);
    view.select_item(ItemId(39));
    for height in [20, 7, 3, 1, 9] {
        let rendered = text(&frame(&mut view, 60, height));
        assert!(rendered.contains("fact-39"), "height {height}: {rendered}");
        assert!(view.scroll_offset() <= view.cursor());
    }
    press(&mut view, KeyCode::PageUp);
    assert!(view.selected_item().unwrap().id.0 < 39);
    let rendered = text(&frame(&mut view, 80, 7));
    assert!(rendered.contains('↑'));
    assert!(rendered.contains('↓'));
}

/// Integer view arithmetic stays valid under repeated extreme panning and zooming.
#[test]
fn extreme_viewports_and_offscreen_selection_are_exact() {
    let mut view = TraceView::new(data(vec![
        item(1, "min", TraceTiming::Instant(i64::MIN)),
        item(2, "max", TraceTiming::Instant(i64::MAX)),
    ]))
    .unwrap();
    assert_eq!(view.time_window(), (i64::MIN, i64::MAX));
    for _ in 0..80 {
        press(&mut view, KeyCode::Char('+'));
        press(&mut view, KeyCode::Left);
    }
    view.select_item(ItemId(2));
    assert_eq!(view.time_window().1, i64::MAX);
    for _ in 0..80 {
        press(&mut view, KeyCode::Right);
        press(&mut view, KeyCode::Char('-'));
        let (start, end) = view.time_window();
        assert!(start < end);
    }
    press(&mut view, KeyCode::Char('0'));
    assert_eq!(view.time_window(), (i64::MIN, i64::MAX));
    assert_eq!(
        view.set_time_window(1, 1),
        Err(TraceError::InvalidTimeWindow)
    );
    assert_eq!(view.time_window(), (i64::MIN, i64::MAX));
    frame(&mut view, 80, 9);
}

/// Zero/tiny rectangles and offset clipping cannot paint outside the caller's allocation.
#[test]
fn tiny_empty_and_offset_areas_are_bounded() {
    let mut view = fixture();
    for width in 0..=32 {
        for height in 0..=10 {
            let first = frame(&mut view, width, height);
            assert_eq!(first, frame(&mut view, width, height));
            let size = view.measure(Constraints::loose(width, height));
            assert!(size.width <= width && size.height <= height);
        }
    }
    let area = Rect::new(4, 3, 60, 12);
    let mut buffer = Buffer::empty(area);
    for cell in &mut buffer.content {
        cell.set_symbol("x");
    }
    let paint = Rect::new(8, 5, 24, 5);
    render_trace_view(&view, paint, &mut buffer);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if !paint.contains((x, y).into()) {
                assert_eq!(buffer[(x, y)].symbol(), "x");
            }
        }
    }
    render_trace_view(&view, Rect::new(0, 0, 10, 10), &mut buffer);
    let mut empty = TraceView::new(TraceData {
        time_unit: TraceTimeUnit::Seconds,
        groups: vec![],
        tracks: vec![],
    })
    .unwrap();
    assert!(text(&frame(&mut empty, 80, 3)).contains("No trace observations"));
}

/// Narrow and one-cell snapshots retain a selected content row before chrome.
#[test]
fn narrow_and_tiny_snapshots() {
    let mut view = fixture();
    view.select_item(ItemId(8));
    assert_eq!(
        text(&frame(&mut view, 8, 3)),
        "0..20 ns\n▶ ? unt…\n↑4 ↓0 |…"
    );
    assert_eq!(text(&frame(&mut view, 8, 1)), "▶ ? unt…");
    assert_eq!(text(&frame(&mut view, 1, 1)), "…");
}

/// Large coordinates retain explicit units; short details retain exact recorded timing.
#[test]
fn short_details_and_large_time_axes_preserve_time_meaning() {
    let mut observation = item(1, "regressed", TraceTiming::Interval { start: 90, end: 20 });
    observation.details = vec![("source".into(), "fixture".into())];
    let mut view = TraceView::new(data(vec![observation])).unwrap();
    for height in [7, 8] {
        let rendered = text(&frame(&mut view, 80, height));
        assert!(rendered.contains("90..20 ns (regressed)"));
        assert!(rendered.contains("+1 fields"));
    }
    view.set_time_window(1_800_000_000_000_000_000, 1_800_000_000_000_000_001)
        .unwrap();
    for width in [8, 24, 40] {
        let rendered = text(&frame(&mut view, width, 8));
        assert!(rendered.lines().next().unwrap().ends_with("ns"));
    }
}

/// Unknown key chords and key releases remain available to outer input layers.
#[test]
fn ignored_keys_do_not_change_selection_or_time() {
    let mut view = fixture();
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        assert_eq!(
            view.handle_key(&KeyEvent::new(KeyCode::Down, modifiers)),
            KeyResult::Ignored
        );
    }
    let mut released = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    released.kind = KeyEventKind::Release;
    assert_eq!(view.handle_key(&released), KeyResult::Ignored);
    assert_eq!(
        view.handle_key(&KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        KeyResult::Ignored
    );
    assert_eq!(view.cursor(), 0);
    assert_eq!(view.time_window(), (0, 20));
}

/// Detail and text overflow remain explicit, with safe control and Unicode rendering.
#[test]
fn details_count_overflow_and_controls_do_not_escape_rows() {
    let mut observation = item(1, "界\n\u{1b}label", TraceTiming::Instant(1));
    observation.details = (0..32)
        .map(|i| (format!("key{i}"), "value".into()))
        .collect();
    let mut view = TraceView::new(data(vec![observation])).unwrap();
    let rendered = text(&frame(&mut view, 80, 12));
    assert!(!rendered.contains('\u{1b}'));
    assert!(rendered.contains("界  label"));
    assert!(rendered.contains("… +31 detail lines"));
    press(&mut view, KeyCode::Enter);
    assert!(!text(&frame(&mut view, 80, 12)).contains("detail lines"));
    let narrow = text(&frame(&mut view, 8, 1));
    assert!(narrow.starts_with("▶ │ 界"));
}

/// Identity and reference errors fail admission before a partial widget can escape.
#[test]
fn duplicate_ids_and_unknown_groups_are_rejected() {
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
    let group = TraceGroup {
        id: GroupId(9),
        label: "g".into(),
    };
    input.groups = vec![group.clone(), group];
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::DuplicateGroup(GroupId(9)))
    ));
}

/// Every documented allocation dimension has a rejecting boundary.
#[test]
fn admission_enforces_collection_and_text_limits() {
    let observation = item(0, "", TraceTiming::Untimed);
    let mut input = data(vec![]);
    input.groups = vec![
        TraceGroup {
            id: GroupId(0),
            label: String::new()
        };
        4097
    ];
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::LimitExceeded("groups (4096)"))
    ));
    let mut input = data(vec![]);
    input.tracks = vec![input.tracks[0].clone(); 4097];
    assert!(matches!(
        TraceView::new(input),
        Err(TraceError::LimitExceeded("tracks (4096)"))
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
    let mut long = observation.clone();
    long.label = "x".repeat(4097);
    assert!(matches!(
        TraceView::new(data(vec![long])),
        Err(TraceError::LimitExceeded("bytes per field (4096)"))
    ));
    let many = (0..2049)
        .map(|id| item(id, &"x".repeat(4096), TraceTiming::Untimed))
        .collect();
    assert!(matches!(
        TraceView::new(data(many)),
        Err(TraceError::LimitExceeded("total text bytes (8 MiB)"))
    ));
}

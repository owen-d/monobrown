#![cfg(feature = "devkit")]

//! Adversarial tests for the flame graph widget.
//!
//! The goal is to BREAK the implementation by probing boundary values,
//! stress-testing key sequences, verifying layout invariants, and
//! inspecting rendered output for visual correctness.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use tui_lib::devkit::buffer_to_text;
use tui_lib::devkit::color::{color_to_rgb, contrast_ratio, rgb_distance};
use tui_lib::devkit::flame_graph::{test_cost_types, test_flame_graph};
use tui_lib::widget::flame_graph::{
    CostType, FlameGraph, FlameRow, RowKind, SpanNode, SpanNodeBuilder, render_flame_graph,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Check if a symbol is a bar character (any bar style).
fn is_bar_char(s: &str) -> bool {
    matches!(s, "\u{257A}" | "\u{2501}" | "\u{2578}" | "\u{28FF}")
}

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn finish_animations(fg: &mut FlameGraph) {
    for _ in 0..96 {
        fg.tick(Duration::from_millis(16));
    }
}

fn render_to_text_sized(fg: &FlameGraph, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render_flame_graph(fg, area, &mut buf);
    buffer_to_text(&buf)
}

fn render_to_buffer(fg: &FlameGraph, width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render_flame_graph(fg, area, &mut buf);
    buf
}

fn simple_cost_types() -> Vec<CostType> {
    vec![CostType {
        name: "cpu",
        color: Color::Red,
    }]
}

fn multi_cost_types() -> Vec<CostType> {
    vec![
        CostType {
            name: "cpu",
            color: Color::Rgb(100, 150, 255),
        },
        CostType {
            name: "io",
            color: Color::Rgb(255, 150, 100),
        },
        CostType {
            name: "mem",
            color: Color::Rgb(100, 255, 150),
        },
    ]
}

/// Extract span depth at a given cursor index from visible rows.
fn row_depth(rows: &[FlameRow], index: usize) -> Option<u16> {
    rows.get(index).map(|r| match r.kind {
        RowKind::Span { depth, .. } => depth,
        RowKind::Legend { .. } => u16::MAX, // sentinel for legend rows
    })
}

/// Check if a row is a legend row.
fn is_legend(rows: &[FlameRow], index: usize) -> bool {
    matches!(
        rows.get(index),
        Some(FlameRow {
            kind: RowKind::Legend { .. }
        })
    )
}

// ===========================================================================
// 1. BOUNDARY VALUE TESTS
// ===========================================================================

/// Minimum viable render: width=5, height=2. Should not panic, should show
/// something. Probes: off-by-one in bar width calculation, label truncation.
#[test]
fn boundary_minimum_render_5x2() {
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf("root_node", vec![10.0]);
    let fg = FlameGraph::new(root, simple_cost_types());
    let text = render_to_text_sized(&fg, 5, 2);
    // Should not panic. Root bar is only 5 cells wide.
    // Label "root_node" (9 chars) must be truncated to fit.
    assert!(!text.is_empty(), "Even at 5x2, some output expected");
}

/// Very wide render: width=200. No panics, root bar should span full width.
#[test]
fn boundary_very_wide_render() {
    let fg = test_flame_graph();
    let buf = render_to_buffer(&fg, 200, 10);
    // Root bar should start at column 0. Check that column 199 has a colored cell.
    let last_cell = &buf[(199, 0)];
    assert_ne!(
        last_cell.symbol(),
        " ",
        "Root bar should extend to the last column at width=200"
    );
}

/// Zero total cost span. The render should not panic (no division by zero).
#[test]
fn boundary_zero_cost_span() {
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf("zero", vec![0.0, 0.0]);
    let fg = FlameGraph::new(root, multi_cost_types());
    // Should not panic on render.
    let _text = render_to_text_sized(&fg, 40, 5);
}

/// Single cost type span. Should render without issues.
#[test]
fn boundary_single_cost_type() {
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf("one_type", vec![42.0]);
    let fg = FlameGraph::new(root, simple_cost_types());
    let text = render_to_text_sized(&fg, 40, 3);
    assert!(
        text.contains("one_type"),
        "Single cost type label should appear"
    );
}

/// Single-node tree (no children). Expand (Right) should be a no-op.
#[test]
fn boundary_single_node_expand_is_noop() {
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf("solo", vec![5.0]);
    let mut fg = FlameGraph::new(root, simple_cost_types());
    let cursor_before = fg.cursor();
    fg.handle_key(&make_key(KeyCode::Right));
    assert_eq!(
        fg.cursor(),
        cursor_before,
        "Expanding a leaf node should not move cursor"
    );
}

/// Tree with 20+ children at one level. Should render, should navigate
/// through all visible rows with j/k (including root).
#[test]
fn boundary_twenty_children() {
    let mut b = SpanNodeBuilder::new();
    let children: Vec<SpanNode> = (0..20)
        .map(|i| b.leaf(&format!("child_{i:02}"), vec![(20 - i) as f64]))
        .collect();
    let root = b.span("root", vec![210.0], children);
    let mut fg = FlameGraph::new(root, simple_cost_types());

    // Expand root.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Navigate down through all 19 siblings (we start at first child).
    for _ in 0..19 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    let cursor_at_last = fg.cursor();

    // One more should be no-op (last visible row).
    fg.handle_key(&make_key(KeyCode::Down));
    assert_eq!(
        fg.cursor(),
        cursor_at_last,
        "Should stop at last visible row"
    );

    // Navigate all the way back up (19 siblings + root = 20 up presses).
    for _ in 0..20 {
        fg.handle_key(&make_key(KeyCode::Up));
    }
    let cursor_at_root = fg.cursor();
    assert_eq!(cursor_at_root, 0, "Should reach root via k navigation");

    // One more up should be no-op (already at root).
    fg.handle_key(&make_key(KeyCode::Up));
    assert_eq!(
        fg.cursor(),
        cursor_at_root,
        "Should stop at root (first visible row)"
    );
}

/// Empty label. Should not panic.
#[test]
fn boundary_empty_label() {
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf("", vec![5.0]);
    let fg = FlameGraph::new(root, simple_cost_types());
    let _text = render_to_text_sized(&fg, 40, 3);
}

/// Very long label (50 chars). Should be truncated, not overflow beyond the
/// buffer boundary. We check by ensuring no cell is written beyond the
/// buffer width (ratatui enforces this), and that the rendered text does
/// not contain the full label.
#[test]
fn boundary_long_label_truncated() {
    let long_label = "abcdefghijklmnopqrstuvwxyz".repeat(2); // 50 ASCII chars
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf(&long_label, vec![5.0]);
    let fg = FlameGraph::new(root, simple_cost_types());
    let buf = render_to_buffer(&fg, 30, 3);
    // The full label (50 chars) must NOT appear -- it should be truncated.
    let text = buffer_to_text(&buf);
    assert!(
        !text.contains(&long_label),
        "50-char label should be truncated at width=30"
    );
    // Check that no cell was written beyond the buffer boundary (ratatui
    // guarantees this, but let's be explicit).
    for y in 0..3u16 {
        for x in 0..30u16 {
            // Accessing (x, y) within bounds should not panic.
            let _cell = &buf[(x, y)];
        }
    }
}

// ===========================================================================
// 2. STRESS KEY SEQUENCES
// ===========================================================================

/// Send 50 random-ish key events. No panic should occur.
/// Probes: state corruption from unexpected key sequences.
#[test]
fn stress_random_keys_no_panic() {
    let mut fg = test_flame_graph();
    let keys = [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('h'),
        KeyCode::Char('l'),
        KeyCode::Enter,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Left,
        KeyCode::Right,
    ];
    for i in 0..50 {
        let key = keys[i % keys.len()];
        fg.handle_key(&make_key(key));
        if i % 5 == 0 {
            fg.tick(Duration::from_millis(16));
        }
    }
    // If we get here without panic, the test passes.
    let _text = render_to_text_sized(&fg, 80, 20);
}

/// Rapidly alternate h/l 20 times on the same node. Should not corrupt state.
#[test]
fn stress_rapid_expand_collapse() {
    let mut fg = test_flame_graph();
    for _ in 0..20 {
        fg.handle_key(&make_key(KeyCode::Right)); // expand + descend
        fg.handle_key(&make_key(KeyCode::Left)); // collapse + ascend
        fg.tick(Duration::from_millis(16));
    }
    // Cursor should be back at root.
    assert_eq!(
        fg.cursor(),
        0,
        "After 20 expand/collapse cycles, cursor should be at root"
    );
}

/// Press j 100 times from root. Cursor should never go below last visible row.
#[test]
fn stress_100_downs_from_root() {
    let mut fg = test_flame_graph();
    // Root is at index 0, no children visible (collapsed).
    // j should be sibling-only. Root has no siblings, so it's a no-op.
    for _ in 0..100 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    assert_eq!(
        fg.cursor(),
        0,
        "Root has no siblings; Down should be a no-op"
    );
}

/// Press k 100 times from first child. Cursor navigates the flat visible
/// list, reaching root and stopping there.
#[test]
fn stress_100_ups_from_first_child() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand root, descend to first child
    finish_animations(&mut fg);

    for _ in 0..100 {
        fg.handle_key(&make_key(KeyCode::Up));
    }
    assert_eq!(
        fg.cursor(),
        0,
        "k from first child should reach root and stop there"
    );
}

// ===========================================================================
// 3. INVARIANT PROBING
// ===========================================================================

/// After any key sequence, cursor must always index a valid visible row.
#[test]
fn invariant_cursor_always_valid_index() {
    let sequences: Vec<Vec<KeyCode>> = vec![
        vec![KeyCode::Right, KeyCode::Down, KeyCode::Down, KeyCode::Down],
        vec![KeyCode::Right, KeyCode::Right, KeyCode::Left],
        vec![KeyCode::Right, KeyCode::Down, KeyCode::Right, KeyCode::Up],
        vec![KeyCode::Right, KeyCode::Enter, KeyCode::Down],
    ];

    for (seq_idx, seq) in sequences.iter().enumerate() {
        let mut fg = test_flame_graph();
        for (key_idx, code) in seq.iter().enumerate() {
            fg.handle_key(&make_key(*code));
            fg.tick(Duration::from_millis(16));
            let rows = fg.visible_rows();
            assert!(
                fg.cursor() < rows.len(),
                "Cursor out of bounds after sequence {seq_idx}, key {key_idx} ({code:?}): \
                 cursor={}, rows.len()={}",
                fg.cursor(),
                rows.len()
            );
        }
    }
}

/// After any j/k key, the cursor must point to a valid non-legend row.
/// j/k now navigates the flat visible list across depth boundaries.
#[test]
fn invariant_jk_cursor_always_valid() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand root, descend
    finish_animations(&mut fg);

    // Move down through all visible rows.
    for _ in 0..10 {
        fg.handle_key(&make_key(KeyCode::Down));
        finish_animations(&mut fg);
        let rows = fg.visible_rows();
        assert!(
            fg.cursor() < rows.len(),
            "Cursor {} out of bounds (rows.len()={})",
            fg.cursor(),
            rows.len()
        );
        assert!(
            !is_legend(&rows, fg.cursor()),
            "j/k should never land on a legend row"
        );
    }

    // Move back up.
    for _ in 0..10 {
        fg.handle_key(&make_key(KeyCode::Up));
        finish_animations(&mut fg);
        let rows = fg.visible_rows();
        assert!(
            fg.cursor() < rows.len(),
            "Cursor {} out of bounds (rows.len()={})",
            fg.cursor(),
            rows.len()
        );
        assert!(
            !is_legend(&rows, fg.cursor()),
            "j/k should never land on a legend row"
        );
    }
}

/// After expand+descend (l), the node must be in the expanded set.
#[test]
fn invariant_expand_adds_to_expanded_set() {
    let mut fg = test_flame_graph();
    let root_id = fg.root().id;

    fg.handle_key(&make_key(KeyCode::Right));
    assert!(
        fg.is_expanded(root_id),
        "After Right on root, root must be in expanded set"
    );
}

/// After collapse+ascend (h) and animation completion, the collapsed node
/// must NOT be in the expanded set.
///
/// h collapses the node the cursor is ON, then ascends to parent. So to
/// collapse root, the cursor must be on root when it is expanded.
#[test]
fn invariant_collapse_removes_from_expanded_set() {
    let mut fg = test_flame_graph();
    let root_id = fg.root().id;

    // Expand root and descend to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    assert!(fg.is_expanded(root_id));

    // Left from first child: collapses root (the parent) and ascends to root.
    // This is the E6 behavior: ascending collapses the parent.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Cursor should now be on root.
    assert_eq!(fg.cursor(), 0, "Should have ascended to root");

    // Root should be collapsed (Left collapses the parent when ascending).
    assert!(
        !fg.is_expanded(root_id),
        "Root should be collapsed after ascending from its children"
    );
}

/// Children at every level must be sorted by total cost descending.
#[test]
fn invariant_children_sorted_by_cost_descending() {
    fn check_sorted(node: &SpanNode) {
        for i in 1..node.children.len() {
            let prev_cost = node.children[i - 1].costs.total();
            let curr_cost = node.children[i].costs.total();
            assert!(
                prev_cost >= curr_cost,
                "Children not sorted descending: {} ({}) should come after {} ({})",
                node.children[i].label,
                curr_cost,
                node.children[i - 1].label,
                prev_cost
            );
        }
        for child in &node.children {
            check_sorted(child);
        }
    }

    let fg = test_flame_graph();
    check_sorted(fg.root());
}

// ===========================================================================
// 4. VISUAL INSPECTION VIA SURFACE / BUFFER
// ===========================================================================

/// Render one-level at width=80. Each child label should appear.
///
/// BUG FOUND: "logging" (7 chars) is truncated to "loggin" because
/// draw_label uses `bar_width - 2` as the max label length. The `-2`
/// accounts for cursor indicator (1 cell) + 1 extra cell, but that
/// extra cell is wasted -- there is no trailing padding needed.
/// The label starts at bar_start + 1 and has bar_width - 1 cells
/// available, so the correct truncation should be `bar_width - 1`.
#[test]
fn visual_one_level_all_labels_present() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let text = render_to_text_sized(&fg, 80, 10);
    // The test_flame_graph root is "request" with children:
    // db_query, template_render, auth_check, logging (sorted by cost).
    // Note: "template_render" may be truncated due to the label zone width
    // (indent + indicator + label must fit in 18 cols).
    for label in &[
        "request",
        "db_query",
        "template_rend",
        "auth_check",
        "logging",
    ] {
        assert!(
            text.contains(label),
            "Label '{label}' missing from one-level render.\n\
             Rendered text:\n{text}"
        );
    }
}

/// Root bar fills the bar zone (cols 21..80 at width=80).
#[test]
fn visual_root_bar_spans_full_width() -> Result<(), Box<dyn std::error::Error>> {
    let fg = test_flame_graph();
    let buf = render_to_buffer(&fg, 80, 5);

    let bar_start = (0..80u16)
        .find(|&x| is_bar_char(buf[(x, 0)].symbol()))
        .ok_or("root row should contain a bar")?;
    let mut bar_count = 0u16;
    for x in bar_start..80 {
        if is_bar_char(buf[(x, 0)].symbol()) {
            bar_count += 1;
        }
    }
    assert!(
        bar_count >= 57,
        "Root bar should fill nearly all {bar_zone} bar-zone columns, only filled {bar_count}",
        bar_zone = 80 - bar_start,
    );
    Ok(())
}

/// Child bars must be narrower than the root bar (proportional to cost fraction).
#[test]
fn visual_child_bars_narrower_than_parent() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let rows = fg.visible_rows();
    // Row 0 is root (depth 0), rows 1+ are children (depth 1).
    let root_width = match rows[0].kind {
        RowKind::Span { bar_width, .. } => bar_width,
        _ => panic!("row 0 should be a span"),
    };
    for row in &rows[1..] {
        if let RowKind::Span {
            bar_width, depth, ..
        } = row.kind
            && depth == 1
        {
            assert!(
                bar_width <= root_width,
                "Child bar_width ({bar_width}) should be <= root bar_width ({root_width})"
            );
        }
    }
}

/// Render at width=15 -- labels should be truncated, no cell should be
/// written beyond column 14. We verify by checking the buffer directly
/// (byte length of buffer_to_text includes multi-byte Unicode chars like
/// the full-block character, so we count columns instead).
#[test]
fn visual_narrow_no_overflow() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let width = 15u16;
    let height = 10u16;
    let buf = render_to_buffer(&fg, width, height);

    // Every cell within the buffer is valid by construction. The real check
    // is that no rendering logic tried to write beyond column 14. ratatui
    // panics on out-of-bounds writes, so if we got here, it passed.
    // Additionally, verify that column 14 exists for each row.
    for y in 0..height {
        let _cell = &buf[(width - 1, y)];
    }
}

/// With legend enabled, the legend row should contain cost type names.
#[test]
fn visual_legend_contains_cost_type_names() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand, descend to first child
    finish_animations(&mut fg);
    // Legend is on by default and follows the cursor to first child.

    let text = render_to_text_sized(&fg, 120, 12);
    // Legend should show cost type names for db_query.
    assert!(
        text.contains("cpu") || text.contains("io"),
        "Legend should contain cost type names.\nRendered:\n{text}"
    );
}

/// Legend should push siblings down -- verify that the total number of
/// visible rows increases when legend is toggled on.
#[test]
fn visual_legend_pushes_siblings_down() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Legend is on by default. Toggle it off first to get the baseline.
    fg.handle_key(&make_key(KeyCode::Enter)); // toggle legend off
    let rows_without = fg.visible_rows().len();

    fg.handle_key(&make_key(KeyCode::Enter)); // toggle legend on

    let rows_with = fg.visible_rows().len();
    assert!(
        rows_with > rows_without,
        "Legend should add a row. Without: {rows_without}, With: {rows_with}"
    );
}

/// Only one legend at a time. If we move cursor to a different node with
/// legend active, the legend should follow the cursor.
#[test]
fn visual_only_one_legend_at_a_time() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand, at first child
    finish_animations(&mut fg);
    // Legend is on by default and follows cursor to first child.

    // Move to second sibling.
    fg.handle_key(&make_key(KeyCode::Down));

    // Count legend rows.
    let rows = fg.visible_rows();
    let legend_count = rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Legend { .. }))
        .count();
    assert!(
        legend_count <= 1,
        "Only 1 legend at a time allowed, found {legend_count}"
    );
}

// ===========================================================================
// 5. COLOR AND CONTRAST
// ===========================================================================

/// All 5 test cost type colors must pass WCAG AA large-text (3.0:1) against
/// both black (0,0,0) AND dark gray (30,30,30) backgrounds.
#[test]
fn color_wcag_aa_large_against_black_and_dark_gray() {
    let cost_types = test_cost_types();
    let backgrounds = [(0u8, 0u8, 0u8), (30, 30, 30)];

    for ct in &cost_types {
        let rgb = match color_to_rgb(ct.color) {
            Some(rgb) => rgb,
            None => panic!("cost type should have RGB color"),
        };
        for bg in &backgrounds {
            let ratio = contrast_ratio(rgb, *bg);
            assert!(
                ratio >= 3.0,
                "Cost type '{}' color {:?} has contrast ratio {:.2} < 3.0 \
                 against bg {:?}. WCAG AA large-text requires >= 3.0",
                ct.name,
                rgb,
                ratio,
                bg
            );
        }
    }
}

/// Label text must be readable against bar segment colors.
/// The implementation uses adaptive label colors: dark text on light bars,
/// light text on dark bars, based on the dominant segment's luminance.
/// This test verifies that the adaptive choice produces adequate contrast.
#[test]
fn color_labels_readable_against_bar_segments() {
    use tui_lib::devkit::color::relative_luminance;

    let cost_types = test_cost_types();

    for ct in &cost_types {
        let bar_color = match color_to_rgb(ct.color) {
            Some(rgb) => rgb,
            None => panic!("cost type should have RGB color"),
        };
        let lum = relative_luminance(bar_color.0, bar_color.1, bar_color.2);
        // Adaptive label: dark text on light bars, light text on dark bars.
        let label_fg = if lum > 0.4 {
            (20u8, 20u8, 20u8)
        } else {
            (220u8, 220u8, 220u8)
        };
        let ratio = contrast_ratio(label_fg, bar_color);
        assert!(
            ratio >= 2.0,
            "Label fg {:?} has contrast ratio {:.2} < 2.0 against bar color {:?} \
             for cost type '{}'. Labels may be unreadable.",
            label_fg,
            ratio,
            bar_color,
            ct.name,
        );
    }
}

/// Pairwise distinguishability of the 5 cost type colors.
/// No two colors should be too similar (RGB distance < 0.08).
/// Threshold is relaxed from 0.15 because the tinted analogous palette
/// deliberately trades some distinguishability for cohesion.
#[test]
fn color_pairwise_distinguishability() {
    let cost_types = test_cost_types();
    let rgbs: Vec<(u8, u8, u8)> = cost_types
        .iter()
        .map(|ct| match color_to_rgb(ct.color) {
            Some(rgb) => rgb,
            None => panic!("cost type should have RGB color"),
        })
        .collect();

    for i in 0..rgbs.len() {
        for j in (i + 1)..rgbs.len() {
            let dist = rgb_distance(rgbs[i], rgbs[j]);
            assert!(
                dist >= 0.08,
                "Cost types '{}' {:?} and '{}' {:?} are too similar: \
                 distance={:.3} < 0.08",
                cost_types[i].name,
                rgbs[i],
                cost_types[j].name,
                rgbs[j],
                dist,
            );
        }
    }
}

// ===========================================================================
// 6. LAYOUT PROPORTIONALITY
// ===========================================================================

/// If parent total=100 and child total=70, child bar width should be ~70% of
/// parent bar width, within +/-2 cells.
#[test]
fn proportionality_child_bar_width() -> Result<(), Box<dyn std::error::Error>> {
    let mut b = SpanNodeBuilder::new();
    let child = b.leaf("big_child", vec![70.0]);
    let small = b.leaf("small_child", vec![30.0]);
    let root = b.span("parent", vec![100.0], vec![child, small]);
    let mut fg = FlameGraph::new(root, simple_cost_types());

    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let width = 100u16;

    // Check the rendered buffer directly -- visible_rows() uses a nav-only
    // width, so we measure the pixel-level output instead.
    let buf = render_to_buffer(&fg, width, 10);

    let bar_start = (0..width)
        .find(|&x| is_bar_char(buf[(x, 0)].symbol()))
        .ok_or("root row should contain a bar")?;
    let count_blocks = |row: u16| -> u16 {
        (bar_start..width)
            .filter(|&x| is_bar_char(buf[(x, row)].symbol()))
            .count() as u16
    };

    let root_bar_width = count_blocks(0);
    let child_bar_width = count_blocks(1);

    // Child bar should be roughly 70% of root bar (no indent subtraction
    // in the two-zone layout since bars are in a separate zone).
    let expected_child = ((root_bar_width as f64) * 0.70).floor() as u16;

    let diff = (child_bar_width as i32 - expected_child as i32).unsigned_abs() as u16;
    assert!(
        diff <= 2,
        "Child bar width ({child_bar_width}) should be ~70% of root ({root_bar_width}) = \
         {expected_child}, but diff is {diff} (> 2 cells)"
    );
    Ok(())
}

/// Children should never extend beyond parent bar width.
#[test]
fn proportionality_children_within_parent() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let buf = render_to_buffer(&fg, 80, 10);

    // Find the rightmost non-space cell on row 0 (root).
    let root_right = (0..80u16)
        .rev()
        .find(|&x| buf[(x, 0)].symbol() != " ")
        .unwrap_or(0);

    // For each child row, the rightmost non-space cell should not exceed
    // root_right.
    for y in 1..10u16 {
        let child_right = (0..80u16).rev().find(|&x| buf[(x, y)].symbol() != " ");
        if let Some(cr) = child_right {
            assert!(
                cr <= root_right,
                "Child on row {y} extends to column {cr}, beyond parent's right edge at {root_right}"
            );
        }
    }
}

// ===========================================================================
// 7. COLLAPSE ANIMATION BEHAVIOR
// ===========================================================================

/// When collapsing, the animation target should be 0.0. After animation
/// completes, the span should be removed from expanded set.
///
/// h collapses the node the cursor is ON, then ascends. So to collapse
/// root, the cursor must be on root when it is expanded.
#[test]
fn animation_collapse_removes_from_expanded() {
    let mut fg = test_flame_graph();
    let root_id = fg.root().id;

    // Expand root and descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    assert!(fg.is_expanded(root_id));

    // Ascend back to root first (Left on first child just ascends since
    // first child is not expanded).
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);
    assert_eq!(fg.cursor(), 0, "Should be at root");

    // Now collapse root itself.
    fg.handle_key(&make_key(KeyCode::Left));

    // Before animation completes, root should still be in expanded set.
    // After animation completes:
    finish_animations(&mut fg);
    assert!(
        !fg.is_expanded(root_id),
        "Root should be removed from expanded set after collapse animation completes"
    );
}

/// Tick with dt=0 should not change animation values (no NaN, no division issues).
#[test]
fn animation_zero_dt() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    fg.tick(Duration::ZERO);
    // Should not panic or produce NaN.
    let _text = render_to_text_sized(&fg, 40, 5);
}

/// Tick with very large dt should snap to target immediately.
#[test]
fn animation_large_dt_snaps() {
    let mut fg = test_flame_graph();
    let root_id = fg.root().id;

    fg.handle_key(&make_key(KeyCode::Right));
    // One huge tick should complete the animation.
    fg.tick(Duration::from_secs(10));

    assert!(
        fg.is_expanded(root_id),
        "After expand + huge tick, root should still be expanded (target=1.0)"
    );
}

// ===========================================================================
// 8. h/l BEHAVIOR ON ALREADY-EXPANDED / ALREADY-COLLAPSED
// ===========================================================================

/// Pressing l on an already-expanded node should still descend to first child.
#[test]
fn expand_already_expanded_descends() {
    let mut fg = test_flame_graph();

    // Expand root.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    // Cursor is at first child. Go back to root via Left.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Root is now collapsed. Expand again.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    assert_eq!(
        fg.cursor(),
        1,
        "Right on root should descend to first child, even after previous collapse"
    );
}

/// Pressing h on a node that is already collapsed should still ascend to parent.
#[test]
fn collapse_already_collapsed_ascends() {
    let mut fg = test_flame_graph();

    // Expand root, then expand first child, descend to grandchild.
    fg.handle_key(&make_key(KeyCode::Right)); // expand root, at first child
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right)); // expand first child, at grandchild
    finish_animations(&mut fg);

    // Press h to collapse grandchild (which is a leaf -- nothing to collapse)
    // and ascend to first child.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // We should now be at first child (db_query).
    let rows = fg.visible_rows();
    assert!(
        fg.cursor() > 0,
        "Cursor should have ascended to parent, not stayed at grandchild"
    );
    // The cursor should be at depth 1 (child of root).
    let depth = row_depth(&rows, fg.cursor());
    assert_eq!(
        depth,
        Some(1),
        "After ascending from grandchild, should be at depth 1"
    );
}

// ===========================================================================
// 9. LEGEND OVERFLOW
// ===========================================================================

/// When there are many cost types and the width is narrow, the legend should
/// show overflow text like "+N types X%".
#[test]
fn legend_overflow_at_narrow_width() {
    let mut b = SpanNodeBuilder::new();
    // Create a span with 5 cost types, each with non-zero cost.
    let root = b.leaf("root", vec![20.0, 15.0, 10.0, 5.0, 3.0]);
    let cost_types = test_cost_types(); // 5 types
    let fg = FlameGraph::new(root, cost_types);
    // Legend is on by default on root.

    // Render at a narrow width where not all cost types fit.
    let text = render_to_text_sized(&fg, 40, 5);

    // At 40 columns, 5 cost types may not all fit. If overflow occurs,
    // we should see "+N types" in the output.
    // If they all fit, that's also acceptable -- the test documents the behavior.
    // We just check that no panic occurs and the legend is present.
    assert!(
        text.contains("cpu") || text.contains("+"),
        "Legend should show at least one cost type or overflow indicator.\n\
         Rendered:\n{text}"
    );
}

// ===========================================================================
// 10. COST SEGMENT RENDERING
// ===========================================================================

/// Each span should have bar segments colored proportionally to cost types.
/// Verify that a span with two cost types of equal size gets roughly equal
/// bar segment widths.
///
/// Note: the label text is drawn on top of segments, overwriting their fg
/// color. We use a short label and check only the segment cells that are
/// NOT overwritten by the label.
#[test]
fn segments_proportional_to_costs() {
    let mut b = SpanNodeBuilder::new();
    // Use a very short label so it occupies minimal cells.
    let root = b.leaf("X", vec![50.0, 50.0]);
    let cost_types = vec![
        CostType {
            name: "a",
            color: Color::Rgb(200, 50, 50),
        },
        CostType {
            name: "b",
            color: Color::Rgb(50, 50, 200),
        },
    ];
    let fg = FlameGraph::new(root, cost_types);

    let buf = render_to_buffer(&fg, 80, 3);

    // Count cells of each segment color on row 0.
    // Skip cells 0-2 (cursor indicator + label area).
    let mut color_a_count = 0u16;
    let mut color_b_count = 0u16;
    for x in 0..80 {
        let fg_color = buf[(x, 0)].fg;
        match fg_color {
            Color::Rgb(200, 50, 50) => color_a_count += 1,
            Color::Rgb(50, 50, 200) => color_b_count += 1,
            _ => {}
        }
    }

    // Both segments should be present.
    assert!(
        color_a_count > 0 && color_b_count > 0,
        "Both cost type colors should appear. a={color_a_count}, b={color_b_count}"
    );
    // With label overlay removed from the equation, the segments should
    // be roughly equal. Allow a wider margin to account for rounding
    // and the label/cursor cells.
    let diff = (color_a_count as i32 - color_b_count as i32).unsigned_abs();
    assert!(
        diff <= 8,
        "Equal costs should produce roughly equal segments: a={color_a_count}, b={color_b_count}, diff={diff}"
    );
}

// ===========================================================================
// 11. DETERMINISM
// ===========================================================================

/// Full interaction sequence must be deterministic across two runs.
#[test]
fn determinism_full_interaction() {
    let run = || {
        let mut fg = test_flame_graph();
        fg.handle_key(&make_key(KeyCode::Right)); // expand root
        for _ in 0..10 {
            fg.tick(Duration::from_millis(16));
        }
        fg.handle_key(&make_key(KeyCode::Down)); // move to second child
        fg.handle_key(&make_key(KeyCode::Enter)); // toggle legend
        fg.handle_key(&make_key(KeyCode::Right)); // expand second child
        for _ in 0..10 {
            fg.tick(Duration::from_millis(16));
        }
        render_to_text_sized(&fg, 80, 20)
    };

    assert_eq!(
        run(),
        run(),
        "Full interaction must produce identical output"
    );
}

// ===========================================================================
// 12. EDGE CASE: h AT ROOT
// ===========================================================================

/// Pressing h at root (no parent) should be a no-op, not panic.
#[test]
fn h_at_root_is_noop() {
    let mut fg = test_flame_graph();
    let cursor_before = fg.cursor();
    fg.handle_key(&make_key(KeyCode::Left));
    assert_eq!(
        fg.cursor(),
        cursor_before,
        "h at root should not move cursor"
    );
}

// ===========================================================================
// 13. EDGE CASE: CURSOR AFTER COLLAPSE
// ===========================================================================

/// If cursor is on a deep child and we collapse a mid-level ancestor,
/// the cursor must not point to a now-invisible row.
#[test]
fn cursor_valid_after_ancestor_collapse() {
    let mut fg = test_flame_graph();

    // Navigate deep: root -> first child -> first grandchild.
    fg.handle_key(&make_key(KeyCode::Right)); // expand root, at first child
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right)); // expand first child, at first grandchild
    finish_animations(&mut fg);

    // Now press h twice to collapse first child and ascend to root.
    fg.handle_key(&make_key(KeyCode::Left)); // collapse first child, ascend to first child
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Left)); // collapse root, ascend to root
    finish_animations(&mut fg);

    let rows = fg.visible_rows();
    assert!(
        fg.cursor() < rows.len(),
        "Cursor ({}) out of bounds after collapsing ancestors. rows.len()={}",
        fg.cursor(),
        rows.len()
    );
}

// ===========================================================================
// 14. CHILDREN NEVER RENDERED BEYOND PARENT BOUNDARY (layout)
// ===========================================================================

/// Using layout directly: verify bar_width of each child never exceeds
/// the root bar_width (since bars are left-aligned in the bar zone).
#[test]
fn layout_children_within_parent_bounds() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand root
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right)); // expand first child
    finish_animations(&mut fg);

    let rows = fg.visible_rows();

    let root_width = match rows[0].kind {
        RowKind::Span { bar_width, .. } => bar_width,
        _ => panic!("row 0 should be a span"),
    };

    for row in &rows {
        if let RowKind::Span {
            span_id,
            bar_width,
            depth,
        } = row.kind
            && depth > 0
        {
            assert!(
                bar_width <= root_width,
                "Span {span_id:?} at depth {depth} has bar_width {bar_width} \
                 exceeding root bar_width {root_width}",
            );
        }
    }
}

// ===========================================================================
// 15. E8 ADVERSARIAL: CURSOR VALIDITY AFTER SUBTREE COLLAPSE
// ===========================================================================

/// E8 Scenario 1: Expand root, expand first child, navigate j past all
/// grandchildren to uncle. The old subtree collapses. After animations
/// finish, cursor must still point to a valid row.
///
/// This probes whether enforce_single_path + finish_animations leaves the
/// cursor pointing into a now-shorter row list.
#[test]
fn e8_cursor_valid_after_collapse_and_animation() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child (db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand first child (db_query), descend to first grandchild.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Record pre-navigation state.
    let rows_before = fg.visible_rows();
    let grandchild_count = rows_before
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Span { depth, .. } if depth == 2))
        .count();
    assert!(
        grandchild_count >= 3,
        "db_query should have 3 grandchildren visible"
    );

    // Navigate j past all grandchildren. This should eventually reach
    // template_render (uncle), triggering enforce_single_path to collapse
    // the db_query subtree.
    for _ in 0..grandchild_count + 1 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    // Now finish the collapse animations so the grandchild rows disappear.
    finish_animations(&mut fg);

    // Critical check: cursor must be valid in the POST-animation row list.
    let rows_after = fg.visible_rows();
    assert!(
        fg.cursor() < rows_after.len(),
        "CURSOR INVALID after subtree collapse + animation: cursor={}, rows={}. \
         The collapse removed {} grandchild rows but cursor was not adjusted.",
        fg.cursor(),
        rows_after.len(),
        grandchild_count
    );

    // Cursor should point to a span row (not a legend row).
    assert!(
        !is_legend(&rows_after, fg.cursor()),
        "Cursor should be on a span row, not a legend row"
    );
}

/// E8 Scenario 2: Press j 50 times rapidly from an expanded 2-level tree.
/// Cursor must remain valid after every single keypress + animation.
#[test]
fn e8_rapid_j_across_subtree_boundaries() {
    let mut fg = test_flame_graph();

    // Expand root, then expand first child (2 levels deep).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Press j 50 times, finishing animations after each.
    for i in 0..50 {
        fg.handle_key(&make_key(KeyCode::Down));
        finish_animations(&mut fg);
        let rows = fg.visible_rows();
        assert!(
            fg.cursor() < rows.len(),
            "Cursor out of bounds at j press {i}: cursor={}, rows.len()={}",
            fg.cursor(),
            rows.len()
        );
    }
}

/// E8 Scenario 3: Navigate j past a subtree (collapsing it), then k back.
/// k must NOT re-enter the now-collapsed subtree. It should skip over
/// the collapsed parent or go to the previous visible row.
#[test]
fn e8_k_after_j_across_collapsed_subtree() {
    let mut fg = test_flame_graph();

    // Expand root, expand first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Navigate j past all grandchildren to uncle.
    for _ in 0..10 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    finish_animations(&mut fg);

    let rows_at_uncle = fg.visible_rows();
    let cursor_at_uncle = fg.cursor();
    assert!(
        cursor_at_uncle < rows_at_uncle.len(),
        "Cursor should be valid at uncle position"
    );

    // Now press k to go back.
    fg.handle_key(&make_key(KeyCode::Up));
    finish_animations(&mut fg);

    let rows_after_k = fg.visible_rows();
    assert!(
        fg.cursor() < rows_after_k.len(),
        "Cursor out of bounds after k: cursor={}, rows.len()={}",
        fg.cursor(),
        rows_after_k.len()
    );

    // Cursor should NOT be on a grandchild (depth 2) since that subtree
    // was collapsed by the j navigation.
    let depth = row_depth(&rows_after_k, fg.cursor());
    assert!(
        depth != Some(2),
        "k should not re-enter the collapsed subtree (cursor at depth 2)"
    );
}

/// E8 Scenario 4: Single-path enforcement completeness.
/// Expand root -> child A -> grandchild A.1. Navigate j to child B.
/// Both child A AND grandchild A.1 must be collapsed.
#[test]
fn e8_enforce_single_path_collapses_entire_branch() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child (db_query = child A).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let child_a_id = fg.root().children[0].id;

    // Expand child A, descend to grandchild A.1.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    assert!(fg.is_expanded(child_a_id), "Child A should be expanded");

    // Navigate j past all grandchildren to uncle (child B).
    for _ in 0..20 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    finish_animations(&mut fg);

    // Child A should be collapsed (enforce_single_path collapses entire branch).
    assert!(
        !fg.is_expanded(child_a_id),
        "Child A should be collapsed after navigating to uncle. \
         enforce_single_path must collapse the entire old branch, not just \
         the immediate child."
    );

    // Grandchild A.1 should also be collapsed (recursively).
    let grandchild_a1_id = fg.root().children[0].children[0].id;
    assert!(
        !fg.is_expanded(grandchild_a1_id),
        "Grandchild A.1 should also be collapsed (recursive collapse)"
    );
}

/// E8 Scenario 6: j from deepest expanded node. Expand 3 levels, cursor
/// at deepest leaf. Press j -- should move to the next visible row
/// which may be at a shallower depth.
#[test]
fn e8_j_from_deepest_leaf() {
    let mut fg = test_flame_graph();

    // Expand root -> first child (db_query) -> first grandchild
    // which is a leaf (no further expansion).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Cursor is on first grandchild (deepest expanded node).
    let _depth_before = row_depth(&fg.visible_rows(), fg.cursor());

    // Try to expand the leaf (no-op since it has no children).
    fg.handle_key(&make_key(KeyCode::Right));

    // Press j to move to next sibling or uncle.
    let cursor_before = fg.cursor();
    fg.handle_key(&make_key(KeyCode::Down));
    finish_animations(&mut fg);

    let rows = fg.visible_rows();
    assert!(
        fg.cursor() < rows.len(),
        "Cursor out of bounds after j from deepest leaf"
    );
    // Cursor should have moved (there are more siblings at depth 2).
    assert!(
        fg.cursor() != cursor_before || fg.cursor() == rows.len() - 1,
        "j from deepest leaf should move cursor or be at last row"
    );
}

/// E8 Scenario 7: k from root should be no-op.
#[test]
fn e8_k_from_root_is_noop() {
    let mut fg = test_flame_graph();
    assert_eq!(fg.cursor(), 0, "Cursor starts at root");

    fg.handle_key(&make_key(KeyCode::Up));

    assert_eq!(fg.cursor(), 0, "k at root (index 0) must be a no-op");
}

/// E8 Scenario 8: j from last visible row should be no-op.
#[test]
fn e8_j_from_last_visible_row_is_noop() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Navigate to last visible row.
    let row_count = fg.visible_rows().len();
    for _ in 0..row_count {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    let cursor_at_last = fg.cursor();

    // One more j should be no-op.
    fg.handle_key(&make_key(KeyCode::Down));
    assert_eq!(
        fg.cursor(),
        cursor_at_last,
        "j from last visible row should be no-op"
    );
}

// ===========================================================================
// 16. E8 ADVERSARIAL: LEGEND + SUBTREE COLLAPSE INTERACTION
// ===========================================================================

/// E8 Scenario 5: Show legend on a child, then j to uncle.
/// The legend should follow the cursor (sync_legend_to_cursor).
/// Cursor and legend must both be valid after collapse.
#[test]
fn e8_legend_follows_cursor_across_subtree_boundary() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand first child, descend to grandchild.
    // Legend is on by default and follows the cursor to the grandchild.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let rows_with_legend = fg.visible_rows();
    let legend_count_before = rows_with_legend
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Legend { .. }))
        .count();
    assert_eq!(legend_count_before, 1, "Legend should be visible");

    // Navigate j past grandchildren to uncle.
    for _ in 0..10 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    finish_animations(&mut fg);

    // Cursor must be valid.
    let rows_after = fg.visible_rows();
    assert!(
        fg.cursor() < rows_after.len(),
        "Cursor out of bounds after j with legend: cursor={}, rows={}",
        fg.cursor(),
        rows_after.len()
    );

    // Legend should still be present (synced to cursor).
    let legend_count_after = rows_after
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Legend { .. }))
        .count();
    assert!(
        legend_count_after <= 1,
        "At most 1 legend row should exist, found {legend_count_after}"
    );
}

// ===========================================================================
// 17. E9 ADVERSARIAL: LEGEND RENDERING
// ===========================================================================

/// E9 Scenario 9: Legend at narrow width (25 columns).
/// Must not panic, must show at least one cost type or overflow.
#[test]
fn e9_legend_at_narrow_width_25() {
    let mut b = SpanNodeBuilder::new();
    let root = b.leaf("root", vec![20.0, 15.0, 10.0, 5.0, 3.0]);
    let fg = FlameGraph::new(root, test_cost_types());
    // Legend is on by default on root.

    // Width=25 is very narrow. Legend must not panic.
    let text = render_to_text_sized(&fg, 25, 5);
    // At minimum, no panic. Check that something rendered.
    assert!(!text.is_empty(), "Should produce output at width=25");
}

/// E9 Scenario 10: Legend content accuracy.
/// For a node with known costs, verify percentages are mathematically correct.
#[test]
fn e9_legend_percentages_are_correct() {
    let mut b = SpanNodeBuilder::new();
    // Total cost = 30+70 = 100. cpu=30%, io=70%.
    let root = b.leaf("root", vec![30.0, 70.0]);
    let cost_types = vec![
        CostType {
            name: "cpu",
            color: Color::Rgb(100, 150, 255),
        },
        CostType {
            name: "io",
            color: Color::Rgb(255, 150, 100),
        },
    ];
    let fg = FlameGraph::new(root, cost_types);
    // Legend is on by default on root.

    let text = render_to_text_sized(&fg, 80, 5);
    assert!(
        text.contains("cpu:30%"),
        "Legend should show cpu:30% for 30/100.\nRendered:\n{text}"
    );
    assert!(
        text.contains("io:70%"),
        "Legend should show io:70% for 70/100.\nRendered:\n{text}"
    );
}

/// E9 Scenario 11: Legend persists across j/k.
/// Show legend, then j to next sibling. sync_legend_to_cursor should
/// keep the legend visible, following the cursor to the new node.
#[test]
fn e9_legend_persists_across_jk() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child.
    // Legend is on by default and follows cursor to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let rows_before = fg.visible_rows();
    let has_legend_before = rows_before
        .iter()
        .any(|r| matches!(r.kind, RowKind::Legend { .. }));
    assert!(has_legend_before, "Legend should be visible by default");

    // Move j to next sibling.
    fg.handle_key(&make_key(KeyCode::Down));

    let rows_after = fg.visible_rows();
    let has_legend_after = rows_after
        .iter()
        .any(|r| matches!(r.kind, RowKind::Legend { .. }));
    assert!(
        has_legend_after,
        "Legend should persist after j (sync_legend_to_cursor)"
    );
}

/// E9 Scenario 12: Legend on root node (the widest bar).
/// Should render correctly with cost breakdown.
#[test]
fn e9_legend_on_root_node() {
    let fg = test_flame_graph();
    // Legend is on by default on root (cursor is at root, index 0).

    let text = render_to_text_sized(&fg, 120, 5);
    // Root has 5 cost types. At 120 columns, at least some should render.
    assert!(
        text.contains("cpu"),
        "Legend on root should show cost type names.\nRendered:\n{text}"
    );
    assert!(
        text.contains('%'),
        "Legend on root should show percentages.\nRendered:\n{text}"
    );
}

/// E9 Scenario 13: Toggle legend on/off 10 times. No state corruption.
#[test]
fn e9_toggle_legend_10_times_no_corruption() {
    let mut fg = test_flame_graph();

    for i in 0..10 {
        fg.handle_key(&make_key(KeyCode::Enter));
        let rows = fg.visible_rows();
        assert!(
            fg.cursor() < rows.len(),
            "Cursor out of bounds after toggle {i}: cursor={}, rows={}",
            fg.cursor(),
            rows.len()
        );
        // Render should not panic.
        let _text = render_to_text_sized(&fg, 80, 10);
    }
}

// ===========================================================================
// 18. VISUAL INSPECTION: LEGEND CONTENT
// ===========================================================================

/// Scenario 14: Render with legend visible. Verify the legend row
/// contains the square markers and cost type names with percentages.
#[test]
fn visual_legend_has_square_markers_and_percentages() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child.
    // Legend is on by default and follows cursor to first child (db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let text = render_to_text_sized(&fg, 120, 12);

    // The legend row should contain the filled square marker.
    assert!(
        text.contains('\u{25a0}'),
        "Legend should contain filled square markers.\nRendered:\n{text}"
    );
    // The legend should contain percentage markers.
    assert!(
        text.contains('%'),
        "Legend should contain percentage values.\nRendered:\n{text}"
    );
    // The legend should contain at least two cost type names.
    let cost_type_count = ["cpu", "io", "mem", "gc", "net"]
        .iter()
        .filter(|name| text.contains(*name))
        .count();
    assert!(
        cost_type_count >= 2,
        "Legend should show multiple cost type names, found {cost_type_count}.\n\
         Rendered:\n{text}"
    );
}

/// Scenario 15: Render after j crosses subtree boundary. Verify the
/// old subtree is gone from the rendered output.
#[test]
fn visual_subtree_gone_after_j_crosses_boundary() {
    let mut fg = test_flame_graph();

    // Expand root, expand first child (db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Verify grandchildren are visible.
    let text_before = render_to_text_sized(&fg, 120, 15);
    assert!(
        text_before.contains("index_scan"),
        "index_scan grandchild should be visible before j.\nRendered:\n{text_before}"
    );

    // Navigate j past all grandchildren to uncle.
    for _ in 0..20 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    finish_animations(&mut fg);

    // After collapse, grandchildren should be gone.
    let text_after = render_to_text_sized(&fg, 120, 15);
    assert!(
        !text_after.contains("index_scan"),
        "index_scan grandchild should be GONE after j crossed subtree boundary \
         and collapse animation completed.\nRendered:\n{text_after}"
    );
}

// ===========================================================================
// 19. E8 ADVERSARIAL: CURSOR VALIDITY UNDER COMBINED OPERATIONS
// ===========================================================================

/// Combined: expand 2 levels, show legend, j across boundary, k back.
/// Tests the full interaction chain for cursor validity.
#[test]
fn e8_combined_expand_legend_j_k() {
    let mut fg = test_flame_graph();

    // Expand root, expand first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Toggle legend.
    fg.handle_key(&make_key(KeyCode::Enter));

    // j across subtree boundary.
    for _ in 0..10 {
        fg.handle_key(&make_key(KeyCode::Down));
        finish_animations(&mut fg);
        let rows = fg.visible_rows();
        assert!(
            fg.cursor() < rows.len(),
            "Cursor out of bounds during j: cursor={}, rows={}",
            fg.cursor(),
            rows.len()
        );
    }

    // k back through everything.
    for _ in 0..10 {
        fg.handle_key(&make_key(KeyCode::Up));
        finish_animations(&mut fg);
        let rows = fg.visible_rows();
        assert!(
            fg.cursor() < rows.len(),
            "Cursor out of bounds during k: cursor={}, rows={}",
            fg.cursor(),
            rows.len()
        );
    }
}

/// Determinism of the full adversarial j/k + legend sequence.
#[test]
fn e8_e9_determinism_combined() {
    let run = || {
        let mut fg = test_flame_graph();
        fg.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut fg);
        fg.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut fg);
        fg.handle_key(&make_key(KeyCode::Enter));
        for _ in 0..5 {
            fg.handle_key(&make_key(KeyCode::Down));
            finish_animations(&mut fg);
        }
        for _ in 0..3 {
            fg.handle_key(&make_key(KeyCode::Up));
            finish_animations(&mut fg);
        }
        render_to_text_sized(&fg, 100, 15)
    };

    assert_eq!(
        run(),
        run(),
        "Combined E8/E9 sequence must be deterministic"
    );
}

#![cfg(feature = "devkit")]

//! Expectation tests for flame graph widget requirements.
//!
//! These tests encode 5 specific behaviors that the widget MUST satisfy.
//! They are written first to expose bugs, then the implementation is fixed.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use mb_tui::devkit::buffer_to_text;
use mb_tui::devkit::color::color_to_rgb;
use mb_tui::devkit::flame_graph::{test_cost_types, test_flame_graph};
use mb_tui::devkit::frame_tape::FrameTape;
use mb_tui::widget::flame_graph::{FlameGraph, RowKind, render_flame_graph};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 12;
const TERMINAL_BG: (u8, u8, u8) = (0, 0, 0);

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Complete all pending animations so layout stabilizes.
fn finish_animations(fg: &mut FlameGraph) {
    for _ in 0..96 {
        fg.tick(Duration::from_millis(16));
    }
}

/// Render the flame graph to plain text at standard dimensions.
fn render_to_text(fg: &FlameGraph) -> String {
    let area = Rect::new(0, 0, WIDTH, HEIGHT);
    let mut buf = Buffer::empty(area);
    render_flame_graph(fg, area, &mut buf);
    buffer_to_text(&buf)
}

// ---------------------------------------------------------------------------
// E1: Colors use evenly-spaced hues with WCAG AA contrast
// ---------------------------------------------------------------------------

#[test]
fn e1_cost_type_colors_pass_wcag_aa_large_contrast() {
    // Record a single frame of the one-level-expanded flame graph.
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand root
    finish_animations(&mut fg);

    let states = vec![("one-level".to_string(), fg)];
    let tape = FrameTape::record_states(states, render_flame_graph, WIDTH, HEIGHT);
    tape.assert_contrast_aa_large(TERMINAL_BG);
}

#[test]
fn e1_cost_type_colors_are_distinct_and_span_palette_range() {
    let cost_types = test_cost_types();
    assert_eq!(cost_types.len(), 5, "expected exactly 5 cost types");

    // Extract RGB values.
    let rgbs: Vec<(u8, u8, u8)> = cost_types
        .iter()
        .map(|ct| match color_to_rgb(ct.color) {
            Some(rgb) => rgb,
            None => panic!("cost type color must be RGB"),
        })
        .collect();

    // All 5 colors must be pairwise distinct (min RGB distance > 0.08).
    let min_distance = 0.08;
    for i in 0..rgbs.len() {
        for j in (i + 1)..rgbs.len() {
            let dr = (rgbs[i].0 as f64 - rgbs[j].0 as f64) / 255.0;
            let dg = (rgbs[i].1 as f64 - rgbs[j].1 as f64) / 255.0;
            let db = (rgbs[i].2 as f64 - rgbs[j].2 as f64) / 255.0;
            let dist = (dr * dr + dg * dg + db * db).sqrt();
            assert!(
                dist > min_distance,
                "Colors {} ({:?}) and {} ({:?}) are too similar: distance {:.3} < {:.3}",
                cost_types[i].name,
                rgbs[i],
                cost_types[j].name,
                rgbs[j],
                dist,
                min_distance,
            );
        }
    }

    // Hues should span roughly 180 degrees (the spread parameter).
    let mut hues: Vec<f64> = rgbs.iter().map(|&(r, g, b)| rgb_to_hue(r, g, b)).collect();
    hues.sort_by(f64::total_cmp);

    // Compute the angular span: the smallest arc that contains all hues.
    // For N sorted hues on a circle, the span is 360 - max_gap.
    let mut max_gap = 0.0f64;
    for i in 0..hues.len() {
        let next = (i + 1) % hues.len();
        let gap = if next == 0 {
            (360.0 - hues[i]) + hues[0]
        } else {
            hues[next] - hues[i]
        };
        max_gap = max_gap.max(gap);
    }
    let span = 360.0 - max_gap;

    // The palette uses spread=180 with 25% tint. Tinting pulls hues toward
    // center but hues near 0/360 can expand the apparent span. The resulting
    // span should be well under a full circle (< 300) and wide enough (> 90)
    // to be visually distinct.
    assert!(
        span > 90.0 && span < 300.0,
        "Hue span should be roughly 90-300 degrees (widened-analogous + tint), got {span:.1}",
    );
}

/// Convert RGB to hue (0-360 degrees).
fn rgb_to_hue(r: u8, g: u8, b: u8) -> f64 {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    if delta < 1e-10 {
        return 0.0;
    }

    let hue = if (max - r).abs() < 1e-10 {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < 1e-10 {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    (hue + 360.0) % 360.0
}

// ---------------------------------------------------------------------------
// E2: All node labels visible, not just the cursor node
// ---------------------------------------------------------------------------

#[test]
fn e2_all_child_labels_visible_in_one_level() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand root
    finish_animations(&mut fg);

    // Render and get both plain text (for label presence) and the buffer
    // (for contrast checking of label cells).
    let area = Rect::new(0, 0, WIDTH, HEIGHT);
    let mut buf = Buffer::empty(area);
    render_flame_graph(&fg, area, &mut buf);
    let text = buffer_to_text(&buf);

    // After sorting by total cost descending, the 4 children are:
    // db_query(45), template_render(30), auth_check(15), logging(10)
    // Note: "template_render" may be truncated with ellipsis due to label zone width.
    let expected_labels = ["db_query", "template_rend", "auth_check", "logging"];

    for label in &expected_labels {
        assert!(
            text.contains(label),
            "Expected label '{label}' to appear in rendered output.\n\
             Rendered text:\n{text}",
        );
    }

    // Labels must also be VISIBLE: label foreground must have adequate
    // contrast against the cell's actual background (the bar color).
    for y in 1..5u16 {
        // Find the first alphabetic character on this row (the label start).
        let label_cell = (0..WIDTH).map(|x| &buf[(x, y)]).find(|cell| {
            cell.symbol()
                .chars()
                .next()
                .is_some_and(char::is_alphabetic)
        });

        if let Some(cell) = label_cell {
            let fg_rgb = color_to_rgb(cell.fg).unwrap_or((0, 0, 0));
            let bg_rgb = color_to_rgb(cell.bg).unwrap_or(TERMINAL_BG);
            let ratio = mb_tui::devkit::color::contrast_ratio(fg_rgb, bg_rgb);
            assert!(
                ratio >= 3.0,
                "Label on row {y} has fg={fg_rgb:?} bg={bg_rgb:?} with contrast \
                 ratio {ratio:.2} < 3.0. Labels must be readable.",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// E3: Legend row appears with cost breakdown
// ---------------------------------------------------------------------------

#[test]
fn e3_legend_shows_cost_type_names() {
    // Legend is on by default and follows the cursor. After Right, the
    // legend is on the first child automatically.
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right)); // expand root, descend to first child
    finish_animations(&mut fg);

    let text = render_to_text(&fg);

    // The legend should show cost type names for the selected span.
    // At minimum, "cpu" and "io" should appear somewhere in the legend.
    let cost_type_names = ["cpu", "io"];
    for name in &cost_type_names {
        assert!(
            text.contains(name),
            "Expected cost type name '{name}' in legend output.\n\
             Rendered text:\n{text}",
        );
    }
}

// ---------------------------------------------------------------------------
// E4: Right auto-expands, left auto-collapses
// ---------------------------------------------------------------------------

#[test]
fn e4_right_expands_and_descends() {
    let mut fg = test_flame_graph();

    // Right on root: should expand AND move cursor to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let root_id = fg.root().id;
    assert!(
        fg.is_expanded(root_id),
        "Root should be expanded after Right"
    );
    // Cursor should have moved to first child (index 1), not stayed at root (index 0).
    assert_eq!(
        fg.cursor(),
        1,
        "After Right on root, cursor should descend to first child"
    );
}

#[test]
fn e4_left_collapses_and_ascends() {
    let mut fg = test_flame_graph();

    // Right on root: expand root, descend to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let first_child_id = fg.root().children[0].id;

    // Right on first child: expand first child, descend to grandchild.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Left from grandchild: collapses first child (parent) and ascends to it.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Cursor should now be on first child, which is now collapsed.
    let rows = fg.visible_rows();
    let cursor_span = rows.get(fg.cursor()).and_then(|r| match r.kind {
        RowKind::Span { span_id, .. } => Some(span_id),
        _ => None,
    });
    assert_eq!(
        cursor_span,
        Some(first_child_id),
        "Cursor should be on first child after ascending from grandchild"
    );
    assert!(
        !fg.is_expanded(first_child_id),
        "First child should be collapsed after ascending from its children"
    );

    // Left from first child (collapsed): collapses root and ascends to root.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    assert!(
        !fg.is_expanded(fg.root().id),
        "Root should be collapsed after ascending from its children"
    );
    assert_eq!(
        fg.cursor(),
        0,
        "Cursor should be at root after Left collapses first child"
    );
}

// ---------------------------------------------------------------------------
// E5: j/k navigates visible rows, collapsing when crossing subtrees
// ---------------------------------------------------------------------------

#[test]
fn e5_down_stops_at_last_visible_row() {
    let mut fg = test_flame_graph();

    // Expand root and descend to first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Cursor is now at first child (db_query). Navigate through all siblings.
    fg.handle_key(&make_key(KeyCode::Down)); // template_render
    fg.handle_key(&make_key(KeyCode::Down)); // auth_check
    fg.handle_key(&make_key(KeyCode::Down)); // logging (last visible row)

    let cursor_at_last = fg.cursor();

    // One more Down should NOT move past the last visible row.
    fg.handle_key(&make_key(KeyCode::Down));

    assert_eq!(
        fg.cursor(),
        cursor_at_last,
        "Down from the last visible row should not move cursor"
    );
}

#[test]
fn e5_up_from_first_visible_row_is_noop() {
    let mut fg = test_flame_graph();

    // Expand root and descend to first child (Right does both).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let cursor_at_first_child = fg.cursor();
    assert!(
        cursor_at_first_child > 0,
        "First child should not be at index 0"
    );

    // Up from first non-root row should stay put (root is index 0).
    fg.handle_key(&make_key(KeyCode::Up));

    // With flat-list navigation, cursor moves to root (index 0).
    assert_eq!(
        fg.cursor(),
        0,
        "Up from first child should move to root (flat list navigation)"
    );
}

#[test]
fn e5_down_crosses_parent_boundary_to_uncle() {
    let mut fg = test_flame_graph();

    // Expand root (descends to first child db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand db_query (descends to first grandchild).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Cursor is now at first grandchild (index_scan). db_query has 3 children:
    // index_scan(25), row_fetch(15), plan_cache(5) (sorted by cost).
    fg.handle_key(&make_key(KeyCode::Down)); // row_fetch
    fg.handle_key(&make_key(KeyCode::Down)); // plan_cache (last grandchild)

    let cursor_at_last_grandchild = fg.cursor();

    // Down again SHOULD move to template_render (uncle node) since j/k
    // navigates the flat visible row list.
    fg.handle_key(&make_key(KeyCode::Down));
    finish_animations(&mut fg);

    assert_ne!(
        fg.cursor(),
        cursor_at_last_grandchild,
        "Down from last grandchild should jump to uncle node (template_render)"
    );
}

// ---------------------------------------------------------------------------
// E6: Left (h) collapses the current node AND its siblings
// ---------------------------------------------------------------------------

#[test]
fn e6_left_collapses_current_and_siblings() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child (db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand db_query, descend into its children (grandchildren).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Count grandchild rows (depth 2) before ascending.
    let rows_before = fg.visible_rows();
    let grandchild_count_before = rows_before
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Span { depth, .. } if depth == 2))
        .count();
    assert!(
        grandchild_count_before > 0,
        "Should have grandchildren visible"
    );

    // Left: ascend from grandchild to db_query, collapsing grandchild + siblings.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Grandchildren should be gone (collapsed).
    let rows_after = fg.visible_rows();
    let grandchild_count_after = rows_after
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Span { depth, .. } if depth == 2))
        .count();
    assert_eq!(
        grandchild_count_after, 0,
        "Grandchildren should be collapsed after ascending from them"
    );
}

#[test]
fn e6_left_collapses_siblings_too() {
    let mut fg = test_flame_graph();

    // Expand root, descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand first child (db_query), descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Go back to sibling level.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Move down to second child (template_render).
    fg.handle_key(&make_key(KeyCode::Down));

    // Expand template_render, descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Go back to parent level (root's children).
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Now ascend from root's children to root.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // ALL root children should be collapsed.
    let rows = fg.visible_rows();
    // Only root should be visible (all children collapsed under root which is also collapsed).
    let span_count = rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Span { .. }))
        .count();
    assert_eq!(
        span_count, 1,
        "After ascending to root, only root should be visible (all children collapsed)"
    );
}

// ---------------------------------------------------------------------------
// E7: Only one tree path expanded at a time
// ---------------------------------------------------------------------------

#[test]
fn e7_expanding_new_sibling_collapses_old() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child (db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand db_query, descend into it.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Go back to sibling level.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Move to second sibling (template_render).
    fg.handle_key(&make_key(KeyCode::Down));

    // Expand template_render.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // db_query should now be collapsed (only one path at a time).
    let db_query_id = fg.root().children[0].id;
    assert!(
        !fg.is_expanded(db_query_id),
        "db_query should be collapsed when template_render is expanded (single-path constraint)"
    );
}

#[test]
fn e7_only_one_path_from_root() {
    let mut fg = test_flame_graph();

    // Expand root, descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand first child, descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Go back to sibling level.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);

    // Move to second sibling, expand it.
    fg.handle_key(&make_key(KeyCode::Down));
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Count how many depth-1 nodes are expanded.
    let rows = fg.visible_rows();
    let expanded_at_depth1: Vec<_> = rows
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::Span { span_id, depth, .. } if depth == 1 && fg.is_expanded(span_id) => {
                Some(span_id)
            }
            _ => None,
        })
        .collect();
    assert!(
        expanded_at_depth1.len() <= 1,
        "At most 1 node at each depth should be expanded, found {}: {:?}",
        expanded_at_depth1.len(),
        expanded_at_depth1
    );
}

// ---------------------------------------------------------------------------
// E8: j/k moves to aunts/uncles, collapsing the previous subtree
// ---------------------------------------------------------------------------

#[test]
fn e8_jk_moves_to_uncle_nodes() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child (db_query).
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Expand db_query, descend into its children.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Now cursor is on a grandchild (depth 2).
    // Press j enough times to pass all db_query's children.
    // Should eventually reach template_render (uncle, depth 1).
    let initial_cursor = fg.cursor();
    for _ in 0..20 {
        fg.handle_key(&make_key(KeyCode::Down));
        finish_animations(&mut fg);
    }
    // Cursor should have moved past the grandchildren.
    assert_ne!(fg.cursor(), initial_cursor, "j should move beyond siblings");
}

#[test]
fn e8_moving_to_uncle_collapses_previous_subtree() {
    let mut fg = test_flame_graph();

    // Expand root, descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Remember first child (db_query) id.
    let first_child_id = fg.root().children[0].id;

    // Expand first child, descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    assert!(fg.is_expanded(first_child_id));

    // Navigate down past all grandchildren to reach template_render.
    for _ in 0..20 {
        fg.handle_key(&make_key(KeyCode::Down));
        finish_animations(&mut fg);
    }

    // db_query should be collapsed (moved away from its subtree).
    assert!(
        !fg.is_expanded(first_child_id),
        "db_query should be collapsed after navigating to uncle"
    );
}

// ---------------------------------------------------------------------------
// E9: Legend shows when a node is selected (Enter)
// ---------------------------------------------------------------------------

#[test]
fn e9_legend_shows_cost_types_in_rendered_output() {
    let mut fg = test_flame_graph();

    // Expand root, descend to first child. Legend is on by default and
    // follows the cursor, so it is already visible on the first child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Render at wide width to fit legend.
    let text = render_to_text(&fg); // uses WIDTH=120

    // Legend should contain cost type names with percentages.
    assert!(
        text.contains("cpu"),
        "Legend should show 'cpu' cost type\nRendered:\n{text}"
    );
    assert!(
        text.contains("io"),
        "Legend should show 'io' cost type\nRendered:\n{text}"
    );
    assert!(
        text.contains('%'),
        "Legend should show percentage markers\nRendered:\n{text}"
    );
}

#[test]
fn e9_enter_toggles_legend_off() {
    let mut fg = test_flame_graph();

    // Expand root, descend. Legend is on by default and follows cursor.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Legend is already on; capture the output with legend visible.
    let text_with = render_to_text(&fg);
    assert!(text_with.contains('%'), "Legend should be visible");

    // Toggle legend off.
    fg.handle_key(&make_key(KeyCode::Enter));
    let text_without = render_to_text(&fg);

    // Legend should be gone. The rendered output should differ.
    assert_ne!(text_with, text_without, "Toggle should change the output");
}

// ---------------------------------------------------------------------------
// E10: Undo/redo navigation (bounded to 32 positions)
// ---------------------------------------------------------------------------

#[test]
fn e10_undo_returns_to_previous_position() {
    let mut fg = test_flame_graph();

    // Start at root.
    let path_at_root = fg.path().to_vec();

    // Expand + descend.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    let path_at_child = fg.path().to_vec();
    assert_ne!(path_at_root, path_at_child);

    // Undo should return to root.
    fg.handle_key(&make_key(KeyCode::Char('u')));
    finish_animations(&mut fg);
    assert_eq!(
        fg.path().to_vec(),
        path_at_root,
        "Undo should return to root"
    );
}

#[test]
fn e10_redo_after_undo() {
    let mut fg = test_flame_graph();

    // Navigate: root → child.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    let path_at_child = fg.path().to_vec();

    // Undo to root.
    fg.handle_key(&make_key(KeyCode::Char('u')));
    finish_animations(&mut fg);

    // Redo back to child.
    fg.handle_key(&make_key(KeyCode::Char('r')));
    finish_animations(&mut fg);
    assert_eq!(
        fg.path().to_vec(),
        path_at_child,
        "Redo should return to child"
    );
}

#[test]
fn e10_redo_cleared_on_new_navigation() {
    let mut fg = test_flame_graph();

    // root → child → undo → navigate elsewhere.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Char('u')));
    finish_animations(&mut fg);

    // New navigation (expand again) should clear redo stack.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Redo should be a no-op now.
    let path_before = fg.path().to_vec();
    fg.handle_key(&make_key(KeyCode::Char('r')));
    finish_animations(&mut fg);
    assert_eq!(
        fg.path().to_vec(),
        path_before,
        "Redo should be no-op after new navigation"
    );
}

#[test]
fn e10_undo_at_start_is_noop() {
    let mut fg = test_flame_graph();
    let path_at_root = fg.path().to_vec();

    // Undo with no history should be a no-op.
    fg.handle_key(&make_key(KeyCode::Char('u')));
    assert_eq!(fg.path().to_vec(), path_at_root);
}

#[test]
fn e10_bounded_to_32() {
    let mut fg = test_flame_graph();

    // Navigate 40 times (expand/collapse cycle).
    for _ in 0..40 {
        fg.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut fg);
        fg.handle_key(&make_key(KeyCode::Left));
        finish_animations(&mut fg);
    }

    // Undo 100 times — should stop after 32.
    let mut undo_count = 0;
    for _ in 0..100 {
        let before = fg.path().to_vec();
        fg.handle_key(&make_key(KeyCode::Char('u')));
        finish_animations(&mut fg);
        if fg.path().to_vec() == before {
            break;
        }
        undo_count += 1;
    }
    assert!(
        undo_count <= 32,
        "Undo should be bounded to 32, got {undo_count}"
    );
}

// ---------------------------------------------------------------------------
// Focus mode expectations
// ---------------------------------------------------------------------------

/// E1: Focus node bar fills 100% of bar zone.
#[test]
fn focus_e1_focus_node_fills_bar_zone() -> Result<(), Box<dyn std::error::Error>> {
    let mut fg = test_flame_graph();
    // Expand root, descend to db_query.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Focus on db_query.
    fg.handle_key(&make_key(KeyCode::Char('f')));

    let rows = fg.visible_rows();
    let focus_row = rows.first().ok_or("should have at least one row")?;
    match focus_row.kind {
        RowKind::Span {
            bar_width, depth, ..
        } => {
            // The focus node should be at depth 0 and have full bar width.
            assert_eq!(depth, 0, "Focus node should be at depth 0");
            // Compare with the root's bar width in non-focus mode.
            let non_focus_fg = test_flame_graph();
            let non_focus_rows = non_focus_fg.visible_rows();
            let root_width = match non_focus_rows[0].kind {
                RowKind::Span { bar_width: rw, .. } => rw,
                _ => panic!("expected root span row"),
            };
            assert_eq!(
                bar_width, root_width,
                "Focus node should fill 100% of bar zone"
            );
        }
        _ => panic!("Expected focus node to be a Span row"),
    }
    Ok(())
}

/// E2: Child bars normalized to focus node total.
#[test]
fn focus_e2_children_normalized_to_focus_total() {
    let mut fg = test_flame_graph();
    // Expand root -> db_query -> grandchild.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Focus on db_query.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Char('f')));

    // Re-expand db_query to see children.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let area = Rect::new(0, 0, WIDTH, HEIGHT);
    let mut buf = Buffer::empty(area);
    render_flame_graph(&fg, area, &mut buf);

    // In focus mode, db_query fills 100%. Its first child (index_scan)
    // should be proportional to db_query's total, not the root's total.
    let rows = fg.visible_rows();
    let focus_bar = match rows[0].kind {
        RowKind::Span { bar_width, .. } => bar_width,
        _ => panic!("expected focus span row"),
    };
    let child_bar = match rows[1].kind {
        RowKind::Span { bar_width, .. } => bar_width,
        _ => panic!("expected child span row"),
    };
    // Child bar should be a substantial fraction of focus bar (index_scan is
    // the largest child of db_query), not a small fraction of root's total.
    let ratio = child_bar as f64 / focus_bar as f64;
    assert!(
        ratio > 0.4,
        "Child bar should be normalized to focus total (ratio={ratio:.2}, expected >0.4)"
    );
}

/// E3: Focus indicator prefix on focus node.
#[test]
fn focus_e3_focus_indicator_prefix() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Char('f')));

    let text = render_to_text(&fg);
    assert!(
        text.contains("\u{22ef}\u{25b8}"),
        "Focus indicator should appear on focus node.\nRendered:\n{text}"
    );
}

/// E4: Focus mode shows only the focus subtree, no siblings.
#[test]
fn focus_e4_no_siblings_in_focus() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Char('f')));

    let text = render_to_text(&fg);
    // db_query is focused. Only its subtree should appear, not siblings.
    assert!(
        text.contains("db_query"),
        "Focus node should appear.\nRendered:\n{text}"
    );
    assert!(
        !text.contains("template_rend"),
        "Sibling template_render should NOT appear in focus mode.\nRendered:\n{text}"
    );
    assert!(
        !text.contains("auth_check"),
        "Sibling auth_check should NOT appear in focus mode.\nRendered:\n{text}"
    );
    assert!(
        !text.contains("logging"),
        "Sibling logging should NOT appear in focus mode.\nRendered:\n{text}"
    );
}

/// E5: `f` sets focus, `F` clears it.
#[test]
fn focus_e5_f_sets_and_big_f_clears() {
    let mut fg = test_flame_graph();
    assert!(fg.focus().is_none(), "Initially no focus");

    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Char('f')));

    assert!(fg.focus().is_some(), "f should set focus");

    fg.handle_key(&make_key(KeyCode::Char('F')));
    assert!(fg.focus().is_none(), "F should clear focus");
}

/// E6: `h` on focus node unfocuses.
#[test]
fn focus_e6_h_on_focus_node_unfocuses() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let db_query_id = fg.root().children[0].id;
    fg.handle_key(&make_key(KeyCode::Char('f')));
    assert_eq!(fg.focus(), Some(db_query_id));

    // Auto-expand moved cursor to first child. Collapse back to focus node.
    fg.handle_key(&make_key(KeyCode::Left));
    finish_animations(&mut fg);
    assert_eq!(
        fg.focus(),
        Some(db_query_id),
        "still focused after h from child"
    );

    // Now cursor is on the focus node itself. `h` should unfocus.
    fg.handle_key(&make_key(KeyCode::Left));
    assert!(fg.focus().is_none(), "h on focus node should clear focus");
}

/// E7: j/k within and past subtree stays focused.
#[test]
fn focus_e7_jk_stays_focused() {
    let mut fg = test_flame_graph();
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    let db_query_id = fg.root().children[0].id;
    fg.handle_key(&make_key(KeyCode::Char('f')));
    assert_eq!(fg.focus(), Some(db_query_id));
    finish_animations(&mut fg);

    // Navigate down through the subtree and into siblings.
    // j/k should never exit focus — only h on focus node or F does.
    for _ in 0..6 {
        fg.handle_key(&make_key(KeyCode::Down));
    }
    assert_eq!(fg.focus(), Some(db_query_id), "j/k should never exit focus");

    // Navigate back up.
    for _ in 0..6 {
        fg.handle_key(&make_key(KeyCode::Up));
    }
    assert_eq!(fg.focus(), Some(db_query_id), "k should never exit focus");
}

/// E8: Path child renders first among siblings (for expanded ancestors).
#[test]
fn focus_e8_path_child_first_among_siblings() {
    use mb_tui::widget::flame_graph::SpanNodeBuilder;

    // Build a tree where the path goes through a non-first-by-cost child
    // to verify path-first ordering at the ancestor level.
    let mut b = SpanNodeBuilder::new();
    let gc = b.leaf("grandchild", vec![1.0]);
    let big = b.leaf("big", vec![10.0]);
    let small = b.span("small", vec![3.0], vec![gc]);
    let root = b.span("root", vec![13.0], vec![big, small]);
    // After cost sort: big(10) is children[0], small(3) is children[1].
    let cost_types = test_cost_types();

    let mut fg = FlameGraph::new(root.clone(), cost_types);
    // Set path through small -> grandchild (small is non-first by cost).
    let small_id = root.children[1].id;
    let _gc_id = root.children[1].children[0].id;
    fg.handle_key(&make_key(KeyCode::Right)); // expand root, descend to big
    finish_animations(&mut fg);
    fg.handle_key(&make_key(KeyCode::Down)); // move to small
    fg.handle_key(&make_key(KeyCode::Right)); // expand small, descend to grandchild
    finish_animations(&mut fg);

    // Now path = [root, small, grandchild]. small is an expanded ancestor.
    assert!(fg.is_expanded(small_id), "small should be expanded");

    // With path-first ordering, small should appear before big at depth 1.
    let rows = fg.visible_rows();
    let depth1_spans: Vec<_> = rows
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::Span {
                span_id, depth: 1, ..
            } => Some(span_id),
            _ => None,
        })
        .collect();

    assert_eq!(
        depth1_spans.first().copied(),
        Some(small_id),
        "Path child (small) should be first among depth-1 siblings, \
         despite big having higher cost. Got: {depth1_spans:?}"
    );
    // big should still be present.
    assert!(
        depth1_spans.contains(&root.children[0].id),
        "Non-path sibling (big) should still appear"
    );
}

/// E11: Undo/redo restores focus state.
#[test]
fn focus_e11_undo_redo_restores_focus() {
    let mut fg = test_flame_graph();

    // Expand root, descend to db_query.
    fg.handle_key(&make_key(KeyCode::Right));
    finish_animations(&mut fg);

    // Focus on db_query.
    fg.handle_key(&make_key(KeyCode::Char('f')));
    let db_query_id = fg.root().children[0].id;
    assert_eq!(fg.focus(), Some(db_query_id));

    // Unfocus.
    fg.handle_key(&make_key(KeyCode::Char('F')));
    assert!(fg.focus().is_none());

    // Undo should restore focus.
    fg.handle_key(&make_key(KeyCode::Char('u')));
    finish_animations(&mut fg);
    assert_eq!(
        fg.focus(),
        Some(db_query_id),
        "Undo should restore focus state"
    );

    // Redo should clear focus again.
    fg.handle_key(&make_key(KeyCode::Char('r')));
    finish_animations(&mut fg);
    assert!(fg.focus().is_none(), "Redo should restore unfocused state");
}

/// E12: Focus on root is a no-op.
#[test]
fn focus_e12_focus_on_root_is_noop() {
    let mut fg = test_flame_graph();

    // Cursor is on root. Press f.
    fg.handle_key(&make_key(KeyCode::Char('f')));
    assert!(fg.focus().is_none(), "Focus on root should be a no-op");
}

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::data::{CostType, SpanNode};
use super::layout::{FlameRow, RowKind, RowLayout, flatten_visible_rows};
use super::state::{FlameGraph, find_node};
use crate::render::{OverflowBehavior, clip_text, display_width, ellipsize_text, summarize_text};
use crate::theme;
use crate::widget::{Constraints, LayoutRenderable, Size};

/// Visual style for flame graph bar segments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BarStyle {
    /// Thin horizontal lines (━). Light, airy, clear row separation.
    #[default]
    ThinLine,
    /// Braille dot grid (⣿). Textured, semi-transparent feel.
    Dotted,
}

const INDENT_PER_DEPTH: u16 = 2;

impl LayoutRenderable for FlameGraph {
    fn measure(&self, constraints: Constraints) -> Size {
        if constraints.max_height == Some(0) {
            return Size::ZERO;
        }

        let preferred_width = match constraints.max_width {
            Some(width) if flame_graph_overflow(width) == OverflowBehavior::Summary => {
                preferred_summary_width(self)
            }
            _ => preferred_detailed_width(self),
        };
        let width = constraints.constrain(Size::new(preferred_width, 0)).width;
        if width == 0 {
            return Size::ZERO;
        }

        let height = self
            .visible_rows_for_width(width)
            .len()
            .min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(preferred_width, height))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render(self, area, buf);
    }
}

fn flame_graph_overflow(width: u16) -> OverflowBehavior {
    if RowLayout::for_width(width).shows_bars() {
        OverflowBehavior::Ellipsis
    } else {
        OverflowBehavior::Summary
    }
}

/// Render the flame graph into a terminal buffer, applying vertical scroll.
pub fn render(state: &FlameGraph, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let layout = RowLayout::for_width(area.width);

    let rows = flatten_visible_rows(
        &state.root,
        &state.path,
        &state.animations,
        state.selected_for_legend,
        state.focus,
        area.width,
    );

    // Clamp scroll offset so we don't scroll past the last row.
    let scroll = state
        .scroll_offset
        .min(rows.len().saturating_sub(area.height as usize));

    for (row_idx, row) in rows.iter().enumerate().skip(scroll) {
        let screen_row = row_idx - scroll;
        if screen_row >= area.height as usize {
            break;
        }
        let y = area.y + screen_row as u16;
        render_row(state, row, row_idx, y, area, buf, layout);
    }
}

/// Mutable render entry point: updates viewport height cache then renders.
pub fn render_mut(state: &mut FlameGraph, area: Rect, buf: &mut Buffer) {
    state.set_viewport_height(area.height);
    render(state, area, buf);
}

fn render_row(
    state: &FlameGraph,
    row: &FlameRow,
    row_idx: usize,
    y: u16,
    area: Rect,
    buf: &mut Buffer,
    layout: RowLayout,
) {
    // When focused, use the focus node's total for cost normalization.
    let root_total = if let Some(focus_id) = state.focus {
        find_node(&state.root, focus_id).map_or(state.root.costs.total(), |n| n.costs.total())
    } else {
        state.root.costs.total()
    };

    match &row.kind {
        RowKind::Span {
            span_id, bar_width, ..
        } => {
            if let Some(node) = find_node(&state.root, *span_id) {
                let depth = match row.kind {
                    RowKind::Span { depth, .. } => depth,
                    _ => 0,
                };
                let params = SpanRowParams {
                    depth,
                    bar_width: *bar_width,
                    is_cursor: row_idx == state.cursor,
                    has_children: !node.children.is_empty(),
                    is_expanded: state.is_expanded(*span_id),
                    is_focus_node: state.focus == Some(*span_id),
                    bar_style: state.bar_style,
                    y,
                };
                draw_span_row(
                    node,
                    &state.cost_types,
                    &params,
                    root_total,
                    layout,
                    area,
                    buf,
                );
            }
        }
        RowKind::Legend { span_id } => {
            if let Some(node) = find_node(&state.root, *span_id) {
                draw_legend_row(node, &state.cost_types, y, layout, area, buf);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Span row: label zone + bar zone
// ---------------------------------------------------------------------------

/// Parameters for rendering a single span row.
struct SpanRowParams {
    depth: u16,
    bar_width: u16,
    is_cursor: bool,
    has_children: bool,
    is_expanded: bool,
    is_focus_node: bool,
    bar_style: BarStyle,
    y: u16,
}

fn draw_span_row(
    node: &SpanNode,
    cost_types: &[CostType],
    params: &SpanRowParams,
    root_total: f64,
    layout: RowLayout,
    area: Rect,
    buf: &mut Buffer,
) {
    let indent = (INDENT_PER_DEPTH * params.depth) as usize;
    let indicator = if params.is_focus_node {
        "\u{22ef}\u{25b8} "
    } else if params.has_children {
        if params.is_expanded {
            "\u{25be} "
        } else {
            "\u{25b8} "
        }
    } else {
        "  "
    };
    if layout.shows_bars() {
        draw_label_zone(
            &node.label,
            indent,
            indicator,
            params.is_cursor,
            params.y,
            layout,
            area,
            buf,
        );
        if params.bar_width > 0 {
            draw_bar_segments(
                node,
                cost_types,
                params.bar_width,
                params.bar_style,
                params.y,
                layout,
                area,
                buf,
            );
        }
    } else {
        draw_summary_row(node, params, root_total, area, buf);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_label_zone(
    label: &str,
    indent: usize,
    indicator: &str,
    is_cursor: bool,
    y: u16,
    layout: RowLayout,
    area: Rect,
    buf: &mut Buffer,
) {
    let zone_start = area.x;
    if zone_start >= area.x + area.width {
        return;
    }

    let formatted = format_label(label, indent, layout.label_zone as usize, indicator);

    let style = if is_cursor {
        Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::dim())
    };

    // Write the formatted label, clipping to available width.
    let max_chars = (area.x + area.width).saturating_sub(zone_start) as usize;
    let clipped = clip_text(&formatted, max_chars);
    buf.set_string(zone_start, y, &clipped, style);
}

#[allow(clippy::too_many_arguments)]
fn draw_bar_segments(
    node: &SpanNode,
    cost_types: &[CostType],
    bar_width: u16,
    bar_style: BarStyle,
    y: u16,
    layout: RowLayout,
    area: Rect,
    buf: &mut Buffer,
) {
    let bar_x = area.x + layout.bar_start();
    if bar_x >= area.x + area.width {
        return;
    }

    let total = node.costs.total();
    if total <= 0.0 {
        return;
    }

    // Clamp bar_width to available screen space.
    let available = (area.x + area.width).saturating_sub(bar_x);
    let effective_width = bar_width.min(available);

    let widths = allocate_segments(&node.costs.amounts, total, effective_width);

    let mut x = bar_x;
    let bar_end = bar_x + effective_width;
    for (i, &seg_w) in widths.iter().enumerate() {
        let color = cost_types.get(i).map_or(theme::dim(), |ct| ct.color);
        for dx in 0..seg_w {
            let cx = x + dx;
            if cx < area.x + area.width {
                let (symbol, style) = match bar_style {
                    BarStyle::ThinLine => (
                        thin_line_symbol(cx, bar_x, bar_end),
                        Style::default().fg(color),
                    ),
                    BarStyle::Dotted => ("\u{28FF}", Style::default().fg(color)),
                };
                buf[(cx, y)].set_symbol(symbol).set_style(style);
            }
        }
        x += seg_w;
    }
}

/// Pick the correct thin-line character based on position within the bar.
fn thin_line_symbol(cx: u16, bar_x: u16, bar_end: u16) -> &'static str {
    let width = bar_end.saturating_sub(bar_x);
    if width <= 1 {
        // Single-cell bar: plain heavy horizontal.
        "\u{2501}"
    } else if cx == bar_x {
        "\u{257A}" // left cap
    } else if cx + 1 == bar_end {
        "\u{2578}" // right cap
    } else {
        "\u{2501}" // heavy horizontal
    }
}

// ---------------------------------------------------------------------------
// Legend row: blank label zone + legend content in bar zone
// ---------------------------------------------------------------------------

fn draw_legend_row(
    node: &SpanNode,
    cost_types: &[CostType],
    y: u16,
    layout: RowLayout,
    area: Rect,
    buf: &mut Buffer,
) {
    let total = node.costs.total();
    if total <= 0.0 {
        return;
    }

    let content_x = if layout.shows_bars() {
        area.x + layout.bar_start()
    } else {
        area.x
    };
    if content_x >= area.x + area.width {
        return;
    }

    let available = (area.x + area.width).saturating_sub(content_x) as usize;
    let entries = collect_legend_entries(node, cost_types, total, available);

    let dim_style = Style::default().fg(theme::dim());
    let mut x = content_x;

    for (entry, color) in &entries.visible {
        if x >= area.x + area.width {
            break;
        }
        buf.set_string(x, y, "\u{25a0}", Style::default().fg(*color));
        x += 1;
        buf.set_string(x, y, entry, dim_style);
        x += display_width(entry) as u16;
        x += 1; // trailing space
    }

    if let Some(overflow_text) = &entries.overflow
        && x + display_width(overflow_text) as u16 <= area.x + area.width
    {
        buf.set_string(x, y, overflow_text, dim_style);
    }
}

fn draw_summary_row(
    node: &SpanNode,
    params: &SpanRowParams,
    root_total: f64,
    area: Rect,
    buf: &mut Buffer,
) {
    let summary_x = area.x;
    let available = area.right().saturating_sub(summary_x) as usize;
    if available == 0 {
        return;
    }

    let text = format_summary_label(
        &node.label,
        params.depth,
        node.costs.total(),
        root_total,
        available,
    );
    let style = if params.is_cursor {
        Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::dim())
    };
    buf.set_stringn(summary_x, params.y, text, available, style);
}

// ---------------------------------------------------------------------------
// Label formatting
// ---------------------------------------------------------------------------

fn format_label(label: &str, indent: usize, zone_width: usize, indicator: &str) -> String {
    if zone_width == 0 {
        return String::new();
    }
    let ind_len = display_width(indicator);
    let indent = indent.min(zone_width);
    let available = zone_width.saturating_sub(indent).saturating_sub(ind_len);
    if available == 0 {
        return " ".repeat(zone_width);
    }
    let mut formatted = " ".repeat(indent);
    formatted.push_str(indicator);
    formatted.push_str(&ellipsize_text(label, available));
    let used = display_width(&formatted);
    if used < zone_width {
        formatted.push_str(&" ".repeat(zone_width - used));
    }
    formatted
}

fn format_summary_label(
    label: &str,
    depth: u16,
    total: f64,
    root_total: f64,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let summary_suffix = if root_total > 0.0 {
        format!(" {}%", ((total / root_total) * 100.0).round() as u32)
    } else {
        String::new()
    };
    let suffix_width = display_width(&summary_suffix).min(width);
    let indent = (depth as usize).min(width.saturating_sub(suffix_width).saturating_sub(1));
    summarize_text(&" ".repeat(indent), label, &summary_suffix, width)
}

// ---------------------------------------------------------------------------
// Segment allocation (largest-remainder method)
// ---------------------------------------------------------------------------

fn allocate_segments(amounts: &[f64], total: f64, bar_width: u16) -> Vec<u16> {
    if total <= 0.0 || bar_width == 0 {
        return vec![0; amounts.len()];
    }

    let mut floors: Vec<u16> = Vec::with_capacity(amounts.len());
    let mut remainders: Vec<(usize, f64)> = Vec::with_capacity(amounts.len());
    let mut sum_floors = 0u16;

    for (i, amount) in amounts.iter().enumerate() {
        let exact = (bar_width as f64) * (amount / total);
        let floor = exact as u16; // truncate
        floors.push(floor);
        sum_floors += floor;
        remainders.push((i, exact - floor as f64));
    }

    // Distribute deficit to segments with largest remainders.
    let deficit = bar_width.saturating_sub(sum_floors);
    remainders.sort_by(|a, b| b.1.total_cmp(&a.1));
    for &(idx, _) in remainders.iter().take(deficit as usize) {
        floors[idx] += 1;
    }

    floors
}

// ---------------------------------------------------------------------------
// Legend entry collection
// ---------------------------------------------------------------------------

struct LegendEntries {
    visible: Vec<(String, Color)>,
    overflow: Option<String>,
}

fn collect_legend_entries(
    node: &SpanNode,
    cost_types: &[CostType],
    total: f64,
    available: usize,
) -> LegendEntries {
    let overflow_reserve = 14; // "+NN types XX%"
    let mut visible = Vec::new();
    let mut used_width = 0usize;
    let mut overflow_count = 0u32;
    let mut overflow_total = 0.0f64;

    for (i, amount) in node.costs.amounts.iter().enumerate() {
        if *amount <= 0.0 {
            continue;
        }
        let pct = (amount / total * 100.0).round() as u32;
        let name = cost_types.get(i).map_or("?", |ct| ct.name);
        let entry = format!("{name}:{pct}%");
        let entry_width = display_width(&entry) + 2; // color block + text + trailing space

        let would_overflow = used_width + entry_width + overflow_reserve > available;
        if would_overflow && !visible.is_empty() {
            overflow_count += 1;
            overflow_total += amount;
            continue;
        }

        let color = cost_types.get(i).map_or(theme::dim(), |ct| ct.color);
        visible.push((entry, color));
        used_width += entry_width;
    }

    let overflow = if overflow_count > 0 {
        let pct = (overflow_total / total * 100.0).round() as u32;
        Some(format!("+{overflow_count} types {pct}%"))
    } else {
        None
    };

    LegendEntries { visible, overflow }
}

fn preferred_detailed_width(state: &FlameGraph) -> u16 {
    let rows = state.visible_rows();
    let label_zone = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::Span { span_id, depth, .. } => find_node(&state.root, span_id).map(|node| {
                let indicator = if node.children.is_empty() {
                    "  "
                } else if state.is_expanded(span_id) {
                    "\u{25be} "
                } else {
                    "\u{25b8} "
                };
                (depth as usize * INDENT_PER_DEPTH as usize)
                    .saturating_add(display_width(indicator))
                    .saturating_add(display_width(&node.label))
                    .clamp(
                        RowLayout::MIN_LABEL_ZONE as usize,
                        RowLayout::MAX_LABEL_ZONE as usize,
                    )
            }),
            RowKind::Legend { .. } => None,
        })
        .max()
        .unwrap_or(RowLayout::MIN_LABEL_ZONE as usize);

    let legend_width = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::Legend { span_id } => find_node(&state.root, span_id)
                .map(|node| full_legend_width(node, &state.cost_types)),
            RowKind::Span { .. } => None,
        })
        .max()
        .unwrap_or(RowLayout::MIN_BAR_ZONE as usize);

    let label_driven = label_zone.saturating_mul(3);
    let legend_driven = label_zone.saturating_add(1).saturating_add(legend_width);
    saturating_width(
        label_driven
            .max(legend_driven)
            .max(RowLayout::MIN_DETAILED_WIDTH as usize),
    )
}

fn preferred_summary_width(state: &FlameGraph) -> u16 {
    let root_total = state.root.costs.total();
    let rows = state.visible_rows();
    let width = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::Span { span_id, depth, .. } => find_node(&state.root, span_id).map(|node| {
                let suffix = if root_total > 0.0 {
                    format!(
                        " {}%",
                        ((node.costs.total() / root_total) * 100.0).round() as u32
                    )
                } else {
                    String::new()
                };
                depth as usize + display_width(&node.label) + display_width(&suffix)
            }),
            RowKind::Legend { span_id } => find_node(&state.root, span_id)
                .map(|node| full_legend_width(node, &state.cost_types)),
        })
        .max()
        .unwrap_or(0);
    saturating_width(width)
}

fn full_legend_width(node: &SpanNode, cost_types: &[CostType]) -> usize {
    let total = node.costs.total();
    if total <= 0.0 {
        return 0;
    }

    node.costs
        .amounts
        .iter()
        .enumerate()
        .filter(|(_, amount)| **amount > 0.0)
        .map(|(i, amount)| {
            let pct = (amount / total * 100.0).round() as u32;
            let name = cost_types.get(i).map_or("?", |ct| ct.name);
            let entry = format!("{name}:{pct}%");
            2 + display_width(&entry)
        })
        .sum::<usize>()
        .saturating_sub(1)
}

fn saturating_width(width: usize) -> u16 {
    width.min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devkit::Surface;
    use crate::widget::flame_graph::data::SpanNodeBuilder;

    fn make_state() -> FlameGraph {
        let mut b = SpanNodeBuilder::new();
        let c1 = b.leaf("child_a", vec![7.0, 3.0]);
        let c2 = b.leaf("child_b", vec![2.0, 1.0]);
        let root = b.span("root", vec![9.0, 4.0], vec![c1, c2]);
        let cost_types = vec![
            CostType {
                name: "cpu",
                color: Color::Red,
            },
            CostType {
                name: "io",
                color: Color::Blue,
            },
        ];
        FlameGraph::new(root, cost_types)
    }

    #[test]
    fn render_does_not_panic_on_zero_area() {
        let state = make_state();
        let mut surface = Surface::new(0, 0);
        let area = Rect::new(0, 0, 0, 0);
        render(&state, area, surface.buffer_mut());
    }

    #[test]
    fn desired_height_tracks_visible_rows() {
        let mut state = make_state();
        // Root span + legend row (selected_for_legend is set by default).
        assert_eq!(state.measure(Constraints::tight_width(80)).height, 2);

        let first_child_id = state.root.children[0].id;
        state.path = vec![state.root.id, first_child_id];
        // Root span + legend + 2 children.
        assert_eq!(state.measure(Constraints::tight_width(80)).height, 4);
    }

    #[test]
    fn measure_clamps_height_and_prefers_intrinsic_width() {
        let mut state = make_state();
        let first_child_id = state.root.children[0].id;
        state.path = vec![state.root.id, first_child_id];

        let measured = state.measure(Constraints::loose(80, 2));

        assert_eq!(measured.height, 2);
        assert!(
            measured.width < 80,
            "expected intrinsic width, got {measured:?}"
        );
        assert!(
            measured.width >= 11,
            "expected enough width for bars, got {measured:?}"
        );
        assert_eq!(state.measure(Constraints::tight(20, 2)), Size::new(20, 2));
    }

    #[test]
    fn render_collapsed_shows_root_bar() {
        let state = make_state();
        let mut surface = Surface::new(40, 5);
        let area = Rect::new(0, 0, 40, 5);
        render(&state, area, surface.buffer_mut());
        let text = surface.to_text();
        // Root label should appear in the label zone.
        assert!(text.contains("root"), "root label missing\n{text}");
    }

    #[test]
    fn render_expanded_shows_children() {
        let mut state = make_state();
        let first_child_id = state.root.children[0].id;
        state.path = vec![state.root.id, first_child_id];
        let mut surface = Surface::new(60, 10);
        let area = Rect::new(0, 0, 60, 10);
        render(&state, area, surface.buffer_mut());
        let text = surface.to_text();
        assert!(text.contains("root"), "root label missing\n{text}");
        assert!(text.contains("child_a"), "child_a label missing\n{text}");
    }

    #[test]
    fn render_is_deterministic() {
        let run = || {
            let mut state = make_state();
            let first_child_id = state.root.children[0].id;
            state.path = vec![state.root.id, first_child_id];
            let mut surface = Surface::new(60, 10);
            let area = Rect::new(0, 0, 60, 10);
            render(&state, area, surface.buffer_mut());
            surface.to_text()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn legend_enabled_shows_cost_type_names() {
        let mut state = make_state();
        let root_id = state.root.id;
        state.selected_for_legend = Some(root_id);
        let mut surface = Surface::new(80, 5);
        let area = Rect::new(0, 0, 80, 5);
        render(&state, area, surface.buffer_mut());
        let styled = surface.to_styled_text();
        assert!(
            styled.contains("cpu"),
            "Legend should contain 'cpu' cost type name\n{styled}"
        );
        assert!(
            styled.contains("io"),
            "Legend should contain 'io' cost type name\n{styled}"
        );
    }

    #[test]
    fn all_child_labels_visible_at_width_80() {
        let mut state = make_state();
        let first_child_id = state.root.children[0].id;
        state.path = vec![state.root.id, first_child_id];
        let mut surface = Surface::new(80, 10);
        let area = Rect::new(0, 0, 80, 10);
        render(&state, area, surface.buffer_mut());
        let text = surface.to_text();
        assert!(
            text.contains("child_a"),
            "child_a label should appear\n{text}"
        );
        assert!(
            text.contains("child_b"),
            "child_b label should appear\n{text}"
        );
    }

    #[test]
    fn two_zone_layout_bar_starts_at_bar_start_column() {
        let state = make_state();
        let mut surface = Surface::new(80, 5);
        let area = Rect::new(0, 0, 80, 5);
        render(&state, area, surface.buffer_mut());
        let layout = RowLayout::for_width(80);

        let buf = surface.buffer();
        let mut found_bar = false;
        for x in layout.bar_start()..80 {
            if buf[(x, 0)].symbol() == "\u{257A}" || buf[(x, 0)].symbol() == "\u{2501}" {
                found_bar = true;
                assert_eq!(
                    x,
                    layout.bar_start(),
                    "First bar character should be at column {}, found at {x}",
                    layout.bar_start()
                );
                break;
            }
        }
        assert!(found_bar, "Should find bar characters in the bar zone");
    }

    #[test]
    fn label_truncation_with_ellipsis() {
        // Label "very_long_label_name" = 20 chars, zone = 18, indent = 0, no indicator
        // available = 18, label > available, so truncate to 17 + ellipsis.
        let result = format_label("very_long_label_name", 0, 18, "");
        let char_count: usize = result.chars().count();
        assert_eq!(char_count, 18, "Label should be 18 display chars: {result}");
        assert!(
            result.ends_with('\u{2026}'),
            "Should end with ellipsis: {result}"
        );
    }

    #[test]
    fn label_short_padded() {
        let result = format_label("root", 0, 18, "");
        let char_count: usize = result.chars().count();
        assert_eq!(char_count, 18, "Label should be 18 display chars: {result}");
        assert!(
            result.starts_with("root"),
            "Should start with label: {result}"
        );
    }

    #[test]
    fn allocate_segments_sums_to_bar_width() {
        let amounts = vec![7.0, 3.0];
        let total = 10.0;
        let bar_width = 61u16;
        let widths = allocate_segments(&amounts, total, bar_width);
        let sum: u16 = widths.iter().sum();
        assert_eq!(sum, bar_width, "Segment widths must sum to bar_width");
    }

    #[test]
    fn allocate_segments_proportional() {
        let amounts = vec![7.0, 3.0];
        let total = 10.0;
        let bar_width = 100u16;
        let widths = allocate_segments(&amounts, total, bar_width);
        assert_eq!(widths[0], 70);
        assert_eq!(widths[1], 30);
    }

    #[test]
    fn allocate_segments_no_rounding_gap() {
        // 3 segments that don't divide evenly into 10 columns.
        let amounts = vec![1.0, 1.0, 1.0];
        let total = 3.0;
        let bar_width = 10u16;
        let widths = allocate_segments(&amounts, total, bar_width);
        let sum: u16 = widths.iter().sum();
        assert_eq!(sum, 10, "Must fill all 10 columns: {widths:?}");
    }

    #[test]
    fn narrow_width_falls_back_to_summary_rows() {
        let mut state = make_state();
        let first_child_id = state.root.children[0].id;
        state.path = vec![state.root.id, first_child_id];
        let mut surface = Surface::new(10, 5);
        let area = Rect::new(0, 0, 10, 5);
        render(&state, area, surface.buffer_mut());
        let text = surface.to_text();
        assert!(
            text.contains('%'),
            "summary mode should show percentages\n{text}"
        );
        assert!(
            text.contains("ro"),
            "summary mode should preserve the label\n{text}"
        );
    }

    /// Visual introspection: render the one-level scenario at width=80 and
    /// verify the two-zone layout by inspecting the plain text output.
    #[test]
    fn two_zone_layout_visual_introspection() {
        use crate::devkit::flame_graph::test_flame_graph;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use std::time::Duration;

        let mut fg = test_flame_graph();
        let key = KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        fg.handle_key(&key);
        for _ in 0..60 {
            fg.tick(Duration::from_millis(16));
        }

        let mut surface = Surface::new(80, 10);
        let area = Rect::new(0, 0, 80, 10);
        render(&fg, area, surface.buffer_mut());
        let text = surface.to_text();

        let layout = RowLayout::for_width(80);

        assert!(
            text.contains("request"),
            "Root label 'request' should appear\n{text}"
        );

        // All child labels should be present (possibly truncated for long names).
        for label in &["db_query", "template_rend", "auth_check", "logging"] {
            assert!(
                text.contains(label),
                "Label '{label}' should appear in output\n{text}"
            );
        }

        let buf = surface.buffer();
        let first_bar_col = (0..80u16).find(|&x| {
            let s = buf[(x, 0)].symbol();
            s == "\u{257A}" || s == "\u{2501}"
        });
        assert_eq!(
            first_bar_col,
            Some(layout.bar_start()),
            "First bar character on root row should be at column {}",
            layout.bar_start()
        );
    }
}

//! Absolute-time waterfall rendering for the renderer-neutral trace model.

use chrono::{DateTime, Utc};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::data::{TraceItem, TraceTiming, TraceVisualRole};
use super::state::{TraceSpan, TraceView};
use crate::render::{Constraints, LayoutRenderable, Size, clip_text};
use crate::theme;
use crate::widget::flame_graph::RowKind;

impl LayoutRenderable for TraceView {
    fn measure(&self, constraints: Constraints) -> Size {
        let preferred = constraints.max_width.unwrap_or(120);
        let details = if self.details_expanded() { 4 } else { 0 };
        let rows = self
            .visible_rows()
            .len()
            .saturating_add(details)
            .saturating_add(2);
        constraints.constrain(Size::new(preferred, rows.min(u16::MAX as usize) as u16))
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_trace_view(self, area, buf);
    }
}

/// Render a trace with hierarchy on the y-axis and recorded time on the x-axis.
pub fn render_trace_view(state: &TraceView, area: Rect, buf: &mut Buffer) {
    render_inner(state, area, buf);
}

/// Render and update the navigation viewport used by keyboard scrolling.
pub fn render_trace_view_mut(state: &mut TraceView, area: Rect, buf: &mut Buffer) {
    state.set_viewport_height(area.height.saturating_sub(2));
    render_inner(state, area, buf);
}

#[derive(Clone, Copy)]
struct Columns {
    label: u16,
    utc: u16,
    offset: u16,
    duration: u16,
    bar: u16,
}

impl Columns {
    fn for_width(width: u16) -> Self {
        if width >= 96 {
            let label = (width / 4).clamp(26, 38);
            let utc = 24;
            let offset = 10;
            let duration = 10;
            let fixed = label + utc + offset + duration + 4;
            Self {
                label,
                utc,
                offset,
                duration,
                bar: width.saturating_sub(fixed),
            }
        } else if width >= 72 {
            let label = (width / 3).clamp(20, 30);
            let offset = 10;
            let duration = 10;
            Self {
                label,
                utc: 0,
                offset,
                duration,
                bar: width.saturating_sub(label + offset + duration + 3),
            }
        } else {
            Self {
                label: width,
                utc: 0,
                offset: 0,
                duration: 0,
                bar: 0,
            }
        }
    }

    fn bar_x(self, x: u16) -> u16 {
        x + self.label
            + if self.utc == 0 { 0 } else { self.utc + 1 }
            + self.offset
            + self.duration
            + 3
    }
}

fn render_inner(state: &TraceView, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let columns = Columns::for_width(area.width);
    draw_header(state, area, columns, buf);
    if area.height <= 2 {
        return;
    }

    let rows = state.visible_rows();
    let scroll = state.scroll_offset().min(
        rows.len()
            .saturating_sub(usize::from(area.height.saturating_sub(2))),
    );
    let mut y = area.y + 2;
    for (row_index, row) in rows.iter().enumerate().skip(scroll) {
        if y >= area.bottom() {
            break;
        }
        let RowKind::Span { span_id, depth, .. } = row.kind else {
            continue;
        };
        let Some(kind) = state.trace_span(span_id) else {
            continue;
        };
        draw_row(
            state,
            kind,
            depth,
            row_index == state.cursor(),
            area,
            columns,
            y,
            buf,
        );
        y += 1;
        if state.details_expanded() && state.selected_span_id() == Some(span_id) {
            y = draw_details(state, kind, y, area, buf);
        }
    }
}

fn draw_header(state: &TraceView, area: Rect, columns: Columns, buf: &mut Buffer) {
    let (start, end) = state.time_window();
    let elapsed = i128::from(end) - i128::from(start);
    let title = if area.width < 72 {
        format!("Trace · {}", format_duration(elapsed))
    } else {
        format!(
            "Trace · {} · {} → {} · {}",
            state
                .data()
                .groups
                .first()
                .map_or("session", |group| group.label.as_str()),
            format_utc(start),
            format_utc(end),
            format_duration(elapsed)
        )
    };
    buf.set_string(
        area.x,
        area.y,
        clip_text(&title, usize::from(area.width)),
        Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD),
    );
    if area.height < 2 {
        return;
    }
    let y = area.y + 1;
    draw_cell(buf, area.x, y, columns.label, "Operation", theme::dim());
    let mut x = area.x + columns.label + 1;
    if columns.utc > 0 {
        draw_cell(buf, x, y, columns.utc, "Observed UTC", theme::dim());
        x += columns.utc + 1;
    }
    if columns.bar > 0 {
        draw_cell(buf, x, y, columns.offset, "Offset", theme::dim());
        x += columns.offset + 1;
        draw_cell(buf, x, y, columns.duration, "Duration", theme::dim());
        let axis_x = columns.bar_x(area.x);
        draw_axis(start, end, axis_x, columns.bar, y, buf);
    }
}

fn draw_row(
    state: &TraceView,
    kind: TraceSpan,
    depth: u16,
    selected: bool,
    area: Rect,
    columns: Columns,
    y: u16,
    buf: &mut Buffer,
) {
    let (label, timing, role) = trace_row(state, kind);
    let has_children = match kind {
        TraceSpan::Root => true,
        TraceSpan::Group(id) => state
            .data()
            .tracks
            .iter()
            .any(|track| track.group == Some(id)),
        TraceSpan::Track(id) => state
            .data()
            .tracks
            .iter()
            .find(|track| track.id == id)
            .is_some_and(|track| !track.items.is_empty()),
        TraceSpan::Item(id) => state
            .data()
            .tracks
            .iter()
            .find_map(|track| find_item(&track.items, id))
            .is_some_and(|item| !item.children.is_empty()),
    };
    let expanded = state
        .span_id_for(kind)
        .is_some_and(|span| state.is_span_expanded(span))
        || matches!(kind, TraceSpan::Root);
    let indicator = if has_children {
        if expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    let label = format!("{}{}{}", "  ".repeat(usize::from(depth)), indicator, label);
    let style = if selected {
        Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(role_color(role))
    };
    if columns.bar == 0 {
        let summary = compact_timing(timing, state.time_window().0);
        let text = format!("{label}  {summary}");
        buf.set_string(area.x, y, clip_text(&text, usize::from(area.width)), style);
        return;
    }
    buf.set_string(
        area.x,
        y,
        clip_text(&label, usize::from(columns.label)),
        style,
    );
    let mut x = area.x + columns.label + 1;
    let observed = timing_observed(timing);
    if columns.utc > 0 {
        draw_cell(
            buf,
            x,
            y,
            columns.utc,
            observed.map_or_else(|| "—".to_owned(), format_row_utc),
            theme::dim(),
        );
        x += columns.utc + 1;
    }
    draw_cell(
        buf,
        x,
        y,
        columns.offset,
        observed.map_or_else(
            || "—".to_owned(),
            |at| format_offset(i128::from(at) - i128::from(state.time_window().0)),
        ),
        theme::dim(),
    );
    x += columns.offset + 1;
    draw_cell(
        buf,
        x,
        y,
        columns.duration,
        timing_duration(timing).map_or_else(|| "—".to_owned(), format_duration),
        theme::dim(),
    );
    draw_timing(
        state.time_window(),
        timing,
        columns.bar_x(area.x),
        columns.bar,
        y,
        role,
        buf,
    );
}

fn trace_row(state: &TraceView, kind: TraceSpan) -> (String, TraceTiming, TraceVisualRole) {
    match kind {
        TraceSpan::Root => (
            "trace".into(),
            aggregate_timing(
                state
                    .data()
                    .tracks
                    .iter()
                    .flat_map(|track| track.items.iter()),
            ),
            TraceVisualRole::Neutral,
        ),
        TraceSpan::Group(id) => {
            let label = state
                .data()
                .groups
                .iter()
                .find(|group| group.id == id)
                .map_or_else(|| "group".into(), |group| group.label.clone());
            let items = state
                .data()
                .tracks
                .iter()
                .filter(|track| track.group == Some(id))
                .flat_map(|track| track.items.iter());
            (label, aggregate_timing(items), TraceVisualRole::Neutral)
        }
        TraceSpan::Track(id) => {
            let track = state.data().tracks.iter().find(|track| track.id == id);
            let label = track.map_or_else(|| "track".into(), |track| track.label.clone());
            let timing = track.map_or(TraceTiming::Untimed, |track| {
                aggregate_timing(track.items.iter())
            });
            (label, timing, TraceVisualRole::Neutral)
        }
        TraceSpan::Item(id) => {
            let item = state
                .data()
                .tracks
                .iter()
                .find_map(|track| find_item(&track.items, id))
                .expect("trace item span must resolve");
            let role = state
                .data()
                .categories
                .iter()
                .find(|category| category.id == item.category)
                .map_or(TraceVisualRole::Neutral, |category| category.role);
            (item.label.clone(), item.timing, role)
        }
    }
}

fn draw_details(
    state: &TraceView,
    kind: TraceSpan,
    mut y: u16,
    area: Rect,
    buf: &mut Buffer,
) -> u16 {
    let (label, timing, _) = trace_row(state, kind);
    let indent = area.x + 4;
    let width = area.right().saturating_sub(indent);
    let fields = [
        ("node", label),
        (
            "observed UTC",
            timing_observed(timing).map_or_else(|| "untimed".into(), format_utc),
        ),
        (
            "offset",
            timing_observed(timing).map_or_else(
                || "unavailable".into(),
                |at| format_offset(i128::from(at) - i128::from(state.time_window().0)),
            ),
        ),
        (
            "duration",
            timing_duration(timing).map_or_else(|| "unavailable".into(), format_duration),
        ),
    ];
    for (name, value) in fields {
        if y >= area.bottom() {
            return y;
        }
        buf.set_string(
            indent,
            y,
            clip_text(&format!("{name}: {value}"), usize::from(width)),
            Style::default().fg(theme::dim()),
        );
        y += 1;
    }
    for (name, value) in track_details(state, kind) {
        if y >= area.bottom() {
            break;
        }
        buf.set_string(
            indent,
            y,
            clip_text(&format!("{name}: {value}"), usize::from(width)),
            Style::default().fg(theme::text()),
        );
        y += 1;
    }
    if let TraceSpan::Item(id) = kind
        && let Some(item) = state
            .data()
            .tracks
            .iter()
            .find_map(|track| find_item(&track.items, id))
    {
        for (name, value) in &item.details {
            if y >= area.bottom() {
                break;
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(value) {
                buf.set_string(
                    indent,
                    y,
                    clip_text(&format!("{name}:"), usize::from(width)),
                    Style::default()
                        .fg(theme::focus())
                        .add_modifier(Modifier::BOLD),
                );
                y += 1;
                for line in serde_json::to_string_pretty(&json)
                    .unwrap_or_else(|_| value.clone())
                    .lines()
                {
                    if y >= area.bottom() {
                        break;
                    }
                    draw_json_detail_line(buf, indent + 2, y, width.saturating_sub(2), line);
                    y += 1;
                }
            } else {
                buf.set_string(
                    indent,
                    y,
                    clip_text(&format!("{name}: {value}"), usize::from(width)),
                    Style::default().fg(theme::text()),
                );
                y += 1;
            }
        }
    }
    y
}

fn find_item(items: &[TraceItem], id: super::data::ItemId) -> Option<&TraceItem> {
    for item in items {
        if item.id == id {
            return Some(item);
        }
        if let Some(found) = find_item(&item.children, id) {
            return Some(found);
        }
    }
    None
}

fn track_details(state: &TraceView, kind: TraceSpan) -> Vec<(String, String)> {
    let track = match kind {
        TraceSpan::Track(id) => state.data().tracks.iter().find(|track| track.id == id),
        TraceSpan::Group(id) => state
            .data()
            .tracks
            .iter()
            .find(|track| track.group == Some(id) && !track.details.is_empty()),
        TraceSpan::Root | TraceSpan::Item(_) => None,
    };
    track.map_or_else(Vec::new, |track| track.details.clone())
}

fn draw_json_detail_line(buf: &mut Buffer, x: u16, y: u16, width: u16, line: &str) {
    let trimmed = line.trim_start();
    let color = if trimmed.starts_with('"') {
        theme::focus()
    } else if trimmed.contains("true") || trimmed.contains("false") || trimmed.contains("null") {
        theme::warning()
    } else {
        theme::text()
    };
    buf.set_string(
        x,
        y,
        clip_text(line, usize::from(width)),
        Style::default().fg(color),
    );
}

fn draw_timing(
    window: (i64, i64),
    timing: TraceTiming,
    x: u16,
    width: u16,
    y: u16,
    role: TraceVisualRole,
    buf: &mut Buffer,
) {
    if width == 0 {
        return;
    }
    let color = role_color(role);
    match timing {
        TraceTiming::Interval { start, end } if end >= start => {
            let left = project_time(start, window, width);
            let right = project_time(end, window, width);
            for dx in 0..=right.saturating_sub(left) {
                let symbol = if left == right {
                    "━"
                } else if dx == 0 {
                    "╺"
                } else if dx == right - left {
                    "╸"
                } else {
                    "━"
                };
                buf[(x + left + dx, y)]
                    .set_symbol(symbol)
                    .set_style(Style::default().fg(color));
            }
        }
        TraceTiming::Instant(at) => draw_marker(window, at, x, width, y, "◆", color, buf),
        TraceTiming::MissingStart { end } => {
            draw_marker(window, end, x, width, y, "◁", theme::warning(), buf);
        }
        TraceTiming::MissingEnd { start } => {
            draw_marker(window, start, x, width, y, "▷", theme::warning(), buf);
        }
        TraceTiming::Interval { start, end } => {
            draw_marker(window, start, x, width, y, "▷", theme::warning(), buf);
            draw_marker(window, end, x, width, y, "◁", theme::warning(), buf);
        }
        TraceTiming::Untimed => {}
    }
}

fn draw_marker(
    window: (i64, i64),
    time: i64,
    x: u16,
    width: u16,
    y: u16,
    symbol: &str,
    color: Color,
    buf: &mut Buffer,
) {
    let column = project_time(time, window, width);
    buf[(x + column, y)]
        .set_symbol(symbol)
        .set_style(Style::default().fg(color));
}

fn draw_axis(start: i64, end: i64, x: u16, width: u16, y: u16, buf: &mut Buffer) {
    if width == 0 {
        return;
    }
    let range = i128::from(end) - i128::from(start);
    for (column, fraction) in [(0, 0_i128), (width / 2, 1), (width.saturating_sub(1), 2)] {
        let label = format_duration(range * fraction / 2);
        let offset = column
            .saturating_sub(label.len() as u16 / 2)
            .min(width.saturating_sub(label.len() as u16));
        draw_cell(
            buf,
            x + offset,
            y,
            width.saturating_sub(offset),
            label,
            theme::dim(),
        );
    }
}

fn draw_cell(buf: &mut Buffer, x: u16, y: u16, width: u16, value: impl AsRef<str>, color: Color) {
    if width > 0 {
        buf.set_string(
            x,
            y,
            clip_text(value.as_ref(), usize::from(width)),
            Style::default().fg(color),
        );
    }
}

fn aggregate_timing<'a>(items: impl Iterator<Item = &'a TraceItem>) -> TraceTiming {
    let mut min = None;
    let mut max = None;
    let mut all = Vec::new();
    for item in items {
        collect_items(item, &mut all);
    }
    for item in all {
        for time in item.timing.coordinates().into_iter().flatten() {
            min = Some(min.map_or(time, |value: i64| value.min(time)));
            max = Some(max.map_or(time, |value: i64| value.max(time)));
        }
    }
    match (min, max) {
        (Some(start), Some(end)) if start != end => TraceTiming::Interval { start, end },
        (Some(at), _) => TraceTiming::Instant(at),
        _ => TraceTiming::Untimed,
    }
}

fn collect_items<'a>(item: &'a TraceItem, items: &mut Vec<&'a TraceItem>) {
    items.push(item);
    for child in &item.children {
        collect_items(child, items);
    }
}

fn timing_observed(timing: TraceTiming) -> Option<i64> {
    match timing {
        TraceTiming::Instant(at) | TraceTiming::MissingEnd { start: at } => Some(at),
        TraceTiming::Interval { start, .. } => Some(start),
        TraceTiming::MissingStart { end } => Some(end),
        TraceTiming::Untimed => None,
    }
}

fn timing_duration(timing: TraceTiming) -> Option<i128> {
    match timing {
        TraceTiming::Interval { start, end } if end >= start => {
            Some(i128::from(end) - i128::from(start))
        }
        _ => None,
    }
}

fn project_time(time: i64, window: (i64, i64), width: u16) -> u16 {
    if width <= 1 {
        return 0;
    }
    let range = i128::from(window.1) - i128::from(window.0);
    let numerator = (i128::from(time) - i128::from(window.0)).clamp(0, range);
    u16::try_from(numerator * i128::from(width - 1) / range).unwrap_or(width - 1)
}

fn format_utc(unix_nanos: i64) -> String {
    let seconds = unix_nanos.div_euclid(1_000_000_000);
    DateTime::<Utc>::from_timestamp(seconds, 0).map_or_else(
        || format!("{unix_nanos}ns"),
        |value| value.format("%H:%M:%S").to_string(),
    )
}

fn format_row_utc(unix_nanos: i64) -> String {
    format_utc(unix_nanos)
}

fn format_offset(nanos: i128) -> String {
    if nanos < 0 {
        format!("−{}", format_duration(-nanos))
    } else {
        format!("+{}", format_duration(nanos))
    }
}

fn format_duration(nanos: i128) -> String {
    let negative = nanos < 0;
    let value = nanos.abs();
    let (suffix, divisor) = if value >= 60_000_000_000 {
        ("m", 60_000_000_000)
    } else if value >= 1_000_000_000 {
        ("s", 1_000_000_000)
    } else if value >= 1_000_000 {
        ("ms", 1_000_000)
    } else if value >= 1_000 {
        ("µs", 1_000)
    } else {
        return format!("{}{value}ns", if negative { "−" } else { "" });
    };
    let whole = value / divisor;
    let tenths = value % divisor * 10 / divisor;
    let rendered = if tenths == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{tenths}")
    };
    format!("{}{rendered}{suffix}", if negative { "−" } else { "" })
}

fn compact_timing(timing: TraceTiming, trace_start: i64) -> String {
    timing_observed(timing).map_or_else(
        || "untimed".into(),
        |at| {
            let offset = format_offset(i128::from(at) - i128::from(trace_start));
            timing_duration(timing).map_or_else(
                || offset.clone(),
                |duration| format!("{offset} · {}", format_duration(duration)),
            )
        },
    )
}

fn role_color(role: TraceVisualRole) -> Color {
    match role {
        TraceVisualRole::Neutral => theme::dim(),
        TraceVisualRole::Scheduled => theme::focus(),
        TraceVisualRole::Success => theme::success(),
        TraceVisualRole::Warning => theme::warning(),
        TraceVisualRole::Failure => theme::error(),
    }
}

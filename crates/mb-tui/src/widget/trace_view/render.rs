//! Clipped terminal rendering, with explicit uncertainty and overflow summaries.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::data::{ItemId, TraceItem, TraceTiming};
use super::layout::{TimeWindow, ViewLayout};
use super::state::{Row, TraceView};
use crate::render::{Constraints, LayoutRenderable, Size, ellipsize_text, summarize_text};
use crate::theme;

impl LayoutRenderable for TraceView {
    /// Measure responsive rows and chrome using the same layout as rendering.
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.constrain(Size::new(80, 0)).width;
        if width == 0 || constraints.max_height == Some(0) {
            return Size::ZERO;
        }
        let preferred = (self.rows.len().max(1) + 2 + if self.show_details { 4 } else { 0 })
            .min(u16::MAX as usize) as u16;
        constraints.constrain(Size::new(width, preferred))
    }

    /// Paint without caching dimensions; selection is still visible after resize.
    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_trace_view(self, area, buf);
    }
}

/// Paint a trace inside `area`, also clipping against the buffer's actual bounds.
///
/// Positive intervals draw `[━━)`; instants `│`; zero-length pairs `◇`;
/// missing starts `◁`; missing ends `▷`; reversed endpoints `!` without a bar.
/// Crowded cells draw `+`. Every underlying item remains selectable with Tab.
/// Untimed observations appear as text rows, outside the time axis.
pub fn render_trace_view(state: &TraceView, area: Rect, buf: &mut Buffer) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].reset();
        }
    }
    let layout = ViewLayout::new(area.width, area.height, state.show_details);
    let scroll = state.effective_scroll(layout.body);
    if layout.header > 0 {
        draw_axis(state, Rect::new(area.x, area.y, area.width, 1), buf);
    }
    let mut crowded = 0;
    for (offset, row) in state
        .rows
        .iter()
        .skip(scroll)
        .take(usize::from(layout.body))
        .enumerate()
    {
        let row_area = Rect::new(
            area.x,
            area.y + layout.header + offset as u16,
            area.width,
            1,
        );
        crowded += draw_row(
            state,
            *row,
            scroll + offset == state.cursor,
            row_area,
            layout,
            buf,
        );
    }
    if state.rows.is_empty() {
        draw_text(
            "No trace observations",
            Rect::new(area.x, area.y + layout.header, area.width, 1),
            normal(),
            buf,
        );
    }
    let details_y = area.y + layout.header + layout.body;
    draw_details(
        state,
        Rect::new(area.x, details_y, area.width, layout.details),
        buf,
    );
    if layout.footer > 0 {
        let below = state
            .rows
            .len()
            .saturating_sub(scroll + usize::from(layout.body));
        let footer = format!(
            "↑{scroll} ↓{below} | {crowded} crowded cells | j/k rows Tab items h/l pan +/- zoom"
        );
        draw_text(
            &footer,
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
            normal(),
            buf,
        );
    }
}

/// Paint and cache content height for subsequent scrolling and page navigation.
pub fn render_trace_view_mut(state: &mut TraceView, area: Rect, buf: &mut Buffer) {
    let area = area.intersection(buf.area);
    let layout = ViewLayout::new(area.width, area.height, state.show_details);
    state.set_viewport_height(layout.body);
    render_trace_view(state, area, buf);
}

/// Show exact integer viewport coordinates and explicit units.
fn draw_axis(state: &TraceView, area: Rect, buf: &mut Buffer) {
    let timed = state
        .data
        .tracks
        .iter()
        .flat_map(|track| &track.items)
        .any(|item| item.timing.coordinates()[0].is_some());
    let range = if timed {
        format!("{}..{} ", state.window.start, state.window.end)
    } else {
        "No timed observations ".into()
    };
    let text = summarize_text(
        "",
        &range,
        &state.data.time_unit.to_string(),
        usize::from(area.width),
    );
    draw_text(&text, area, normal(), buf);
}

/// Render one group or lane, returning the number of crowded terminal cells.
fn draw_row(
    state: &TraceView,
    row: Row,
    selected: bool,
    area: Rect,
    layout: ViewLayout,
    buf: &mut Buffer,
) -> usize {
    let style = if selected { selected_style() } else { normal() };
    match row {
        Row::Group(index) => {
            let group = &state.data.groups[index];
            let marker = if state.is_group_collapsed(group.id) {
                "▸"
            } else {
                "▾"
            };
            let (tracks, items) = state.group_counts(group.id);
            draw_text(
                &format!("{marker} {} ({tracks} tracks / {items} items)", group.label),
                area,
                style,
                buf,
            );
            0
        }
        Row::Lane { track, lane } => {
            let track_data = &state.data.tracks[track];
            let lane_data = &state.lanes[track][lane];
            if layout.labels == 0 || !lane_data.timed {
                draw_summary(state, track, lane, selected, area, buf);
                return 0;
            }
            let marker = if selected { "▶" } else { " " };
            let label = format!("{marker} {} [{}]", track_data.label, lane + 1);
            draw_text(
                &label,
                Rect::new(area.x, area.y, layout.labels, 1),
                style,
                buf,
            );
            let axis = Rect::new(
                area.x + layout.labels + 1,
                area.y,
                area.width - layout.labels - 1,
                1,
            );
            draw_lane(state, track, lane, selected, axis, buf)
        }
    }
}

/// Retain time meaning and selection in text-only and untimed rows.
fn draw_summary(
    state: &TraceView,
    track: usize,
    lane: usize,
    selected: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let track_data = &state.data.tracks[track];
    let lane_data = &state.lanes[track][lane];
    let item = if selected {
        state.selected_item()
    } else {
        lane_data.items.first().map(|&item| &track_data.items[item])
    };
    let marker = if selected { "▶" } else { " " };
    let text = match item {
        Some(item) => format!(
            "{marker} {} {} | {} | {} ({} items)",
            timing_marker(item.timing),
            item.label,
            item.timing.description(state.data.time_unit),
            track_data.label,
            lane_data.items.len()
        ),
        None => format!("{marker} {} (empty)", track_data.label),
    };
    draw_text(
        &text,
        area,
        if selected { selected_style() } else { normal() },
        buf,
    );
}

/// One projected cell retains collision and selection information until painting.
#[derive(Clone, Copy, Default)]
struct Mark {
    symbol: &'static str,
    item: Option<ItemId>,
    crowded: bool,
    selected: bool,
    warning: bool,
}

/// Paint a lane through a width-bounded collision buffer, then add bar labels.
fn draw_lane(
    state: &TraceView,
    track: usize,
    lane: usize,
    selected: bool,
    area: Rect,
    buf: &mut Buffer,
) -> usize {
    let mut marks = vec![Mark::default(); usize::from(area.width)];
    let items = &state.data.tracks[track].items;
    let selected_id = if selected {
        state.selected_item().map(|item| item.id)
    } else {
        None
    };
    for &index in &state.lanes[track][lane].items {
        plot_item(&items[index], selected_id, state.window, &mut marks);
    }
    for (column, mark) in marks.iter().enumerate() {
        if mark.item.is_none() {
            continue;
        }
        let style = if mark.selected {
            selected_style()
        } else if mark.warning {
            Style::default().fg(theme::warning())
        } else {
            Style::default().fg(theme::focus())
        };
        let symbol = if mark.crowded { "+" } else { mark.symbol };
        buf[(area.x + column as u16, area.y)]
            .set_symbol(symbol)
            .set_style(style);
    }
    for &index in &state.lanes[track][lane].items {
        draw_bar_label(&items[index], selected_id, state.window, &marks, area, buf);
    }
    marks.iter().filter(|mark| mark.crowded).count()
}

/// Project only recorded geometry; reversed and missing intervals get no fill.
fn plot_item(item: &TraceItem, selected: Option<ItemId>, window: TimeWindow, marks: &mut [Mark]) {
    let width = marks.len() as u16;
    if let Some((start, end)) = item.timing.interval() {
        if end < window.start || start > window.end || width == 0 {
            return;
        }
        let left = window
            .column(start.max(window.start), width)
            .expect("clipped start");
        let right = window
            .column(end.min(window.end), width)
            .expect("clipped end");
        for column in left..=right {
            let symbol = if column == left && start < window.start {
                "‹"
            } else if column == right && end > window.end {
                "›"
            } else if column == left {
                "["
            } else if column == right {
                ")"
            } else {
                "━"
            };
            add_mark(&mut marks[usize::from(column)], item, selected, symbol);
        }
    } else {
        let mut previous = None;
        for time in item.timing.coordinates().into_iter().flatten() {
            if let Some(column) = window.column(time, width) {
                if previous != Some(column) {
                    add_mark(
                        &mut marks[usize::from(column)],
                        item,
                        selected,
                        timing_marker(item.timing),
                    );
                }
                previous = Some(column);
            }
        }
    }
}

/// Merge a mark without allowing later items to erase a selected collision.
fn add_mark(mark: &mut Mark, item: &TraceItem, selected: Option<ItemId>, symbol: &'static str) {
    mark.crowded |= mark.item.is_some_and(|id| id != item.id);
    mark.item = Some(item.id);
    mark.symbol = symbol;
    mark.selected |= selected == Some(item.id);
    mark.warning |= matches!(item.timing, TraceTiming::Interval { start, end } if start > end)
        || matches!(
            item.timing,
            TraceTiming::MissingStart { .. } | TraceTiming::MissingEnd { .. }
        );
}

/// Place labels only inside uncontested positive bars, preserving both endpoints.
fn draw_bar_label(
    item: &TraceItem,
    selected: Option<ItemId>,
    window: TimeWindow,
    marks: &[Mark],
    area: Rect,
    buf: &mut Buffer,
) {
    let Some((start, end)) = item.timing.interval() else {
        return;
    };
    let Some(left) = window.column(start.max(window.start), area.width) else {
        return;
    };
    let Some(right) = window.column(end.min(window.end), area.width) else {
        return;
    };
    if right.saturating_sub(left) <= 3 {
        return;
    }
    let interior = usize::from(left + 1)..usize::from(right);
    if marks[interior]
        .iter()
        .any(|mark| mark.crowded || mark.item != Some(item.id))
    {
        return;
    }
    let style = if selected == Some(item.id) {
        selected_style()
    } else {
        normal()
    };
    draw_text(
        &item.label,
        Rect::new(area.x + left + 1, area.y, right - left - 1, 1),
        style,
        buf,
    );
}

/// Distinguish observations from missing endpoints and regressed pairs in text.
fn timing_marker(timing: TraceTiming) -> &'static str {
    match timing {
        TraceTiming::Instant(_) => "│",
        TraceTiming::Interval { start, end } if start > end => "!",
        TraceTiming::Interval { start, end } if start == end => "◇",
        TraceTiming::Interval { .. } => "[)",
        TraceTiming::MissingStart { .. } => "◁",
        TraceTiming::MissingEnd { .. } => "▷",
        TraceTiming::Untimed => "?",
    }
}

/// Show bounded selected-item details and count any lines that cannot fit.
fn draw_details(state: &TraceView, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    let Some(item) = state.selected_item() else {
        return;
    };
    let total = item.details.len() + 2;
    for line in 0..usize::from(area.height).min(total) {
        let text = if line >= 2
            && line + 1 == usize::from(area.height)
            && total > usize::from(area.height)
        {
            format!("… +{} detail lines", total - line)
        } else if line == 0 {
            let title = safe_text(&format!("{} [item {}]", item.label, item.id.0));
            let suffix = if area.height == 2 && !item.details.is_empty() {
                format!(" +{} fields", item.details.len())
            } else {
                String::new()
            };
            summarize_text("", &title, &suffix, usize::from(area.width))
        } else if line == 1 {
            item.timing.description(state.data.time_unit)
        } else {
            let (key, value) = &item.details[line - 2];
            format!("{key}: {value}")
        };
        draw_text(
            &text,
            Rect::new(area.x, area.y + line as u16, area.width, 1),
            normal(),
            buf,
        );
    }
}

/// Normalize terminal controls, then use the shared Unicode-width overflow rule.
fn draw_text(text: &str, area: Rect, style: Style, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    let text = ellipsize_text(&safe_text(text), usize::from(area.width));
    buf.set_stringn(area.x, area.y, text, usize::from(area.width), style);
}

/// Normalize controls before width-sensitive formatting can lose a suffix.
fn safe_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// Use the shared adaptive palette for regular trace text.
fn normal() -> Style {
    Style::default().fg(theme::dim())
}

/// Use the shared focus color and weight for selection, including crowded cells.
fn selected_style() -> Style {
    Style::default()
        .fg(theme::text())
        .add_modifier(Modifier::BOLD)
}

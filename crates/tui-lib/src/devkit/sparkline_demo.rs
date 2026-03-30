use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::sparkline::Sparkline;

#[derive(Clone)]
pub struct State {
    pub data: Vec<f64>,
    pub tick_count: u64,
}

pub fn initial_state() -> State {
    State {
        data: Vec::new(),
        tick_count: 0,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Informational text in the upper area.
    let dim = Style::default().fg(theme::dim());
    let text = Style::default().fg(theme::text());

    let lines: &[(&str, Style)] = &[
        ("  Sparkline Demo", text),
        ("", dim),
        (&format!("  Points: {}", state.data.len()), dim),
        ("  Press 'r' to reset", dim),
    ];

    for (i, &(ref t, s)) in lines.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height.saturating_sub(1) {
            break;
        }
        buf.set_stringn(area.x, y, t, area.width as usize, s);
    }

    // Sparkline at the bottom row.
    let bottom = area.y + area.height - 1;
    let spark = Sparkline::new(state.data.clone()).label("tok/s".to_string());
    spark.render(Rect::new(area.x, bottom, area.width, 1), buf);
}

pub fn tick(state: &mut State, _dt: Duration) {
    let v = ((state.tick_count * 7 + 13) % 100) as f64;
    state.data.push(v);
    state.tick_count += 1;
}

pub fn apply(state: &mut State, key: &KeyEvent) {
    if key.code == KeyCode::Char('r') {
        state.data.clear();
        state.tick_count = 0;
    }
}

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::gauge::Gauge;

#[derive(Clone)]
pub struct State {
    pub tick_count: u64,
    pub value: f64,
}

pub fn initial_state() -> State {
    State {
        tick_count: 0,
        value: 0.5,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 6 || area.width == 0 {
        return;
    }

    // Temperature gauge with gradient coloring.
    Gauge::new(state.value)
        .label("Temperature".to_string())
        .low_label("cold".to_string())
        .high_label("hot".to_string())
        .gradient()
        .render(Rect::new(area.x, area.y, area.width, 2), buf);

    // Confidence gauge with fixed color.
    Gauge::new(state.value)
        .label("Confidence".to_string())
        .color(Color::Cyan)
        .render(Rect::new(area.x, area.y + 3, area.width, 1), buf);

    // Value display.
    let info = format!("  Value: {:.2}", state.value);
    if area.y + 5 < area.y + area.height {
        buf.set_stringn(
            area.x,
            area.y + 5,
            &info,
            area.width as usize,
            Style::default().fg(theme::dim()),
        );
    }
}

pub fn tick(state: &mut State, _dt: Duration) {
    state.tick_count += 1;
    state.value = 0.5 + 0.5 * ((state.tick_count as f64 * 0.05).sin());
}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.value = (state.value + 0.1).clamp(0.0, 1.0);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.value = (state.value - 0.1).clamp(0.0, 1.0);
        }
        _ => {}
    }
}

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::range_slider::RangeSlider;

#[derive(Clone)]
pub struct State {
    pub sliders: Vec<RangeSlider>,
    pub active: usize,
}

pub fn initial_state() -> State {
    State {
        sliders: vec![
            RangeSlider::new(0.5)
                .label("Volume".to_string())
                .show_value(true),
            RangeSlider::new(0.4)
                .label("Quality".to_string())
                .steps(5)
                .show_value(true),
        ],
        active: 0,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 5 || area.width == 0 {
        return;
    }
    let labels = ["Volume", "Quality"];
    let dim = Style::default().fg(theme::dim());
    let focus = Style::default().fg(theme::focus());
    for (i, slider) in state.sliders.iter().enumerate() {
        let y = area.y + (i as u16) * 2;
        let marker = if i == state.active { "> " } else { "  " };
        let style = if i == state.active { focus } else { dim };
        buf.set_stringn(
            area.x,
            y,
            format!("{}{}", marker, labels[i]),
            area.width as usize,
            style,
        );
        slider.render(
            Rect::new(area.x + 2, y + 1, area.width.saturating_sub(2), 1),
            buf,
        );
    }
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Tab => state.active = (state.active + 1) % 2,
        KeyCode::Left | KeyCode::Char('h') => state.sliders[state.active].decrement(),
        KeyCode::Right | KeyCode::Char('l') => state.sliders[state.active].increment(),
        _ => {}
    }
}

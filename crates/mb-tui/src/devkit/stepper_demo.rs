use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::stepper::Stepper;

#[derive(Clone)]
pub struct State {
    pub steppers: Vec<Stepper>,
    pub active: usize,
}

pub fn initial_state() -> State {
    State {
        steppers: vec![
            Stepper::new(vec![
                "Low".into(),
                "Medium".into(),
                "High".into(),
                "Ultra".into(),
            ]),
            Stepper::new(vec![
                "10".into(),
                "12".into(),
                "14".into(),
                "16".into(),
                "18".into(),
            ])
            .wrap(false),
        ],
        active: 0,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 5 || area.width == 0 {
        return;
    }
    let labels = ["Detail Level", "Font Size"];
    let dim = Style::default().fg(theme::dim());
    let focus = Style::default().fg(theme::focus());
    for (i, stepper) in state.steppers.iter().enumerate() {
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
        stepper.render(
            Rect::new(area.x + 2, y + 1, area.width.saturating_sub(2), 1),
            buf,
        );
    }
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Tab => state.active = (state.active + 1) % 2,
        KeyCode::Left | KeyCode::Char('h') => state.steppers[state.active].prev(),
        KeyCode::Right | KeyCode::Char('l') => state.steppers[state.active].next(),
        _ => {}
    }
}

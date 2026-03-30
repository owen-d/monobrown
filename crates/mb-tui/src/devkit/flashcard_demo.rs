use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::flashcard::{CardSide, Flashcard};

#[derive(Clone)]
pub struct State {
    pub cards: Vec<Flashcard>,
    pub current: usize,
}

pub fn initial_state() -> State {
    State {
        cards: vec![
            Flashcard::new(
                "What does RAII stand for?".into(),
                "Resource Acquisition Is Initialization".into(),
            ),
            Flashcard::new(
                "What is the borrow checker?".into(),
                "A compile-time system that enforces ownership and borrowing rules".into(),
            ),
            Flashcard::new(
                "What is a trait in Rust?".into(),
                "A collection of methods defined for an unknown type, similar to interfaces".into(),
            ),
        ],
        current: 0,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 4 || area.width == 0 || state.cards.is_empty() {
        return;
    }

    let dim = Style::default().fg(theme::dim());

    // Navigation info at the top.
    let nav = format!(
        "  Card {}/{} ({})",
        state.current + 1,
        state.cards.len(),
        match state.cards[state.current].side() {
            CardSide::Front => "front",
            CardSide::Back => "back",
        },
    );
    buf.set_stringn(area.x, area.y, &nav, area.width as usize, dim);

    // Render current flashcard below the nav line.
    state.cards[state.current].render(
        Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        ),
        buf,
    );
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Char(' ') => state.cards[state.current].flip(),
        KeyCode::Right | KeyCode::Char('l') if state.current + 1 < state.cards.len() => {
            state.current += 1;
            state.cards[state.current].set_side(CardSide::Front);
        }
        KeyCode::Left | KeyCode::Char('h') if state.current > 0 => {
            state.current -= 1;
            state.cards[state.current].set_side(CardSide::Front);
        }
        _ => {}
    }
}

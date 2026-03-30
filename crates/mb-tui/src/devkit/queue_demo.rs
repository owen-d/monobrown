use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::queue_list::QueueList;

#[derive(Clone)]
pub struct State {
    pub queue: QueueList,
    pub add_count: u32,
}

pub fn initial_state() -> State {
    State {
        queue: QueueList::new(vec![
            "Parse config".into(),
            "Lint code".into(),
            "Run tests".into(),
            "Build binary".into(),
            "Deploy".into(),
        ]),
        add_count: 0,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 2 || area.width == 0 {
        return;
    }
    let list_h = area.height.saturating_sub(2);
    state
        .queue
        .render(Rect::new(area.x, area.y, area.width, list_h), buf);
    let info_y = area.y + area.height - 1;
    let dim = Style::default().fg(theme::dim());
    if !state.queue.is_empty() {
        let sel = &state.queue.items()[state.queue.selected()];
        buf.set_stringn(
            area.x,
            info_y,
            format!("  Selected: {sel}"),
            area.width as usize,
            dim,
        );
    }
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => state.queue.next(),
        KeyCode::Up | KeyCode::Char('k') => state.queue.prev(),
        KeyCode::Char('d') if !state.queue.is_empty() => {
            let idx = state.queue.selected();
            state.queue.remove(idx);
        }
        KeyCode::Char('K') => state.queue.move_up(),
        KeyCode::Char('J') => state.queue.move_down(),
        KeyCode::Char('a') => {
            state.add_count += 1;
            state.queue.push(format!("New task {}", state.add_count));
        }
        _ => {}
    }
}

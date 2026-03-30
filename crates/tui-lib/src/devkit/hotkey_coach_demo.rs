use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::hotkey_coach::{HotkeyCoach, Orientation, Shortcut};

#[derive(Clone)]
pub struct State {
    pub orientation: Orientation,
}

pub fn initial_state() -> State {
    State {
        orientation: Orientation::Horizontal,
    }
}

fn make_shortcuts() -> Vec<Shortcut> {
    vec![
        Shortcut::new("j/k", "navigate"),
        Shortcut::new("Space", "flip"),
        Shortcut::new("Tab", "switch"),
        Shortcut::new("q", "quit"),
        Shortcut::new("?", "help"),
    ]
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 2 || area.width == 0 {
        return;
    }

    let dim_style = Style::default().fg(theme::dim());

    // Show current orientation at top.
    let label = match state.orientation {
        Orientation::Horizontal => "Orientation: Horizontal (press 'o' to toggle)",
        Orientation::Vertical => "Orientation: Vertical (press 'o' to toggle)",
    };
    buf.set_stringn(area.x, area.y, label, area.width as usize, dim_style);

    match state.orientation {
        Orientation::Horizontal => {
            // Render horizontal coach at the bottom of the area.
            let coach = HotkeyCoach::new(make_shortcuts()).orientation(Orientation::Horizontal);
            let bottom_y = area.y + area.height - 1;
            let coach_area = Rect::new(area.x, bottom_y, area.width, 1);
            coach.render(coach_area, buf);
        }
        Orientation::Vertical => {
            // Render vertical coach in the main area.
            let coach = HotkeyCoach::new(make_shortcuts()).orientation(Orientation::Vertical);
            let coach_area = Rect::new(
                area.x,
                area.y + 2,
                area.width,
                area.height.saturating_sub(2),
            );
            coach.render(coach_area, buf);
        }
    }
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    if key.code == KeyCode::Char('o') {
        state.orientation = match state.orientation {
            Orientation::Horizontal => Orientation::Vertical,
            Orientation::Vertical => Orientation::Horizontal,
        };
    }
}

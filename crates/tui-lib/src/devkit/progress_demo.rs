use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::eta_display::EtaDisplay;
use crate::widget::progress_bar::ProgressBar;

const LABELS: [&str; 3] = ["Build ", "Test  ", "Deploy"];
const RATES: [f64; 3] = [0.003, 0.002, 0.001];

#[derive(Clone)]
pub struct State {
    pub progress: Vec<f64>,
    pub elapsed_ms: u64,
}

pub fn initial_state() -> State {
    State {
        progress: vec![0.0; 3],
        elapsed_ms: 0,
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // gap
        Constraint::Length(1), // bar 0
        Constraint::Length(1), // bar 1
        Constraint::Length(1), // bar 2
        Constraint::Length(1), // gap
        Constraint::Length(1), // eta
        Constraint::Min(0),    // spacer
        Constraint::Length(1), // instructions
    ])
    .split(area);

    // Title.
    let title = Line::from(Span::styled(
        " Progress Bar Demo",
        Style::default().fg(theme::focus()),
    ));
    buf.set_line(rows[0].x, rows[0].y, &title, rows[0].width);

    // Three progress bars.
    for i in 0..3 {
        let mut bar = ProgressBar::new(state.progress[i]).label(LABELS[i].to_string());
        if i == 1 {
            let forecast = (state.progress[i] + 0.15).min(1.0);
            bar = bar.forecast(forecast);
        }
        bar.render(rows[2 + i], buf);
    }

    // ETA display based on progress[0].
    let eta = if state.progress[0] >= 1.0 {
        EtaDisplay::done()
    } else if state.progress[0] > 0.0 {
        let fraction_remaining = 1.0 - state.progress[0];
        let elapsed_secs = state.elapsed_ms as f64 / 1000.0;
        let rate = state.progress[0] / elapsed_secs;
        let remaining_secs = fraction_remaining / rate;
        EtaDisplay::new(Duration::from_secs_f64(remaining_secs))
    } else {
        EtaDisplay::unknown()
    };
    eta.label("ETA".to_string()).render(rows[6], buf);

    // Instructions.
    let help = Line::from(Span::styled(
        " 'r' reset | 'f' fast-forward",
        Style::default().fg(theme::dim()),
    ));
    buf.set_line(rows[8].x, rows[8].y, &help, rows[8].width);
}

pub fn tick(state: &mut State, dt: Duration) {
    let ms = dt.as_millis() as u64;
    state.elapsed_ms += ms;
    let t = dt.as_secs_f64() / 0.05; // normalize: 50ms tick = 1.0 multiplier
    for (i, rate) in RATES.iter().enumerate() {
        state.progress[i] = (state.progress[i] + rate * t).min(1.0);
    }
}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Char('r') => {
            state.progress = vec![0.0; 3];
            state.elapsed_ms = 0;
        }
        KeyCode::Char('f') => {
            for p in &mut state.progress {
                *p = (*p + 0.1).min(1.0);
            }
        }
        _ => {}
    }
}

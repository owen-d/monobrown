use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::{Constraints, GridRenderable, LayoutRenderable, Size, heatmap_style};
use crate::theme;

#[derive(Clone)]
pub struct AgentInfo {
    pub name: &'static str,
    pub task: &'static str,
    pub progress: f64,
    pub tokens: u32,
}

#[derive(Clone)]
pub struct State {
    pub agents: Vec<AgentInfo>,
    pub selected: usize,
}

pub fn initial_state() -> State {
    State {
        agents: vec![
            AgentInfo {
                name: "Parser",
                task: "Analyzing syntax tree",
                progress: 0.8,
                tokens: 1200,
            },
            AgentInfo {
                name: "Linter",
                task: "Checking style rules",
                progress: 0.45,
                tokens: 800,
            },
            AgentInfo {
                name: "Builder",
                task: "Compiling modules",
                progress: 0.2,
                tokens: 2400,
            },
            AgentInfo {
                name: "Tester",
                task: "Running test suite",
                progress: 0.6,
                tokens: 1600,
            },
        ],
        selected: 0,
    }
}

struct AgentCard<'a> {
    info: &'a AgentInfo,
    selected: bool,
}

impl LayoutRenderable for AgentCard<'_> {
    fn measure(&self, constraints: Constraints) -> Size {
        Size::new(constraints.fill_width(), 5)
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height < 5 || area.width < 10 {
            return;
        }
        let w = area.width as usize;
        let bs = if self.selected {
            Style::default().fg(theme::focus())
        } else {
            Style::default().fg(theme::border())
        };
        let ds = Style::default().fg(theme::dim());
        render_card_header(buf, area, w, self.info.name, bs);
        render_text_row(
            buf,
            area.x,
            area.y + 1,
            w,
            self.info.task,
            bs,
            Style::default().fg(theme::text()),
        );
        render_progress_row(
            buf,
            area.x,
            area.y + 2,
            area.width,
            w,
            self.info.progress,
            bs,
            ds,
        );
        render_text_row(
            buf,
            area.x,
            area.y + 3,
            w,
            &format!(" {} tok", self.info.tokens),
            bs,
            ds,
        );
        render_card_bottom(buf, area.x, area.y + 4, w, bs);
    }
}

fn render_card_header(buf: &mut Buffer, area: Rect, w: usize, name: &str, bs: Style) {
    let label = format!(" {name} ");
    let rem = w.saturating_sub(2 + label.len());
    buf.set_stringn(
        area.x,
        area.y,
        format!(
            "\u{250c}\u{2500}{}{}\u{2510}",
            label,
            "\u{2500}".repeat(rem)
        ),
        w,
        bs,
    );
}

fn render_text_row(buf: &mut Buffer, x: u16, y: u16, w: usize, text: &str, bs: Style, ts: Style) {
    buf.set_stringn(x, y, "\u{2502}", 1, bs);
    buf.set_stringn(
        x + 1,
        y,
        format!(" {:<width$}", text, width = w.saturating_sub(3)),
        w.saturating_sub(2),
        ts,
    );
    buf.set_stringn(x + (w as u16).saturating_sub(1), y, "\u{2502}", 1, bs);
}

#[allow(clippy::too_many_arguments)]
fn render_progress_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    w: usize,
    progress: f64,
    bs: Style,
    ds: Style,
) {
    buf.set_stringn(x, y, "\u{2502}", 1, bs);
    let bar_w = w.saturating_sub(8);
    let filled = (progress * bar_w as f64).round() as usize;
    let bar_s = heatmap_style(progress);
    for i in 0..filled.min(bar_w) {
        buf.set_stringn(x + 2 + i as u16, y, "\u{2501}", 1, bar_s);
    }
    for i in filled..bar_w {
        buf.set_stringn(x + 2 + i as u16, y, "\u{2500}", 1, ds);
    }
    let pct = format!("{:>3}%", (progress * 100.0).round() as u32);
    let px = x + 2 + bar_w as u16 + 1;
    if px + 4 < x + width {
        buf.set_stringn(px, y, &pct, 4, ds);
    }
    buf.set_stringn(x + width.saturating_sub(1), y, "\u{2502}", 1, bs);
}

fn render_card_bottom(buf: &mut Buffer, x: u16, y: u16, w: usize, bs: Style) {
    buf.set_stringn(
        x,
        y,
        format!("\u{2514}{}\u{2518}", "\u{2500}".repeat(w.saturating_sub(2))),
        w,
        bs,
    );
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height < 2 || area.width == 0 {
        return;
    }
    let content = Rect::new(area.x, area.y, area.width, area.height - 1);
    buf.set_stringn(
        area.x,
        area.y + area.height - 1,
        " j/k select  +/- adjust progress",
        area.width as usize,
        Style::default().fg(theme::dim()),
    );
    let mut grid = GridRenderable::new(28).gap(1, 1);
    for (i, agent) in state.agents.iter().enumerate() {
        grid.push(AgentCard {
            info: agent,
            selected: i == state.selected,
        });
    }
    grid.render(content, buf);
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    let len = state.agents.len();
    if len == 0 {
        return;
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(len - 1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Char('+') | KeyCode::Right => {
            state.agents[state.selected].progress =
                (state.agents[state.selected].progress + 0.1).clamp(0.0, 1.0);
        }
        KeyCode::Char('-') | KeyCode::Left => {
            state.agents[state.selected].progress =
                (state.agents[state.selected].progress - 0.1).clamp(0.0, 1.0);
        }
        _ => {}
    }
}

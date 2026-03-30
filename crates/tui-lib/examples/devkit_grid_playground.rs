use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use tui_lib::devkit::playground;
use tui_lib::render::{Constraints, GridRenderable, LayoutRenderable, Size};

// ---------------------------------------------------------------------------
// Card data
// ---------------------------------------------------------------------------

struct AgentCard {
    name: &'static str,
    task: &'static str,
    progress: u8,
}

const AGENTS: [AgentCard; 8] = [
    AgentCard {
        name: "Parser",
        task: "Tokenizing input",
        progress: 85,
    },
    AgentCard {
        name: "Linter",
        task: "Checking style rules",
        progress: 42,
    },
    AgentCard {
        name: "Builder",
        task: "Compiling crate",
        progress: 67,
    },
    AgentCard {
        name: "Tester",
        task: "Running suite",
        progress: 23,
    },
    AgentCard {
        name: "Deployer",
        task: "Staging release",
        progress: 91,
    },
    AgentCard {
        name: "Monitor",
        task: "Watching metrics",
        progress: 55,
    },
    AgentCard {
        name: "Reviewer",
        task: "Analyzing diff",
        progress: 38,
    },
    AgentCard {
        name: "Optimizer",
        task: "Reducing allocations",
        progress: 10,
    },
];

// ---------------------------------------------------------------------------
// LayoutRenderable for AgentCard
// ---------------------------------------------------------------------------

impl LayoutRenderable for &AgentCard {
    fn measure(&self, constraints: Constraints) -> Size {
        let width = constraints.constrain(Size::new(30, 5)).width;
        Size::new(width, 5)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height < 5 || area.width < 10 {
            return;
        }

        let w = area.width as usize;
        let dim = Style::default().fg(Color::DarkGray);
        let name_style = Style::default().fg(Color::Cyan);
        let task_style = Style::default().fg(Color::White);

        // Line 0: header
        let header = format!("\u{250c}\u{2500} {} \u{2500}\u{2510}", self.name);
        buf.set_stringn(area.x, area.y, &header, w, name_style);

        // Line 1: task
        let task_line = format!("  {}", self.task);
        buf.set_stringn(area.x, area.y + 1, &task_line, w, task_style);

        // Line 2: progress bar
        let bar_width = (w.saturating_sub(8)).min(20);
        let filled = (self.progress as usize * bar_width) / 100;
        let empty = bar_width.saturating_sub(filled);
        let bar = format!(
            "  [{}{}] {:>3}%",
            "\u{2588}".repeat(filled),
            "\u{2591}".repeat(empty),
            self.progress,
        );
        let bar_color = match self.progress {
            0..=33 => Color::Red,
            34..=66 => Color::Yellow,
            _ => Color::Green,
        };
        buf.set_stringn(area.x, area.y + 2, &bar, w, Style::default().fg(bar_color));

        // Line 3: cost
        let cost_cents = (self.name.len() * 7 + self.progress as usize) % 100;
        let cost_line = format!("  cost: $0.{cost_cents:02}");
        buf.set_stringn(area.x, area.y + 3, &cost_line, w, dim);

        // Line 4: blank separator (already blank in buffer)
    }
}

// ---------------------------------------------------------------------------
// Playground state
// ---------------------------------------------------------------------------

struct DemoState {
    card_count: usize,
}

fn render(state: &DemoState, area: Rect, buf: &mut Buffer) {
    let mut grid = GridRenderable::new(30).gap(2, 1);
    for card in &AGENTS[..state.card_count] {
        grid.push(card);
    }
    grid.render(area, buf);
}

fn tick(_state: &mut DemoState, _dt: Duration) {}

fn apply(state: &mut DemoState, key: &crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('+') | KeyCode::Right if state.card_count < AGENTS.len() => {
            state.card_count += 1;
        }
        KeyCode::Char('-') | KeyCode::Left if state.card_count > 1 => {
            state.card_count -= 1;
        }
        _ => {}
    }
}

fn main() -> std::io::Result<()> {
    playground::run_animated_interactive(
        DemoState { card_count: 4 },
        "Grid: Agent Dashboard (+/- to add/remove cards)",
        render,
        tick,
        apply,
        Duration::from_millis(100),
    )
}

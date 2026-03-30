use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tui_lib::devkit::flame_graph::{generate_palette, test_span_tree};
use tui_lib::devkit::playground;
use tui_lib::widget::flame_graph::{FlameGraph, render_flame_graph};

const COST_NAMES: [&str; 5] = ["cpu", "io", "mem", "gc", "net"];

// ---------------------------------------------------------------------------
// ThemePicker state
// ---------------------------------------------------------------------------

struct ThemePicker {
    fg: FlameGraph,
    center_hue: f64,
    spread: f64,
    tint: f64,
    active_param: usize,
}

impl ThemePicker {
    fn new() -> Self {
        let span_tree = test_span_tree();
        let center_hue = 210.0;
        let spread = 150.0;
        let tint = 0.25;
        let cost_types = generate_palette(center_hue, spread, tint, &COST_NAMES);

        let mut fg = FlameGraph::new(span_tree, cost_types);

        // Start one-level expanded with legend visible so all colors show.
        let root_id = fg.root().id;
        let first_child_id = fg.root().children[0].id;
        fg.handle_key(&make_key(KeyCode::Right)); // expand root, descend
        fg.handle_key(&make_key(KeyCode::Enter)); // toggle legend

        // Tick to completion so the expand animation finishes cleanly.
        for _ in 0..60 {
            fg.tick(Duration::from_millis(16));
        }

        // Verify the path is set correctly (root -> first child).
        debug_assert_eq!(fg.path(), &[root_id, first_child_id]);

        Self {
            fg,
            center_hue,
            spread,
            tint,
            active_param: 0,
        }
    }

    fn regenerate_palette(&mut self) {
        let palette = generate_palette(self.center_hue, self.spread, self.tint, &COST_NAMES);
        self.fg.set_cost_types(palette);
    }

    fn adjust_active_param(&mut self, up: bool) {
        match self.active_param {
            0 => {
                // center_hue: 0-350, step 10, wraps
                if up {
                    self.center_hue = (self.center_hue + 10.0).rem_euclid(360.0);
                } else {
                    self.center_hue = (self.center_hue - 10.0).rem_euclid(360.0);
                }
            }
            1 => {
                // spread: 30-360, step 10
                if up {
                    self.spread = (self.spread + 10.0).min(360.0);
                } else {
                    self.spread = (self.spread - 10.0).max(30.0);
                }
            }
            2 => {
                // tint: 0.0-0.80, step 0.05
                if up {
                    self.tint = (self.tint + 0.05).min(0.80);
                } else {
                    self.tint = (self.tint - 0.05).max(0.0);
                }
                // Round to avoid float drift.
                self.tint = (self.tint * 100.0).round() / 100.0;
            }
            _ => {}
        }
        self.regenerate_palette();
    }
}

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

// ---------------------------------------------------------------------------
// Callbacks for run_animated_interactive
// ---------------------------------------------------------------------------

fn tick(state: &mut ThemePicker, dt: Duration) {
    state.fg.tick(dt);
}

fn apply(state: &mut ThemePicker, key: &KeyEvent) {
    match key.code {
        KeyCode::Tab => {
            state.active_param = (state.active_param + 1) % 3;
        }
        KeyCode::Up => {
            state.adjust_active_param(true);
        }
        KeyCode::Down => {
            state.adjust_active_param(false);
        }
        // Forward navigation keys to inner FlameGraph.
        KeyCode::Char('j')
        | KeyCode::Char('k')
        | KeyCode::Char('h')
        | KeyCode::Char('l')
        | KeyCode::Char('u')
        | KeyCode::Char('r')
        | KeyCode::Enter => {
            state.fg.handle_key(key);
        }
        _ => {}
    }
}

fn render(state: &ThemePicker, area: Rect, buf: &mut Buffer) {
    if area.height < 4 {
        render_flame_graph(&state.fg, area, buf);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Min(1),    // flame graph
        Constraint::Length(2), // parameter panel
    ])
    .split(area);

    render_flame_graph(&state.fg, chunks[0], buf);
    render_param_panel(state, chunks[1], buf);
}

fn render_param_panel(state: &ThemePicker, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focus_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let normal_style = Style::default().fg(Color::Rgb(160, 160, 160));
    let hint_style = Style::default().fg(Color::Rgb(100, 100, 100));

    // Line 1: parameter values.
    let center_label = format!("center={}\u{00b0}", state.center_hue as u32);
    let spread_label = format!("spread={}\u{00b0}", state.spread as u32);
    let tint_label = format!("tint={}%", (state.tint * 100.0).round() as u32);

    let center_style = if state.active_param == 0 {
        focus_style
    } else {
        normal_style
    };
    let spread_style = if state.active_param == 1 {
        focus_style
    } else {
        normal_style
    };
    let tint_style = if state.active_param == 2 {
        focus_style
    } else {
        normal_style
    };

    let line1 = Line::from(vec![
        Span::styled(" Theme: ", normal_style),
        Span::styled(center_label, center_style),
        Span::styled("  ", normal_style),
        Span::styled(spread_label, spread_style),
        Span::styled("  ", normal_style),
        Span::styled(tint_label, tint_style),
        Span::styled("   [Tab to switch, \u{2191}\u{2193} to adjust]", hint_style),
    ]);

    let y0 = area.y;
    let paragraph = Paragraph::new(line1);
    let line1_area = Rect::new(area.x, y0, area.width, 1);
    ratatui::widgets::Widget::render(paragraph, line1_area, buf);

    // Line 2: color swatches.
    if area.height >= 2 {
        let palette = generate_palette(state.center_hue, state.spread, state.tint, &COST_NAMES);
        let mut spans: Vec<Span> = vec![Span::styled(" Colors: ", normal_style)];
        for ct in &palette {
            spans.push(Span::styled(
                "\u{2588}\u{2588}\u{2588}\u{2588}",
                Style::default().fg(ct.color),
            ));
            spans.push(Span::styled(format!(" {} ", ct.name), normal_style));
        }
        let line2 = Line::from(spans);
        let paragraph2 = Paragraph::new(line2);
        let line2_area = Rect::new(area.x, y0 + 1, area.width, 1);
        ratatui::widgets::Widget::render(paragraph2, line2_area, buf);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> std::io::Result<()> {
    let state = ThemePicker::new();

    playground::run_animated_interactive(
        state,
        "Theme Picker",
        render,
        tick,
        apply,
        Duration::from_millis(16),
    )
}

#![cfg(feature = "devkit")]

//! Visual comparison of BarStyle variants for the flame graph.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::layout::Rect;
use tui_lib::devkit::Surface;
use tui_lib::devkit::flame_graph::test_flame_graph;
use tui_lib::widget::flame_graph::{BarStyle, render_flame_graph};

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn finish_animations(fg: &mut tui_lib::widget::flame_graph::FlameGraph) {
    for _ in 0..96 {
        fg.tick(Duration::from_millis(16));
    }
}

/// Render both bar styles side-by-side for visual comparison.
#[test]
fn compare_bar_styles() {
    let width = 80;
    let height = 12;
    let area = Rect::new(0, 0, width, height);

    for (name, style) in [
        ("ThinLine", BarStyle::ThinLine),
        ("Dotted", BarStyle::Dotted),
    ] {
        let mut fg = test_flame_graph();
        fg.set_bar_style(style);
        fg.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut fg);

        let mut surface = Surface::new(width, height);
        render_flame_graph(&fg, area, surface.buffer_mut());
        let styled = surface.to_styled_text();
        let plain = surface.to_text();

        eprintln!("=== {name} Style ===");
        eprintln!("{styled}");
        eprintln!();

        // Both should contain all labels.
        for label in &[
            "request",
            "db_query",
            "template_rend",
            "auth_check",
            "logging",
        ] {
            assert!(plain.contains(label), "{name} style missing {label}");
        }
    }
}

/// Render focused mode with both styles.
#[test]
fn compare_bar_styles_focused() {
    let width = 80;
    let height = 10;
    let area = Rect::new(0, 0, width, height);

    for (name, style) in [
        ("ThinLine", BarStyle::ThinLine),
        ("Dotted", BarStyle::Dotted),
    ] {
        let mut fg = test_flame_graph();
        fg.set_bar_style(style);
        fg.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut fg);
        fg.handle_key(&make_key(KeyCode::Char('f')));
        finish_animations(&mut fg);

        let mut surface = Surface::new(width, height);
        render_flame_graph(&fg, area, surface.buffer_mut());
        eprintln!("=== {name} Style (focused) ===");
        eprintln!("{}", surface.to_styled_text());
        eprintln!();
    }
}

/// Render at narrow width to check summary mode isn't affected.
#[test]
fn narrow_width_unaffected_by_bar_style() {
    let width = 20;
    let height = 8;

    for style in [BarStyle::ThinLine, BarStyle::Dotted] {
        let mut fg = test_flame_graph();
        fg.set_bar_style(style);
        fg.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut fg);

        let mut surface = Surface::new(width, height);
        render_flame_graph(&fg, Rect::new(0, 0, width, height), surface.buffer_mut());
        let text = surface.to_text();

        // In summary mode (no bars), bar style shouldn't matter.
        assert!(text.contains('%'), "summary mode should show percentages");
    }
}

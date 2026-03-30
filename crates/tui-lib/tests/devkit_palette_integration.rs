//! Integration tests for the palette <-> playground dispatch chain.
//!
//! These tests exercise the full key path: KeyEvent -> palette handle_key ->
//! apply adapter -> PlaygroundController -> mode transition. They exist to
//! catch seam bugs where individual layers work but the wiring between
//! them is wrong.

#![cfg(feature = "devkit")]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use tui_lib::command_palette::{CommandPaletteState, HotkeyBinding, PaletteItem};
use tui_lib::devkit::{PlaygroundController, PlaygroundMode, Scenario, ScenarioCatalog};
use tui_lib::input::KeyResult;

// ---------------------------------------------------------------------------
// Wiring (mirrors the example, but minimal)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cmd {
    Alpha,
    Beta,
}

fn palette() -> CommandPaletteState<Cmd> {
    CommandPaletteState::new(vec![
        PaletteItem {
            label: "Alpha".into(),
            hotkey: Some(HotkeyBinding {
                label: "Ctrl+A",
                matches: |k| {
                    k.code == KeyCode::Char('a') && k.modifiers.contains(KeyModifiers::CONTROL)
                },
            }),
            id: Cmd::Alpha,
        },
        PaletteItem {
            label: "Beta".into(),
            hotkey: None,
            id: Cmd::Beta,
        },
    ])
}

fn render(_state: &CommandPaletteState<Cmd>, _area: Rect, _buf: &mut Buffer) {}

fn apply(state: &mut CommandPaletteState<Cmd>, event: &KeyEvent) -> KeyResult {
    state.handle_key(event).to_key_result()
}

fn catalog() -> ScenarioCatalog<CommandPaletteState<Cmd>> {
    let mut cat = ScenarioCatalog::new_interactive(render, apply);
    cat.add(Scenario {
        name: "default",
        description: "",
        state: palette(),
        inputs: vec![],
    });
    cat
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Enter Live mode from Explorer.
fn enter_live(ctrl: &mut PlaygroundController<CommandPaletteState<Cmd>>) {
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn esc_in_live_returns_to_explorer() {
    let cat = catalog();
    let mut ctrl = PlaygroundController::new(&cat);
    enter_live(&mut ctrl);

    ctrl.handle_key(&key(KeyCode::Esc));
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
}

#[test]
fn typing_stays_in_live() {
    let cat = catalog();
    let mut ctrl = PlaygroundController::new(&cat);
    enter_live(&mut ctrl);

    ctrl.handle_key(&key(KeyCode::Char('a')));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
    assert_eq!(ctrl.live_state().filter_text(), "a");
}

#[test]
fn ctrl_hotkey_stays_in_live() {
    let cat = catalog();
    let mut ctrl = PlaygroundController::new(&cat);
    enter_live(&mut ctrl);

    // Ctrl+A is a palette hotkey — should be consumed by the palette,
    // NOT bubble up to the playground.
    ctrl.handle_key(&key_ctrl('a'));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
    // Filter should be empty (hotkey, not text input).
    assert_eq!(ctrl.live_state().filter_text(), "");
}

#[test]
fn unbound_ctrl_stays_in_live() {
    let cat = catalog();
    let mut ctrl = PlaygroundController::new(&cat);
    enter_live(&mut ctrl);

    // Ctrl+Z is not bound anywhere — palette returns Ignored,
    // playground fallback also doesn't match. Should stay in Live.
    ctrl.handle_key(&key_ctrl('z'));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

#[test]
fn arrow_keys_stay_in_live() {
    let cat = catalog();
    let mut ctrl = PlaygroundController::new(&cat);
    enter_live(&mut ctrl);

    ctrl.handle_key(&key(KeyCode::Down));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);

    ctrl.handle_key(&key(KeyCode::Up));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

#[test]
fn backspace_stays_in_live() {
    let cat = catalog();
    let mut ctrl = PlaygroundController::new(&cat);
    enter_live(&mut ctrl);

    // Type then backspace.
    ctrl.handle_key(&key(KeyCode::Char('x')));
    ctrl.handle_key(&key(KeyCode::Backspace));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
    assert_eq!(ctrl.live_state().filter_text(), "");
}

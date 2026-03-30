#![cfg(feature = "devkit")]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use tui_lib::devkit::vim_editor::{apply_vim_editor, render_vim_editor};
use tui_lib::devkit::{Action, PlaygroundController, PlaygroundMode, Scenario, ScenarioCatalog};
use tui_lib::input::{KeyResult, RenderEffect};
use tui_lib::widget::VimEditor;

// ---------------------------------------------------------------------------
// Test state types
// ---------------------------------------------------------------------------

/// Records every key event forwarded to it. Always returns `Consumed`.
#[derive(Clone, Default)]
struct TestState {
    keys: Vec<KeyEvent>,
}

fn test_render(_state: &TestState, _area: Rect, _buf: &mut Buffer) {}

fn test_apply(state: &mut TestState, event: &KeyEvent) -> KeyResult {
    state.keys.push(*event);
    KeyResult::Consumed
}

/// Like `test_apply` but returns `Ignored` for Esc.
fn test_apply_esc_ignored(state: &mut TestState, event: &KeyEvent) -> KeyResult {
    if event.code == KeyCode::Esc {
        return KeyResult::Ignored;
    }
    state.keys.push(*event);
    KeyResult::Consumed
}

/// Simulates a widget with an "open popup" flag.
/// - Esc when popup open: closes popup, returns Consumed.
/// - Esc when popup closed: returns Ignored.
/// - Everything else: Consumed.
#[derive(Clone)]
struct NestedState {
    popup_open: bool,
}

fn nested_render(_state: &NestedState, _area: Rect, _buf: &mut Buffer) {}

fn nested_apply(state: &mut NestedState, event: &KeyEvent) -> KeyResult {
    if event.code == KeyCode::Esc && state.popup_open {
        state.popup_open = false;
        return KeyResult::Consumed;
    }
    if event.code == KeyCode::Esc {
        return KeyResult::Ignored;
    }
    KeyResult::Consumed
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

/// Build an interactive catalog with `test_apply_esc_ignored` and one
/// scenario that has the given inputs.
fn esc_ignored_catalog(inputs: Vec<KeyEvent>) -> ScenarioCatalog<TestState> {
    let mut catalog = ScenarioCatalog::new_interactive(test_render, test_apply_esc_ignored);
    catalog.add(Scenario {
        name: "default",
        description: "",
        state: TestState::default(),
        inputs,
    });
    catalog
}

/// Build an interactive catalog with `test_apply` (consumes everything)
/// and one scenario with the given inputs.
fn consume_all_catalog(inputs: Vec<KeyEvent>) -> ScenarioCatalog<TestState> {
    let mut catalog = ScenarioCatalog::new_interactive(test_render, test_apply);
    catalog.add(Scenario {
        name: "default",
        description: "",
        state: TestState::default(),
        inputs,
    });
    catalog
}

/// Build an interactive catalog with `test_apply` and multiple scenarios.
fn multi_scenario_catalog(count: usize) -> ScenarioCatalog<TestState> {
    let mut catalog = ScenarioCatalog::new_interactive(test_render, test_apply);
    for i in 0..count {
        catalog.add(Scenario {
            name: match i {
                0 => "first",
                1 => "second",
                2 => "third",
                _ => "extra",
            },
            description: "",
            state: TestState::default(),
            inputs: vec![KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)],
        });
    }
    catalog
}

/// Build a static (non-interactive) catalog with multiple scenarios.
fn static_catalog(count: usize) -> ScenarioCatalog<TestState> {
    let mut catalog = ScenarioCatalog::new(test_render);
    for i in 0..count {
        catalog.add(Scenario {
            name: match i {
                0 => "first",
                1 => "second",
                2 => "third",
                _ => "extra",
            },
            description: "",
            state: TestState::default(),
            inputs: vec![],
        });
    }
    catalog
}

// ===========================================================================
// Group 1: Modifier handling
// ===========================================================================

#[test]
fn ctrl_b_forwarded_to_widget_in_live_mode() {
    // No inputs => Enter goes straight to Live.
    let catalog = esc_ignored_catalog(vec![]);
    let mut ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);

    // Enter scenario (Live).
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);

    let action = ctrl.handle_key(&key_ctrl(KeyCode::Char('b')));
    assert_eq!(action, Action::Continue);

    // Widget should have received exactly one event with Ctrl modifier.
    let keys = &ctrl.live_state().keys;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].code, KeyCode::Char('b'));
    assert!(keys[0].modifiers.contains(KeyModifiers::CONTROL));
}

// ===========================================================================
// Group 2: Scenario navigation
// ===========================================================================

#[test]
fn brackets_navigate_in_explorer() {
    let catalog = multi_scenario_catalog(3);
    let mut ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
    assert_eq!(ctrl.current(), 0);

    ctrl.handle_key(&key(KeyCode::Char(']')));
    assert_eq!(ctrl.current(), 1);

    ctrl.handle_key(&key(KeyCode::Char(']')));
    assert_eq!(ctrl.current(), 2);

    ctrl.handle_key(&key(KeyCode::Char('[')));
    assert_eq!(ctrl.current(), 1);
}

#[test]
fn brackets_navigate_in_static_explorer() {
    let catalog = static_catalog(3);
    let mut ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
    assert_eq!(ctrl.current(), 0);

    ctrl.handle_key(&key(KeyCode::Char(']')));
    assert_eq!(ctrl.current(), 1);

    ctrl.handle_key(&key(KeyCode::Char(']')));
    assert_eq!(ctrl.current(), 2);

    ctrl.handle_key(&key(KeyCode::Char('[')));
    assert_eq!(ctrl.current(), 1);
}

// ===========================================================================
// Group 3: Mode transitions
// ===========================================================================

#[test]
fn initial_mode_always_explorer() {
    // Interactive catalog with inputs.
    let catalog = consume_all_catalog(vec![KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)]);
    let ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);

    // Interactive catalog without inputs.
    let catalog = consume_all_catalog(vec![]);
    let ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);

    // Non-interactive catalog.
    let catalog = static_catalog(1);
    let ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
}

#[test]
fn enter_goes_to_live() {
    let catalog = esc_ignored_catalog(vec![]);
    let mut ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);

    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

#[test]
fn question_mark_toggles_help_overlay() {
    let catalog = static_catalog(1);
    let mut ctrl = PlaygroundController::new(&catalog);
    assert!(!ctrl.help_open());

    ctrl.handle_key(&key(KeyCode::Char('?')));
    assert!(ctrl.help_open());

    ctrl.handle_key(&key(KeyCode::Esc));
    assert!(!ctrl.help_open());
}

#[test]
fn step_key_emits_render_for_help_toggle() {
    let catalog = static_catalog(1);
    let mut ctrl = PlaygroundController::new(&catalog);

    let step = ctrl.step_key(&key(KeyCode::Char('?')));

    assert_eq!(step.action, Action::Continue);
    assert_eq!(step.effect, Some(RenderEffect::ScheduleRender));
    assert!(ctrl.help_open());
}

#[test]
fn enter_goes_to_live_with_inputs() {
    // Even when scenario has inputs, Enter goes straight to Live (no replay).
    let catalog = consume_all_catalog(vec![KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)]);
    let mut ctrl = PlaygroundController::new(&catalog);
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

#[test]
fn enter_noop_for_non_interactive() {
    let catalog = static_catalog(1);
    let mut ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);

    let step = ctrl.step_key(&key(KeyCode::Enter));
    assert_eq!(step.effect, None);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
}

#[test]
fn esc_returns_to_explorer_from_live() {
    let catalog = esc_ignored_catalog(vec![]);
    let mut ctrl = PlaygroundController::new(&catalog);
    // Enter Live.
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);

    let action = ctrl.handle_key(&key(KeyCode::Esc));
    assert_eq!(action, Action::Continue);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
}

#[test]
fn esc_consumed_by_widget_in_live() {
    let catalog = consume_all_catalog(vec![]);
    let mut ctrl = PlaygroundController::new(&catalog);
    // Enter Live.
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);

    let action = ctrl.handle_key(&key(KeyCode::Esc));
    assert_eq!(action, Action::Continue);
    // Widget consumed Esc -- mode stays Live.
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

#[test]
fn step_key_emits_render_when_live_widget_consumes_input() {
    let catalog = consume_all_catalog(vec![]);
    let mut ctrl = PlaygroundController::new(&catalog);
    ctrl.handle_key(&key(KeyCode::Enter));

    let step = ctrl.step_key(&key(KeyCode::Char('x')));

    assert_eq!(step.action, Action::Continue);
    assert_eq!(step.effect, Some(RenderEffect::ScheduleRender));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
}

// ===========================================================================
// Group 4: Inner-first Esc (the key test)
// ===========================================================================

#[test]
fn nested_esc_peels_layers() {
    let mut catalog = ScenarioCatalog::new_interactive(nested_render, nested_apply);
    catalog.add(Scenario {
        name: "nested",
        description: "",
        state: NestedState { popup_open: true },
        inputs: vec![],
    });
    let mut ctrl = PlaygroundController::new(&catalog);
    // Enter Live.
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);

    // First Esc: widget closes popup, returns Consumed. Stay in Live.
    let action = ctrl.handle_key(&key(KeyCode::Esc));
    assert_eq!(action, Action::Continue);
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);
    assert!(!ctrl.live_state().popup_open);

    // Second Esc: widget returns Ignored (popup already closed).
    // Playground transitions to Explorer.
    let action = ctrl.handle_key(&key(KeyCode::Esc));
    assert_eq!(action, Action::Continue);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
}

#[test]
fn vim_editor_normal_mode_escape_returns_to_explorer() {
    let mut editor = VimEditor::new();
    apply_vim_editor(&mut editor, &key(KeyCode::Char('b')));
    apply_vim_editor(&mut editor, &key(KeyCode::Char('y')));
    apply_vim_editor(&mut editor, &key(KeyCode::Char('e')));
    apply_vim_editor(&mut editor, &key(KeyCode::Esc));

    let mut catalog = ScenarioCatalog::new_interactive(render_vim_editor, apply_vim_editor);
    catalog.add(Scenario {
        name: "normal",
        description: "",
        state: editor,
        inputs: vec![],
    });

    let mut ctrl = PlaygroundController::new(&catalog);
    ctrl.handle_key(&key(KeyCode::Enter));
    assert_eq!(ctrl.mode(), PlaygroundMode::Live);

    let action = ctrl.handle_key(&key(KeyCode::Esc));
    assert_eq!(action, Action::Continue);
    assert_eq!(ctrl.mode(), PlaygroundMode::Explorer);
}

// ===========================================================================
// Group 5: Context breadcrumb
// ===========================================================================

#[test]
fn context_fn_reports_widget_context() {
    fn test_context(state: &TestState) -> Vec<&'static str> {
        if state.keys.is_empty() {
            vec!["widget"]
        } else {
            vec!["widget", "active"]
        }
    }

    let mut catalog =
        ScenarioCatalog::new_interactive(test_render, test_apply).with_context_fn(test_context);
    catalog.add(Scenario {
        name: "default",
        description: "",
        state: TestState::default(),
        inputs: vec![],
    });

    let ctrl = PlaygroundController::new(&catalog);
    assert_eq!(ctrl.catalog().context(ctrl.live_state()), vec!["widget"]);
}

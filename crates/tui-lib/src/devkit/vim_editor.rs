use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{Scenario, ScenarioCatalog};
use crate::input::KeyResult;
use crate::render::LayoutRenderable;
use crate::widget::{EditorEffect, VimEditor, VimMode};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn char_key(c: char) -> KeyEvent {
    if c.is_uppercase() {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    } else {
        key(KeyCode::Char(c))
    }
}

fn type_text(editor: &mut VimEditor, text: &str) {
    for c in text.chars() {
        editor.step(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn normal_editor(text: &str) -> VimEditor {
    let mut editor = VimEditor::new();
    type_text(&mut editor, text);
    let effect = editor.step(key(KeyCode::Esc));
    assert_eq!(effect, EditorEffect::Consumed);
    editor
}

fn visual_editor(text: &str, inputs: &[KeyEvent]) -> VimEditor {
    let mut editor = normal_editor(text);
    for input in inputs {
        let effect = editor.step(*input);
        assert_ne!(effect, EditorEffect::Ignored);
    }
    editor
}

pub fn render_vim_editor(editor: &VimEditor, area: Rect, buf: &mut Buffer) {
    editor.render(area, buf);
}

pub fn apply_vim_editor(editor: &mut VimEditor, event: &KeyEvent) -> KeyResult {
    match editor.step(*event) {
        EditorEffect::Consumed | EditorEffect::Submit(_) => KeyResult::Consumed,
        // Bubble editor exit to the outer playground shell so Esc in normal
        // mode can peel back to explorer instead of getting trapped here.
        EditorEffect::Exit => KeyResult::Ignored,
        EditorEffect::Ignored => KeyResult::Ignored,
    }
}

pub fn vim_editor_context(editor: &VimEditor) -> Vec<&'static str> {
    let mut context = vec!["vim-editor"];
    let mode = match editor.mode() {
        VimMode::Insert => "insert",
        VimMode::Normal => "normal",
        VimMode::Visual => "visual",
    };
    context.push(mode);
    if editor.pending_display().is_some() {
        context.push("pending");
    }
    context
}

pub fn vim_editor_static_catalog() -> ScenarioCatalog<VimEditor> {
    let mut catalog = ScenarioCatalog::new(render_vim_editor);
    catalog.add(Scenario {
        name: "insert-empty",
        description: "Fresh editor in insert mode",
        state: VimEditor::new(),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "insert-with-text",
        description: "Insert mode with text and cursor at end",
        state: {
            let mut editor = VimEditor::new();
            type_text(&mut editor, "hello world");
            editor
        },
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "normal-mode",
        description: "Normal mode after leaving insert",
        state: normal_editor("hello world"),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "visual-selection",
        description: "Visual mode with an active forward selection",
        state: visual_editor(
            "hello world",
            &[char_key('0'), char_key('v'), char_key('w')],
        ),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "pending-delete",
        description: "Normal mode showing an operator-pending delete command",
        state: {
            let mut editor = normal_editor("hello world");
            let effect = editor.step(char_key('d'));
            assert_eq!(effect, EditorEffect::Consumed);
            editor
        },
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "pending-find",
        description: "Normal mode showing a pending find command",
        state: {
            let mut editor = normal_editor("hello world");
            let effect = editor.step(char_key('f'));
            assert_eq!(effect, EditorEffect::Consumed);
            editor
        },
        inputs: vec![],
    });
    catalog
}

pub fn vim_editor_interactive_catalog() -> ScenarioCatalog<VimEditor> {
    let mut catalog = ScenarioCatalog::new_interactive(render_vim_editor, apply_vim_editor)
        .with_context_fn(vim_editor_context);
    catalog.add(Scenario {
        name: "insert-to-normal",
        description: "Type text and leave insert mode",
        state: VimEditor::new(),
        inputs: vec![
            char_key('h'),
            char_key('e'),
            char_key('l'),
            char_key('l'),
            char_key('o'),
            key(KeyCode::Esc),
        ],
    });
    catalog.add(Scenario {
        name: "visual-motion",
        description: "Create a visual selection with v then w",
        state: {
            let mut editor = VimEditor::new();
            type_text(&mut editor, "hello world");
            editor
        },
        inputs: vec![
            key(KeyCode::Esc),
            char_key('0'),
            char_key('v'),
            char_key('w'),
        ],
    });
    catalog.add(Scenario {
        name: "pending-command",
        description: "Enter operator-pending delete-find sequence",
        state: {
            let mut editor = VimEditor::new();
            type_text(&mut editor, "delete target");
            editor
        },
        inputs: vec![
            key(KeyCode::Esc),
            char_key('0'),
            char_key('d'),
            char_key('f'),
        ],
    });
    catalog.add(Scenario {
        name: "submit-behavior",
        description: "Submit text with Enter from insert mode",
        state: VimEditor::new(),
        inputs: vec![char_key('o'), char_key('k'), key(KeyCode::Enter)],
    });
    catalog.add(Scenario {
        name: "exit-behavior",
        description: "Exit from normal mode with Esc",
        state: {
            let mut editor = VimEditor::new();
            type_text(&mut editor, "bye");
            editor
        },
        inputs: vec![key(KeyCode::Esc), key(KeyCode::Esc)],
    });
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_vim_editor_bubbles_exit_from_normal_mode() {
        let mut editor = VimEditor::new();
        type_text(&mut editor, "bye");
        assert_eq!(editor.step(key(KeyCode::Esc)), EditorEffect::Consumed);

        assert_eq!(
            apply_vim_editor(&mut editor, &key(KeyCode::Esc)),
            KeyResult::Ignored
        );
    }
}

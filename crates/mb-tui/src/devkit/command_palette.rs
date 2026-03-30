use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{Scenario, ScenarioCatalog};
use crate::command_palette::{
    CommandPaletteState, HotkeyBinding, PaletteItem, render_command_palette as render_palette,
};
use crate::input::KeyResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoId {
    GitCommit,
    GitPush,
    GitPull,
    GitBranch,
    BuildDebug,
    BuildRelease,
    BuildClean,
    FormatAll,
    LintCheck,
    TestRun,
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn char_key(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn simple(label: &str, id: DemoId) -> PaletteItem<DemoId> {
    PaletteItem {
        label: label.into(),
        hotkey: None,
        id,
    }
}

pub fn demo_palette() -> CommandPaletteState<DemoId> {
    CommandPaletteState::new(vec![
        PaletteItem {
            label: "Git: Commit".into(),
            hotkey: Some(HotkeyBinding {
                label: "Ctrl+G",
                matches: |k| {
                    k.code == KeyCode::Char('g') && k.modifiers.contains(KeyModifiers::CONTROL)
                },
            }),
            id: DemoId::GitCommit,
        },
        simple("Git: Push", DemoId::GitPush),
        simple("Git: Pull", DemoId::GitPull),
        simple("Git: Branch", DemoId::GitBranch),
        PaletteItem {
            label: "Build: Debug".into(),
            hotkey: Some(HotkeyBinding {
                label: "Ctrl+B",
                matches: |k| {
                    k.code == KeyCode::Char('b') && k.modifiers.contains(KeyModifiers::CONTROL)
                },
            }),
            id: DemoId::BuildDebug,
        },
        simple("Build: Release", DemoId::BuildRelease),
        simple("Build: Clean", DemoId::BuildClean),
        PaletteItem {
            label: "Format All".into(),
            hotkey: Some(HotkeyBinding {
                label: "Ctrl+F",
                matches: |k| {
                    k.code == KeyCode::Char('f') && k.modifiers.contains(KeyModifiers::CONTROL)
                },
            }),
            id: DemoId::FormatAll,
        },
        simple("Lint: Check", DemoId::LintCheck),
        PaletteItem {
            label: "Test: Run".into(),
            hotkey: Some(HotkeyBinding {
                label: "Ctrl+T",
                matches: |k| {
                    k.code == KeyCode::Char('t') && k.modifiers.contains(KeyModifiers::CONTROL)
                },
            }),
            id: DemoId::TestRun,
        },
    ])
}

fn filtered_palette(filter: &str) -> CommandPaletteState<DemoId> {
    let mut palette = demo_palette();
    for c in filter.chars() {
        palette.type_char(c);
    }
    palette
}

fn scrolled_palette(steps: usize) -> CommandPaletteState<DemoId> {
    let mut palette = demo_palette();
    for _ in 0..steps {
        palette.scroll_down();
    }
    palette
}

pub fn render_command_palette(state: &CommandPaletteState<DemoId>, area: Rect, buf: &mut Buffer) {
    render_palette(state, area, buf);
}

pub fn apply_command_palette(
    state: &mut CommandPaletteState<DemoId>,
    event: &KeyEvent,
) -> KeyResult {
    state.handle_key(event).to_key_result()
}

pub fn command_palette_context(_state: &CommandPaletteState<DemoId>) -> Vec<&'static str> {
    vec!["cmdpalette"]
}

pub fn command_palette_static_catalog() -> ScenarioCatalog<CommandPaletteState<DemoId>> {
    let mut catalog = ScenarioCatalog::new(render_command_palette);
    add_static_scenarios(&mut catalog);
    catalog
}

pub fn command_palette_interactive_catalog() -> ScenarioCatalog<CommandPaletteState<DemoId>> {
    let mut catalog =
        ScenarioCatalog::new_interactive(render_command_palette, apply_command_palette)
            .with_context_fn(command_palette_context);
    add_static_scenarios(&mut catalog);
    catalog.add(Scenario {
        name: "type-git-select",
        description: "Type 'git' then navigate down twice",
        state: demo_palette(),
        inputs: vec![
            char_key('g'),
            char_key('i'),
            char_key('t'),
            key(KeyCode::Down),
            key(KeyCode::Down),
        ],
    });
    catalog
}

fn add_static_scenarios(catalog: &mut ScenarioCatalog<CommandPaletteState<DemoId>>) {
    catalog.add(Scenario {
        name: "initial",
        description: "Fresh palette, no filter, all 10 items",
        state: demo_palette(),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "filtered-git",
        description: "After typing 'git', showing 4 git items",
        state: filtered_palette("git"),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "filtered-build",
        description: "After typing 'b', showing build-prefixed results for compact summary snapshots",
        state: filtered_palette("b"),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "scrolled-down",
        description: "After scrolling down 3 times",
        state: scrolled_palette(3),
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "no-matches",
        description: "After typing 'xyz', empty result",
        state: filtered_palette("xyz"),
        inputs: vec![],
    });
}

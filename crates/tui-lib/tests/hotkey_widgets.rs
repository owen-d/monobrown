use tui_lib::devkit::Surface;
use tui_lib::input::modal::{HotkeyHint, HotkeySection};
use tui_lib::render::{Constraints, LayoutRenderable};
use tui_lib::widget::hotkey::{HelpPaneRenderable, HotkeyBarRenderable, format_hint_string};

#[test]
fn format_hint_string_joins_key_action_pairs() {
    let hints = vec![
        HotkeyHint {
            key: "q",
            action: "quit",
            description: "Quit the app",
        },
        HotkeyHint {
            key: "?",
            action: "help",
            description: "Open help",
        },
    ];

    assert_eq!(format_hint_string(&hints), "q:quit  ?:help");
}

#[test]
fn hotkey_bar_wraps_at_narrow_widths() {
    let hints = vec![
        HotkeyHint {
            key: "q",
            action: "quit",
            description: "Quit the app",
        },
        HotkeyHint {
            key: "?",
            action: "help",
            description: "Open help",
        },
        HotkeyHint {
            key: "enter",
            action: "open",
            description: "Open item",
        },
    ];
    let bar = HotkeyBarRenderable { hints };

    assert!(bar.measure(Constraints::tight_width(12)).height > 1);

    let surface = Surface::auto(12, &bar);
    let text = surface.to_text();
    assert!(text.contains("q:quit"));
    assert!(text.contains("?:help"));
    assert!(text.lines().count() > 1, "bar should wrap:\n{text}");
}

#[test]
fn help_pane_renders_grouped_sections() {
    let sections = vec![
        HotkeySection {
            title: "Normal Mode",
            hints: vec![HotkeyHint {
                key: "q",
                action: "quit",
                description: "Quit the app",
            }],
        },
        HotkeySection {
            title: "Help Overlay",
            hints: vec![HotkeyHint {
                key: "esc",
                action: "close",
                description: "Close help overlay",
            }],
        },
    ];
    let pane = HelpPaneRenderable {
        title: " Help ",
        sections: &sections,
        appendix: &[],
    };

    let surface = Surface::with_area(40, pane.measure(Constraints::tight_width(40)).height, &pane);
    let text = surface.to_text();
    assert!(text.contains("Help"));
    assert!(text.contains("Normal Mode"));
    assert!(text.contains("Quit the app"));
    assert!(text.contains("Help Overlay"));
    assert!(text.contains("Close help overlay"));
}

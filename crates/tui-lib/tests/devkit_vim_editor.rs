#![cfg(feature = "devkit")]

use tui_lib::devkit::vim_editor::{vim_editor_interactive_catalog, vim_editor_static_catalog};

#[test]
fn vim_editor_static_snapshots() {
    let catalog = vim_editor_static_catalog();
    catalog.assert_all_snapshots(50, 4);
}

#[test]
fn vim_editor_interactive_snapshots() {
    let catalog = vim_editor_interactive_catalog();
    catalog.assert_all_snapshots_after_inputs(50, 4);
}

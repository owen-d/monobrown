#![cfg(feature = "devkit")]

use mb_tui::devkit::command_palette::{
    command_palette_interactive_catalog, command_palette_static_catalog,
};

#[test]
fn command_palette_static_snapshots() {
    let catalog = command_palette_static_catalog();
    catalog.assert_all_snapshots(50, 15);
}

#[test]
fn command_palette_compact_static_snapshots() {
    let catalog = command_palette_static_catalog();
    catalog.assert_all_snapshots(20, 1);
}

#[test]
fn command_palette_interactive_snapshots() {
    let catalog = command_palette_interactive_catalog();
    catalog.assert_all_snapshots_after_inputs(50, 15);
}

#[test]
fn command_palette_compact_interactive_snapshots() {
    let catalog = command_palette_interactive_catalog();
    catalog.assert_all_snapshots_after_inputs(20, 1);
}

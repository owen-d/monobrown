#![cfg(feature = "devkit")]

use tui_lib::devkit::flame_graph::flame_graph_interactive_catalog;

#[test]
fn flame_graph_wide_snapshots() {
    let catalog = flame_graph_interactive_catalog();
    catalog.assert_all_snapshots(80, 10);
}

#[test]
fn flame_graph_narrow_snapshots() {
    let catalog = flame_graph_interactive_catalog();
    catalog.assert_all_snapshots(20, 6);
}

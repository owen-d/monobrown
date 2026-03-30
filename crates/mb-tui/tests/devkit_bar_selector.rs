#![cfg(feature = "devkit")]

use mb_tui::devkit::bar_selector::bar_selector_catalog;

#[test]
fn bar_selector_wide_snapshots() {
    let catalog = bar_selector_catalog();
    catalog.assert_all_snapshots(30, 4);
}

#[test]
fn bar_selector_explorer_snapshots() {
    let catalog = bar_selector_catalog();
    catalog.assert_all_snapshots(80, 18);
}

#[test]
fn bar_selector_compact_snapshots() {
    let catalog = bar_selector_catalog();
    catalog.assert_all_snapshots(10, 1);
}

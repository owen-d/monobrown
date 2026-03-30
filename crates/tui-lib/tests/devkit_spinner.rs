#![cfg(feature = "devkit")]

use tui_lib::devkit::simple_widgets::{labeled_spinner_catalog, spinner_catalog};

#[test]
fn spinner_snapshots() {
    let catalog = spinner_catalog();
    catalog.assert_all_snapshots(10, 1);
}

#[test]
fn labeled_spinner_snapshots() {
    let catalog = labeled_spinner_catalog();
    catalog.assert_all_snapshots(40, 1);
}

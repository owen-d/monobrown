#![cfg(feature = "devkit")]

use tui_lib::devkit::simple_widgets::shimmer_catalog;

#[test]
fn shimmer_snapshots() {
    let catalog = shimmer_catalog();
    catalog.assert_all_snapshots(20, 1);
}

#[test]
fn shimmer_styled_snapshots() {
    let catalog = shimmer_catalog();
    catalog.assert_all_styled_snapshots(20, 1);
}

#[test]
fn shimmer_fallback_and_truecolor_styled_output_differ() {
    let catalog = shimmer_catalog();

    let fallback = catalog.render_to_styled_text(1, 20, 1);
    let truecolor = catalog.render_to_styled_text(2, 20, 1);

    assert!(fallback.contains("<dim>") || fallback.contains("<bold>"));
    assert!(truecolor.contains("<fg:#"));
    assert_ne!(fallback, truecolor);
}

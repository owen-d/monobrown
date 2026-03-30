use ratatui::layout::Rect;

use mb_tui::devkit::Surface;
use mb_tui::render::{Constraints, LayoutRenderable};
use mb_tui::widget::bar_selector::{BarSelector, render_bar_selector};

#[test]
fn measured_height_tracks_layout_mode() {
    let sel = BarSelector::new(&["Alpha", "Beta", "Gamma"]);
    assert_eq!(sel.measure(Constraints::tight_width(30)).height, 4);
    assert_eq!(sel.measure(Constraints::tight_width(8)).height, 1);
}

#[test]
fn narrow_render_collapses_to_one_line_summary() {
    let mut sel = BarSelector::new(&["Alpha", "Beta", "Gamma"]);
    sel.select(1);

    let mut surface = Surface::new(7, 1);
    let area = Rect::new(0, 0, 7, 1);
    render_bar_selector(&sel, area, surface.buffer_mut());

    let text = surface.to_text();
    assert_eq!(text.lines().count(), 1);
    assert!(
        text.contains("2/3"),
        "narrow summary should preserve the selected position\n{text}"
    );
    assert!(
        !text.trim().is_empty(),
        "narrow summary should not render as blank output\n{text}"
    );
}

#[test]
fn detailed_render_truncates_labels_per_slot() {
    let sel = BarSelector::new(&["Alphabet", "Beta", "Gamma"]);

    let mut surface = Surface::new(12, 4);
    let area = Rect::new(0, 0, 12, 4);
    render_bar_selector(&sel, area, surface.buffer_mut());

    let text = surface.to_text();
    assert!(
        text.contains("Alp…"),
        "detailed layout should keep labels inside each slot\n{text}"
    );
    assert!(
        !text.contains("Alphabet"),
        "detailed layout should truncate labels before they spill into neighbors\n{text}"
    );
}

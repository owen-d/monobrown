use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{Scenario, ScenarioCatalog};
use crate::render::render_pane_frame;
use crate::widget::bar_selector::{BarSelector, render_bar_selector};

pub fn render_selector(state: &BarSelector, area: Rect, buf: &mut Buffer) {
    if let Some((frame_area, inner)) = selector_demo_areas(area)
        && let Some(content) = render_pane_frame(frame_area, buf, "Selector", false)
    {
        let centered_inner = centered_rect(content, inner.width, inner.height);
        render_bar_selector(state, centered_inner, buf);
        return;
    }

    render_bar_selector(state, area, buf);
}

pub fn bar_selector_catalog() -> ScenarioCatalog<BarSelector> {
    let mut catalog = ScenarioCatalog::new(render_selector);

    catalog.add(Scenario {
        name: "initial",
        description: "Default selector with the first slot active",
        state: BarSelector::new(&["Plan", "Code", "Review"]),
        inputs: vec![],
    });

    let mut second_selected = BarSelector::new(&["Plan", "Code", "Review"]);
    second_selected.select(1);
    for _ in 0..32 {
        second_selected.tick(Duration::from_millis(16));
    }
    catalog.add(Scenario {
        name: "second-selected",
        description: "Second slot selected after the transition settles",
        state: second_selected,
        inputs: vec![],
    });

    let mut long_labels = BarSelector::new(&["Discovery", "Implementation", "Verification"]);
    long_labels.select(2);
    for _ in 0..32 {
        long_labels.tick(Duration::from_millis(16));
    }
    catalog.add(Scenario {
        name: "long-labels",
        description: "Long labels to verify truncation and compact summaries",
        state: long_labels,
        inputs: vec![],
    });

    catalog
}

fn selector_demo_areas(area: Rect) -> Option<(Rect, Rect)> {
    let max_outer_width = 34;
    let max_outer_height = 8;
    let outer_width = area.width.min(max_outer_width);
    let outer_height = area.height.min(max_outer_height);
    if outer_width < 14 || outer_height < 6 {
        return None;
    }

    let frame_area = centered_rect(area, outer_width, outer_height);
    let inner_width = outer_width.saturating_sub(4).max(12);
    let inner_height = outer_height.saturating_sub(3).max(4);
    Some((frame_area, Rect::new(0, 0, inner_width, inner_height)))
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

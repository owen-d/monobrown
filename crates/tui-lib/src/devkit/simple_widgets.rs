use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use super::{Scenario, ScenarioCatalog};
use crate::widget::{shimmer_spans_with, spinner_frame};

#[derive(Clone)]
pub struct ShimmerDemo {
    pub text: &'static str,
    pub elapsed: Duration,
    pub has_true_color: bool,
}

#[derive(Clone)]
pub struct SpinnerWithLabel {
    pub elapsed: Duration,
    pub label: &'static str,
}

pub fn render_spinner(elapsed: &Duration, area: Rect, buf: &mut Buffer) {
    let ch = spinner_frame(*elapsed);
    buf.set_string(area.x, area.y, ch.to_string(), Style::default());
}

pub fn render_labeled_spinner(state: &SpinnerWithLabel, area: Rect, buf: &mut Buffer) {
    let ch = spinner_frame(state.elapsed);
    let text = format!("{ch} {}", state.label);
    buf.set_string(area.x, area.y, &text, Style::default());
}

pub fn render_shimmer(state: &ShimmerDemo, area: Rect, buf: &mut Buffer) {
    let line = Line::from(shimmer_spans_with(
        state.text,
        state.elapsed,
        state.has_true_color,
    ));
    line.style(Style::default()).render(area, buf);
}

pub fn spinner_catalog() -> ScenarioCatalog<Duration> {
    let mut catalog = ScenarioCatalog::new(render_spinner);
    catalog.add(Scenario {
        inputs: vec![],
        name: "frame-0",
        description: "First animation frame",
        state: Duration::ZERO,
    });
    catalog.add(Scenario {
        inputs: vec![],
        name: "frame-5",
        description: "Midpoint animation frame",
        state: Duration::from_millis(500),
    });
    catalog.add(Scenario {
        inputs: vec![],
        name: "frame-9",
        description: "Last frame before wrap",
        state: Duration::from_millis(900),
    });
    catalog.add(Scenario {
        inputs: vec![],
        name: "cycle-wrap",
        description: "Wraps back to first frame",
        state: Duration::from_millis(1000),
    });
    catalog.add(Scenario {
        inputs: vec![],
        name: "mid-frame",
        description: "Mid-interval floor behavior",
        state: Duration::from_millis(150),
    });
    catalog
}

pub fn labeled_spinner_catalog() -> ScenarioCatalog<SpinnerWithLabel> {
    let mut catalog = ScenarioCatalog::new(render_labeled_spinner);
    catalog.add(Scenario {
        inputs: vec![],
        name: "loading",
        description: "Standard loading message",
        state: SpinnerWithLabel {
            elapsed: Duration::ZERO,
            label: "Loading...",
        },
    });
    catalog.add(Scenario {
        inputs: vec![],
        name: "long-label",
        description: "Label that may truncate in narrow terminals",
        state: SpinnerWithLabel {
            elapsed: Duration::from_millis(300),
            label: "Processing very long task name...",
        },
    });
    catalog
}

pub fn shimmer_catalog() -> ScenarioCatalog<ShimmerDemo> {
    let mut catalog = ScenarioCatalog::new(render_shimmer);
    catalog.add(Scenario {
        name: "fallback-start",
        description: "Modifier-based shimmer at the start of the sweep",
        state: ShimmerDemo {
            text: "Loading",
            elapsed: Duration::ZERO,
            has_true_color: false,
        },
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "fallback-mid",
        description: "Modifier-based shimmer mid sweep",
        state: ShimmerDemo {
            text: "Loading",
            elapsed: Duration::from_secs(1),
            has_true_color: false,
        },
        inputs: vec![],
    });
    catalog.add(Scenario {
        name: "truecolor-mid",
        description: "RGB shimmer mid sweep",
        state: ShimmerDemo {
            text: "Loading",
            elapsed: Duration::from_secs(1),
            has_true_color: true,
        },
        inputs: vec![],
    });
    catalog
}

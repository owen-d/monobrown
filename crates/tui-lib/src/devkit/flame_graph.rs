use crossterm::event::KeyEvent;
use ratatui::style::Color;

use super::color::hsl_to_rgb;
use super::{Scenario, ScenarioCatalog};
use crate::input::KeyResult;
use crate::widget::flame_graph::{
    CostType, FlameGraph, SpanNode, SpanNodeBuilder, render_flame_graph,
};

/// Cost type names in display order.
const COST_TYPE_NAMES: [&str; 5] = ["cpu", "io", "mem", "gc", "net"];

/// Palette parameters chosen via the theme picker.
const CENTER_HUE: f64 = 70.0;
const SPREAD: f64 = 180.0;
const TINT: f64 = 0.25;

/// Generate a palette of cost types from HSL parameters.
///
/// Hues are spread evenly around `center_hue` within `spread` degrees.
/// Each hue is tinted back toward `center_hue` by the `tint` fraction
/// (0.0 = no tint, 1.0 = all hues collapse to center).
/// Lightness varies 55-70% for distinguishability on dark backgrounds.
pub fn generate_palette(
    center_hue: f64,
    spread: f64,
    tint: f64,
    names: &[&'static str],
) -> Vec<CostType> {
    debug_assert!(!names.is_empty(), "names must not be empty");
    let n = names.len();
    let start = center_hue - spread / 2.0;
    let step = if n > 1 { spread / (n - 1) as f64 } else { 0.0 };
    names
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let raw_hue = (start + step * i as f64).rem_euclid(360.0);
            let tinted_hue = (raw_hue + (center_hue - raw_hue) * tint).rem_euclid(360.0);
            let l = if n > 1 {
                0.55 + (i as f64 / (n - 1) as f64) * 0.15
            } else {
                0.55
            };
            let (r, g, b) = hsl_to_rgb(tinted_hue, 0.60, l);
            CostType {
                name,
                color: Color::Rgb(r, g, b),
            }
        })
        .collect()
}

/// The 5 cost types used in test data.
///
/// Colors use widened-analogous + tint: hues spread 180° around center 70°
/// (warm yellow-green), tinted 25% back toward center for cohesion.
pub fn test_cost_types() -> Vec<CostType> {
    generate_palette(CENTER_HUE, SPREAD, TINT, &COST_TYPE_NAMES)
}

/// Build a synthetic span tree modeling a web request.
pub fn test_span_tree() -> SpanNode {
    let mut b = SpanNodeBuilder::new();
    build_request_tree(&mut b)
}

/// Create a `FlameGraph` with test data.
pub fn test_flame_graph() -> FlameGraph {
    FlameGraph::new(test_span_tree(), test_cost_types())
}

/// Create an interactive scenario catalog for the playground.
pub fn flame_graph_interactive_catalog() -> ScenarioCatalog<FlameGraph> {
    let mut catalog = ScenarioCatalog::new_interactive(
        render_flame_graph,
        |state: &mut FlameGraph, key: &KeyEvent| -> KeyResult { state.handle_key(key) },
    );

    catalog.add(Scenario {
        name: "collapsed",
        description: "Root span only, no expansion",
        state: test_flame_graph(),
        inputs: vec![],
    });

    add_one_level_scenario(&mut catalog);
    add_deep_scenario(&mut catalog);
    add_legend_scenario(&mut catalog);
    add_lower_sibling_scenario(&mut catalog);
    add_focused_mid_scenario(&mut catalog);
    add_focused_leaf_scenario(&mut catalog);

    catalog
}

// --- Tree construction helpers ---

fn build_request_tree(b: &mut SpanNodeBuilder) -> SpanNode {
    let db_query = build_db_query(b);
    let template_render = build_template_render(b);
    let auth_check = build_auth_check(b);
    let logging = b.leaf("logging", vec![2.0, 3.0, 3.0, 1.0, 1.0]);

    b.span(
        "request",
        vec![30.0, 45.0, 15.0, 5.0, 5.0],
        vec![db_query, template_render, auth_check, logging],
    )
}

fn build_db_query(b: &mut SpanNodeBuilder) -> SpanNode {
    let index_scan = b.leaf("index_scan", vec![3.0, 20.0, 1.0, 0.0, 1.0]);
    let row_fetch = b.leaf("row_fetch", vec![1.0, 13.0, 1.0, 0.0, 0.0]);
    let plan_cache = b.leaf("plan_cache", vec![1.0, 2.0, 1.0, 1.0, 0.0]);
    b.span(
        "db_query",
        vec![5.0, 35.0, 3.0, 1.0, 1.0],
        vec![index_scan, row_fetch, plan_cache],
    )
}

fn build_template_render(b: &mut SpanNodeBuilder) -> SpanNode {
    let flex_calc = b.leaf("flex_calc", vec![9.0, 0.0, 1.0, 0.0, 0.0]);
    let paint = b.leaf("paint", vec![6.0, 0.0, 1.0, 1.0, 0.0]);
    let layout_pass = b.span(
        "layout_pass",
        vec![15.0, 0.0, 2.0, 1.0, 0.0],
        vec![flex_calc, paint],
    );
    let hydration = b.leaf("hydration", vec![3.0, 1.0, 2.0, 0.0, 1.0]);
    let minify = b.leaf("minify", vec![2.0, 1.0, 1.0, 1.0, 0.0]);
    b.span(
        "template_render",
        vec![20.0, 2.0, 5.0, 2.0, 1.0],
        vec![layout_pass, hydration, minify],
    )
}

fn build_auth_check(b: &mut SpanNodeBuilder) -> SpanNode {
    let token_verify = b.leaf("token_verify", vec![2.0, 1.0, 2.0, 1.0, 2.0]);
    let session_load = b.leaf("session_load", vec![1.0, 4.0, 2.0, 0.0, 0.0]);
    b.span(
        "auth_check",
        vec![3.0, 5.0, 4.0, 1.0, 2.0],
        vec![token_verify, session_load],
    )
}

// --- Scenario construction helpers ---

fn add_one_level_scenario(catalog: &mut ScenarioCatalog<FlameGraph>) {
    let mut state = test_flame_graph();
    let first_child_id = state.root.children[0].id;
    state.path = vec![state.root.id, first_child_id];
    state.cursor = 1;
    catalog.add(Scenario {
        name: "one-level",
        description: "Root expanded, children visible",
        state,
        inputs: vec![],
    });
}

fn add_deep_scenario(catalog: &mut ScenarioCatalog<FlameGraph>) {
    let mut state = test_flame_graph();
    let first_child_id = state.root.children[0].id;
    let first_grandchild_id = state.root.children[0].children[0].id;
    state.path = vec![state.root.id, first_child_id, first_grandchild_id];
    state.cursor = 2;
    catalog.add(Scenario {
        name: "deep",
        description: "Two levels deep",
        state,
        inputs: vec![],
    });
}

fn add_legend_scenario(catalog: &mut ScenarioCatalog<FlameGraph>) {
    let mut state = test_flame_graph();
    let first_child_id = state.root.children[0].id;
    state.path = vec![state.root.id, first_child_id];
    state.selected_for_legend = Some(first_child_id);
    state.cursor = 1;
    catalog.add(Scenario {
        name: "with-legend",
        description: "Legend visible for first child",
        state,
        inputs: vec![],
    });
}

fn add_lower_sibling_scenario(catalog: &mut ScenarioCatalog<FlameGraph>) {
    let mut state = test_flame_graph();
    let third_child_id = state.root.children[2].id;
    state.path = vec![state.root.id, third_child_id];
    state.cursor = 3;
    catalog.add(Scenario {
        name: "lower-sibling",
        description: "Cursor on a lower sibling to verify summary visibility in narrow layouts",
        state,
        inputs: vec![],
    });
}

fn add_focused_mid_scenario(catalog: &mut ScenarioCatalog<FlameGraph>) {
    let mut state = test_flame_graph();
    // Focus on db_query (mid-level with children).
    let db_query_id = state.root.children[0].id;
    let first_grandchild_id = state.root.children[0].children[0].id;
    state.path = vec![state.root.id, db_query_id, first_grandchild_id];
    state.focus = Some(db_query_id);
    state.cursor = 0;
    state.selected_for_legend = None;
    catalog.add(Scenario {
        name: "focused-mid",
        description: "Focus on db_query, expand to first grandchild",
        state,
        inputs: vec![],
    });
}

fn add_focused_leaf_scenario(catalog: &mut ScenarioCatalog<FlameGraph>) {
    let mut state = test_flame_graph();
    // Focus on logging (leaf, no children).
    let logging_id = state.root.children[3].id;
    state.path = vec![state.root.id, logging_id];
    state.focus = Some(logging_id);
    state.cursor = 0;
    state.selected_for_legend = None;
    catalog.add(Scenario {
        name: "focused-leaf",
        description: "Focus on logging (leaf node, siblings below)",
        state,
        inputs: vec![],
    });
}

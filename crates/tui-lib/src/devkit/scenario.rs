use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::Surface;
use crate::input::KeyResult;

#[derive(Clone, Copy)]
enum RenderOutput {
    Plain,
    Styled,
}

#[derive(Clone, Copy)]
enum ScenarioPhase {
    Initial,
    AfterInputs,
}

/// A named piece of state for rendering.
pub struct Scenario<S> {
    pub name: &'static str,
    pub description: &'static str,
    pub state: S,
    /// Key events to replay against this scenario's state.
    /// Empty = static scenario (render-only).
    pub inputs: Vec<KeyEvent>,
}

/// A collection of scenarios with a shared render function.
pub struct ScenarioCatalog<S> {
    scenarios: Vec<Scenario<S>>,
    render_fn: fn(&S, Rect, &mut Buffer),
    /// Apply a key event to state. None = catalog is render-only.
    apply_fn: Option<fn(&mut S, &KeyEvent) -> KeyResult>,
    /// Returns the widget's current context breadcrumb segments.
    context_fn: Option<fn(&S) -> Vec<&'static str>>,
}

impl<S> ScenarioCatalog<S> {
    /// Create a new catalog with the given render function.
    ///
    /// The resulting catalog is render-only (no input handling).
    pub fn new(render_fn: fn(&S, Rect, &mut Buffer)) -> Self {
        Self {
            scenarios: Vec::new(),
            render_fn,
            apply_fn: None,
            context_fn: None,
        }
    }

    /// Create an interactive catalog with both render and apply functions.
    pub fn new_interactive(
        render_fn: fn(&S, Rect, &mut Buffer),
        apply_fn: fn(&mut S, &KeyEvent) -> KeyResult,
    ) -> Self {
        Self {
            scenarios: Vec::new(),
            render_fn,
            apply_fn: Some(apply_fn),
            context_fn: None,
        }
    }

    /// Set a context function that returns breadcrumb segments for the widget.
    pub fn with_context_fn(mut self, f: fn(&S) -> Vec<&'static str>) -> Self {
        self.context_fn = Some(f);
        self
    }

    /// Returns the widget's current context breadcrumb segments.
    pub fn context(&self, state: &S) -> Vec<&'static str> {
        self.context_fn.map_or_else(Vec::new, |f| f(state))
    }

    /// Whether this catalog supports input handling.
    pub fn is_interactive(&self) -> bool {
        self.apply_fn.is_some()
    }

    /// Apply a key event to the given state.
    /// Returns `Consumed` if the event was handled, `Ignored` otherwise.
    /// Returns `Ignored` if the catalog is not interactive.
    pub fn apply(&self, state: &mut S, event: &KeyEvent) -> KeyResult {
        match self.apply_fn {
            Some(apply) => apply(state, event),
            None => KeyResult::Ignored,
        }
    }

    /// Add a scenario to the catalog.
    pub fn add(&mut self, scenario: Scenario<S>) {
        self.scenarios.push(scenario);
    }

    /// Render one scenario to styled text via Surface.
    pub fn render_to_styled_text(&self, index: usize, width: u16, height: u16) -> String {
        let surface = self.render_to_surface(&self.scenarios[index].state, width, height);
        surface.to_styled_text()
    }

    fn render_to_surface(&self, state: &S, width: u16, height: u16) -> Surface {
        let mut surface = Surface::new(width, height);
        let area = Rect::new(0, 0, width, height);
        (self.render_fn)(state, area, surface.buffer_mut());
        surface
    }

    fn render_output(
        &self,
        index: usize,
        width: u16,
        height: u16,
        output: RenderOutput,
        phase: ScenarioPhase,
    ) -> String
    where
        S: Clone,
    {
        let surface = match phase {
            ScenarioPhase::Initial => {
                self.render_to_surface(&self.scenarios[index].state, width, height)
            }
            ScenarioPhase::AfterInputs => {
                let state = self.state_for_phase(index);
                self.render_to_surface(&state, width, height)
            }
        };
        match output {
            RenderOutput::Plain => surface.to_text(),
            RenderOutput::Styled => surface.to_styled_text(),
        }
    }

    fn state_for_phase(&self, index: usize) -> S
    where
        S: Clone,
    {
        let scenario = &self.scenarios[index];
        let mut state = scenario.state.clone();
        if let Some(apply) = self.apply_fn {
            for input in &scenario.inputs {
                apply(&mut state, input);
            }
        }
        state
    }

    /// Render a scenario directly into a buffer area.
    ///
    /// Used by the playground to render into a live terminal frame.
    pub fn render_into(&self, index: usize, area: Rect, buf: &mut Buffer) {
        let scenario = &self.scenarios[index];
        (self.render_fn)(&scenario.state, area, buf);
    }

    /// Render arbitrary state using the catalog's render function.
    ///
    /// Used by the playground to render mutated state (after replay
    /// or live input) rather than the scenario's original state.
    pub fn render_state(&self, state: &S, area: Rect, buf: &mut Buffer) {
        (self.render_fn)(state, area, buf);
    }

    /// Get a reference to a scenario's initial state.
    pub fn initial_state(&self, index: usize) -> &S {
        &self.scenarios[index].state
    }

    /// Number of scenarios in the catalog.
    pub fn len(&self) -> usize {
        self.scenarios.len()
    }

    /// Whether the catalog contains no scenarios.
    pub fn is_empty(&self) -> bool {
        self.scenarios.is_empty()
    }

    /// Get a scenario's name by index.
    pub fn name(&self, index: usize) -> &str {
        self.scenarios[index].name
    }

    /// Get a scenario's description by index.
    pub fn description(&self, index: usize) -> &str {
        self.scenarios[index].description
    }

    /// Find a scenario's index by name. Returns `None` if not found.
    pub fn scenario_index_by_name(&self, name: &str) -> Option<usize> {
        self.scenarios.iter().position(|s| s.name == name)
    }

    /// Render all scenarios and assert each with insta.
    pub fn assert_all_snapshots(&self, width: u16, height: u16)
    where
        S: Clone,
    {
        self.assert_all_snapshots_with(width, height, RenderOutput::Plain, ScenarioPhase::Initial);
    }

    /// Render all scenarios as styled text and assert each with insta.
    pub fn assert_all_styled_snapshots(&self, width: u16, height: u16)
    where
        S: Clone,
    {
        self.assert_all_snapshots_with(width, height, RenderOutput::Styled, ScenarioPhase::Initial);
    }

    /// Snapshot all scenarios after applying their inputs.
    pub fn assert_all_snapshots_after_inputs(&self, width: u16, height: u16)
    where
        S: Clone,
    {
        self.assert_all_snapshots_with(
            width,
            height,
            RenderOutput::Plain,
            ScenarioPhase::AfterInputs,
        );
    }

    fn assert_all_snapshots_with(
        &self,
        width: u16,
        height: u16,
        output: RenderOutput,
        phase: ScenarioPhase,
    ) where
        S: Clone,
    {
        for i in 0..self.scenarios.len() {
            self.assert_snapshot_with(i, width, height, output, phase);
        }
    }

    fn assert_snapshot_with(
        &self,
        index: usize,
        width: u16,
        height: u16,
        output: RenderOutput,
        phase: ScenarioPhase,
    ) where
        S: Clone,
    {
        let text = self.render_output(index, width, height, output, phase);
        let snap_name = snapshot_name(self.scenarios[index].name, width, height, output, phase);
        insta::assert_snapshot!(snap_name, text);
    }
}

fn snapshot_name(
    scenario_name: &str,
    width: u16,
    height: u16,
    output: RenderOutput,
    phase: ScenarioPhase,
) -> String {
    let mut name = scenario_name.to_string();
    if matches!(output, RenderOutput::Styled) {
        name.push_str("_styled");
    }
    if matches!(phase, ScenarioPhase::AfterInputs) {
        name.push_str("_after");
    }
    format!("{name}_{width}x{height}")
}

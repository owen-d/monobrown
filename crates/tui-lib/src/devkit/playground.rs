//! Interactive terminal playground for scenario catalogs.
//!
//! All catalogs start in **Explorer** mode (browsing). Interactive catalogs
//! can enter scenarios via Enter, which leads to **Live** mode where the
//! widget owns the keyboard (inner-first dispatch). Esc returns to Explorer.
//!
//! Two entry points:
//! - **`run`**: explorer + live input for scenario catalogs.
//! - **`run_animated`**: tick-based animation (orthogonal to input replay).

use std::io;
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::ScenarioCatalog;
use crate::input::KeyResult;
use crate::input::modal::{self, HelpContext, KeyBinding, ModalHelp, ModalInput};
use crate::input::{RenderScheduler, RenderStep, apply_render_effect};
use crate::render::{LayoutRenderable, centered_rect};
use crate::theme;
use crate::widget::hotkey::{HelpPaneRenderable, HotkeyBarRenderable};

// ---------------------------------------------------------------------------
// PlaygroundController -- testable pure state machine
// ---------------------------------------------------------------------------

/// Control flow returned by `PlaygroundController::step_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}

/// Current playground interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaygroundMode {
    /// Browsing scenarios. Playground owns the keyboard.
    Explorer,
    /// Widget owns the keyboard (inner-first dispatch).
    Live,
}

// ---------------------------------------------------------------------------
// ModalInput wiring
// ---------------------------------------------------------------------------

/// Semantic actions the playground can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaygroundIntent {
    Quit,
    NextScenario,
    PrevScenario,
    EnterScenario,
    ExitToExplorer,
    ToggleHelp,
}

/// Lightweight view of playground state for ModalInput (no lifetimes/generics).
struct PlaygroundView {
    mode: PlaygroundMode,
    interactive: bool,
    show_help: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaygroundContext {
    HelpOverlay,
    Explorer,
    Live,
}

/// ModalInput implementation for the playground's own key bindings.
struct PlaygroundModal;

impl ModalInput for PlaygroundModal {
    type Intent = PlaygroundIntent;
    type App = PlaygroundView;
    type Context = PlaygroundContext;

    fn active_context(app: &PlaygroundView) -> PlaygroundContext {
        if app.show_help {
            PlaygroundContext::HelpOverlay
        } else {
            match app.mode {
                PlaygroundMode::Explorer => PlaygroundContext::Explorer,
                PlaygroundMode::Live => PlaygroundContext::Live,
            }
        }
    }

    fn bindings(ctx: &PlaygroundContext) -> Vec<KeyBinding<PlaygroundIntent, PlaygroundView>> {
        match ctx {
            PlaygroundContext::HelpOverlay => help_overlay_bindings(),
            PlaygroundContext::Explorer => explorer_bindings(),
            PlaygroundContext::Live => live_bindings(),
        }
    }
}

impl ModalHelp for PlaygroundModal {
    fn help_contexts() -> Vec<HelpContext<PlaygroundContext>> {
        vec![
            HelpContext {
                title: "Explorer",
                context: PlaygroundContext::Explorer,
            },
            HelpContext {
                title: "Live",
                context: PlaygroundContext::Live,
            },
            HelpContext {
                title: "Help Overlay",
                context: PlaygroundContext::HelpOverlay,
            },
        ]
    }
}

fn help_overlay_bindings() -> Vec<KeyBinding<PlaygroundIntent, PlaygroundView>> {
    vec![
        KeyBinding {
            key_label: "esc",
            action: "close",
            description: "Close help overlay",
            resolve: |key| {
                let unmod = !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                (unmod && key.code == KeyCode::Esc).then_some(PlaygroundIntent::ToggleHelp)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "?",
            action: "close",
            description: "Close help overlay",
            resolve: |key| {
                matches!(key.code, KeyCode::Char('?')).then_some(PlaygroundIntent::ToggleHelp)
            },
            guard: None,
        },
    ]
}

fn explorer_bindings() -> Vec<KeyBinding<PlaygroundIntent, PlaygroundView>> {
    vec![
        KeyBinding {
            key_label: "q/esc",
            action: "quit",
            description: "Quit the playground",
            resolve: |key| {
                let unmod = !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                (unmod && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc))
                    .then_some(PlaygroundIntent::Quit)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "\u{2192}/[]",
            action: "next",
            description: "Next scenario",
            resolve: |key| {
                let unmod = !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                (unmod
                    && matches!(
                        key.code,
                        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(']')
                    ))
                .then_some(PlaygroundIntent::NextScenario)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "\u{2190}/[]",
            action: "prev",
            description: "Previous scenario",
            resolve: |key| {
                let unmod = !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                (unmod
                    && matches!(
                        key.code,
                        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('[')
                    ))
                .then_some(PlaygroundIntent::PrevScenario)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "enter",
            action: "interact",
            description: "Enter scenario",
            resolve: |key| {
                matches!(key.code, KeyCode::Enter).then_some(PlaygroundIntent::EnterScenario)
            },
            guard: Some(|app| app.interactive),
        },
        KeyBinding {
            key_label: "?",
            action: "help",
            description: "Toggle help overlay",
            resolve: |key| {
                matches!(key.code, KeyCode::Char('?')).then_some(PlaygroundIntent::ToggleHelp)
            },
            guard: None,
        },
    ]
}

fn live_bindings() -> Vec<KeyBinding<PlaygroundIntent, PlaygroundView>> {
    vec![
        KeyBinding {
            key_label: "esc",
            action: "back",
            description: "Return to explorer",
            resolve: |key| {
                let unmod = !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                (unmod && key.code == KeyCode::Esc).then_some(PlaygroundIntent::ExitToExplorer)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "?",
            action: "help",
            description: "Toggle help overlay",
            resolve: |key| {
                matches!(key.code, KeyCode::Char('?')).then_some(PlaygroundIntent::ToggleHelp)
            },
            guard: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// PlaygroundController
// ---------------------------------------------------------------------------

/// Pure state machine for the scenario playground.
///
/// Owns the current scenario index, live (mutated) state, and mode.
/// All key dispatch and mode transitions happen here, making the controller
/// fully testable without a terminal.
pub struct PlaygroundController<'a, S> {
    catalog: &'a ScenarioCatalog<S>,
    current: usize,
    live_state: S,
    mode: PlaygroundMode,
    show_help: bool,
}

// Accessors -- no `Clone` bound required.
impl<'a, S> PlaygroundController<'a, S> {
    pub fn mode(&self) -> PlaygroundMode {
        self.mode
    }

    pub fn help_open(&self) -> bool {
        self.show_help
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn live_state(&self) -> &S {
        &self.live_state
    }

    pub fn catalog(&self) -> &ScenarioCatalog<S> {
        self.catalog
    }
}

impl<'a, S: Clone> PlaygroundController<'a, S> {
    /// Create a new controller starting at scenario 0.
    pub fn new(catalog: &'a ScenarioCatalog<S>) -> Self {
        Self {
            live_state: catalog.initial_state(0).clone(),
            catalog,
            current: 0,
            mode: PlaygroundMode::Explorer,
            show_help: false,
        }
    }

    // -- public API ----------------------------------------------------------

    /// Dispatch a key press and return both control flow and render intent.
    pub fn step_key(&mut self, key: &KeyEvent) -> RenderStep<Action> {
        if self.show_help {
            let view = self.view();
            return match modal::resolve::<PlaygroundModal>(&view, key) {
                Some(intent) => self.apply_intent(intent),
                None => RenderStep::new(Action::Continue),
            };
        }

        match self.mode {
            PlaygroundMode::Explorer => {
                let view = self.view();
                match modal::resolve::<PlaygroundModal>(&view, key) {
                    Some(intent) => self.apply_intent(intent),
                    None => RenderStep::new(Action::Continue),
                }
            }
            PlaygroundMode::Live => {
                // Inner-first: widget gets the key.
                match self.catalog.apply(&mut self.live_state, key) {
                    KeyResult::Consumed => RenderStep::schedule_render(Action::Continue),
                    KeyResult::Ignored => {
                        // Widget declined. Playground gets fallback.
                        let view = self.view();
                        match modal::resolve::<PlaygroundModal>(&view, key) {
                            Some(intent) => self.apply_intent(intent),
                            None => RenderStep::new(Action::Continue),
                        }
                    }
                }
            }
        }
    }

    /// Backwards-compatible helper that discards render intent.
    pub fn handle_key(&mut self, key: &KeyEvent) -> Action {
        self.step_key(key).action
    }

    // -- helpers -------------------------------------------------------------

    fn view(&self) -> PlaygroundView {
        PlaygroundView {
            mode: self.mode,
            interactive: self.catalog.is_interactive(),
            show_help: self.show_help,
        }
    }

    fn apply_intent(&mut self, intent: PlaygroundIntent) -> RenderStep<Action> {
        match intent {
            PlaygroundIntent::Quit => RenderStep::new(Action::Quit),
            PlaygroundIntent::NextScenario => {
                self.next_scenario();
                RenderStep::schedule_render(Action::Continue)
            }
            PlaygroundIntent::PrevScenario => {
                self.prev_scenario();
                RenderStep::schedule_render(Action::Continue)
            }
            PlaygroundIntent::EnterScenario => {
                self.mode = PlaygroundMode::Live;
                RenderStep::schedule_render(Action::Continue)
            }
            PlaygroundIntent::ExitToExplorer => {
                self.mode = PlaygroundMode::Explorer;
                RenderStep::schedule_render(Action::Continue)
            }
            PlaygroundIntent::ToggleHelp => {
                self.show_help = !self.show_help;
                RenderStep::schedule_render(Action::Continue)
            }
        }
    }

    // -- scenario navigation -------------------------------------------------

    fn next_scenario(&mut self) {
        let count = self.catalog.len();
        self.current = (self.current + 1) % count;
        self.switch_scenario();
    }

    fn prev_scenario(&mut self) {
        let count = self.catalog.len();
        self.current = (self.current + count - 1) % count;
        self.switch_scenario();
    }

    fn switch_scenario(&mut self) {
        self.live_state = self.catalog.initial_state(self.current).clone();
        // Stay in Explorer -- mode doesn't change on navigation.
    }
}

// ---------------------------------------------------------------------------
// Public entry: run
// ---------------------------------------------------------------------------

/// Run the playground for a scenario catalog.
///
/// All catalogs start in Explorer mode. Interactive catalogs can enter
/// scenarios (live) via Enter. Non-interactive catalogs browse only.
///
/// Requires `S: Clone` so scenarios can be reset to initial state.
pub fn run<S: Clone>(catalog: &ScenarioCatalog<S>) -> Result<(), Box<dyn std::error::Error>> {
    if catalog.is_empty() {
        return Err("catalog has no scenarios".into());
    }

    // Set up terminal.
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, catalog);

    // Restore terminal unconditionally.
    terminal::disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

fn run_loop<S: Clone>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    catalog: &ScenarioCatalog<S>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ctrl = PlaygroundController::new(catalog);
    let now = Instant::now();
    let mut scheduler = RenderScheduler::new(now);
    scheduler.schedule_render_now(now);

    loop {
        let now = Instant::now();
        if scheduler.should_render(now, false) {
            draw(terminal, &ctrl)?;
            scheduler.record_render(now);
        }

        let timeout = scheduler
            .time_until_next_render(Instant::now(), false)
            .unwrap_or(Duration::from_secs(60));

        if !event::poll(timeout)? {
            continue;
        }

        let event = event::read()?;
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let step = ctrl.step_key(&key);
            apply_render_effect(&mut scheduler, Instant::now(), step.effect);
            if step.action == Action::Quit {
                break;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw<S>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ctrl: &PlaygroundController<S>,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Length(2), // top bar
            Constraint::Min(1),    // main area
            Constraint::Length(1), // bottom bar
        ])
        .split(area);
        let view = PlaygroundView {
            mode: ctrl.mode(),
            interactive: ctrl.catalog().is_interactive(),
            show_help: ctrl.help_open(),
        };

        draw_top_bar(frame, chunks[0], ctrl);
        draw_content(frame, chunks[1], ctrl);
        draw_bottom_bar(frame, chunks[2], &view);
        if ctrl.help_open() {
            draw_help_overlay_generic::<PlaygroundModal>(frame, area, " Playground Help ");
        }
    })?;
    Ok(())
}

fn draw_top_bar<S>(frame: &mut ratatui::Frame, area: Rect, ctrl: &PlaygroundController<S>) {
    let catalog = ctrl.catalog();
    let count = catalog.len();
    let current = ctrl.current();
    let name = catalog.name(current);
    let description = catalog.description(current);

    let mut spans = vec![
        Span::styled(
            format!(" {name} [{}/{}]", current + 1, count),
            Style::default().fg(theme::warning()),
        ),
        Span::raw("  "),
        Span::styled(description, Style::default().fg(theme::dim())),
    ];

    // Append mode/context breadcrumb.
    spans.push(Span::raw("  "));
    spans.push(mode_breadcrumb(ctrl));

    let paragraph =
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(paragraph, area);
}

fn mode_breadcrumb<S>(ctrl: &PlaygroundController<S>) -> Span<'static> {
    let text = match ctrl.mode() {
        PlaygroundMode::Explorer => "[explorer]".to_string(),
        PlaygroundMode::Live => {
            let ctx = ctrl.catalog().context(ctrl.live_state());
            if ctx.is_empty() {
                "[live]".to_string()
            } else {
                format!("[live > {}]", ctx.join(" > "))
            }
        }
    };

    let color = match ctrl.mode() {
        PlaygroundMode::Explorer => theme::dim(),
        PlaygroundMode::Live => theme::focus(),
    };

    Span::styled(text, Style::default().fg(color))
}

fn draw_content<S>(frame: &mut ratatui::Frame, area: Rect, ctrl: &PlaygroundController<S>) {
    match ctrl.mode() {
        PlaygroundMode::Explorer => {
            // Render from the catalog's original immutable state.
            ctrl.catalog()
                .render_into(ctrl.current(), area, frame.buffer_mut());
        }
        PlaygroundMode::Live => {
            // Render from the mutated live_state.
            ctrl.catalog()
                .render_state(ctrl.live_state(), area, frame.buffer_mut());
        }
    }
}

fn draw_bottom_bar(frame: &mut ratatui::Frame, area: Rect, view: &PlaygroundView) {
    let hints = modal::hints::<PlaygroundModal>(view);
    HotkeyBarRenderable { hints }.render(area, frame.buffer_mut());
}

pub(super) fn draw_help_overlay_generic<M: ModalHelp>(
    frame: &mut ratatui::Frame,
    full_area: Rect,
    title: &str,
) {
    let sections = modal::all_hints::<M>();
    let area = centered_rect(full_area, 64, 70);
    HelpPaneRenderable {
        title,
        sections: &sections,
        appendix: &[],
    }
    .render(area, frame.buffer_mut());
}

// ---------------------------------------------------------------------------
// Animated playground (unchanged -- orthogonal to input replay)
// ---------------------------------------------------------------------------

/// Run an animated playground with live-updating state.
///
/// The `tick` function advances state by a duration each frame.
/// `step_size` controls how much a single manual step advances.
///
/// Controls:
/// - **Space**: pause / resume
/// - **Right / l**: step forward one frame (when paused)
/// - **Left / h**: step backward one frame (when paused)
/// - **q / Esc**: quit
pub fn run_animated<S>(
    state: S,
    title: &str,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick: fn(&mut S, Duration),
    step_size: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(run_animated_inner(
        state, title, render_fn, tick, None, step_size,
    )?)
}

fn run_animated_inner<S>(
    state: S,
    title: &str,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick: fn(&mut S, Duration),
    apply: Option<fn(&mut S, &KeyEvent)>,
    step_size: Duration,
) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_animated_loop_inner(
        &mut terminal,
        state,
        title,
        render_fn,
        tick,
        apply,
        step_size,
    );

    terminal::disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn draw_animated_inner<S>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &S,
    title: &str,
    paused: bool,
    interactive: bool,
    render_fn: fn(&S, Rect, &mut Buffer),
) -> io::Result<()> {
    let status = if paused { "PAUSED" } else { "PLAYING" };
    terminal.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

        // Top bar.
        let top = Line::from(vec![
            Span::styled(format!(" {title}"), Style::default().fg(theme::warning())),
            Span::raw("  "),
            Span::styled(
                format!("[{status}]"),
                Style::default().fg(if paused {
                    theme::error()
                } else {
                    theme::success()
                }),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(top).block(Block::default().borders(Borders::BOTTOM)),
            chunks[0],
        );

        // Scenario content.
        render_fn(state, chunks[1], frame.buffer_mut());

        // Bottom bar.
        let help = if paused {
            " \u{2190} \u{2192} step  space resume  q quit"
        } else if interactive {
            " space pause  (keys \u{2192} widget)  q quit"
        } else {
            " space pause  q quit"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                help,
                Style::default().fg(theme::dim()),
            ))),
            chunks[2],
        );
    })?;
    Ok(())
}

fn run_animated_loop_inner<S>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut state: S,
    title: &str,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick: fn(&mut S, Duration),
    apply: Option<fn(&mut S, &KeyEvent)>,
    step_size: Duration,
) -> io::Result<()> {
    let interactive = apply.is_some();
    let mut paused = false;
    let mut last_tick = Instant::now();
    let mut scheduler = RenderScheduler::new(last_tick);
    scheduler.schedule_render_now(last_tick);

    loop {
        let animate = !paused;
        let now = Instant::now();
        if scheduler.should_render(now, animate) {
            if animate {
                let dt = now - last_tick;
                last_tick = now;
                tick(&mut state, dt);
            }

            draw_animated_inner(terminal, &state, title, paused, interactive, render_fn)?;
            scheduler.record_render(now);
        }

        let timeout = scheduler
            .time_until_next_render(Instant::now(), animate)
            .unwrap_or(Duration::from_secs(60));

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => {
                    paused = !paused;
                    if !paused {
                        last_tick = Instant::now(); // avoid time jump on resume
                    }
                    scheduler.schedule_render(Instant::now());
                }
                KeyCode::Right | KeyCode::Char('l') if paused => {
                    tick(&mut state, step_size);
                    scheduler.schedule_render(Instant::now());
                }
                KeyCode::Left | KeyCode::Char('h') if paused => {
                    // Step backward not supported; forward only.
                }
                _ => {
                    if let Some(apply_fn) = apply {
                        apply_fn(&mut state, &key);
                        scheduler.schedule_render(Instant::now());
                    }
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Animated interactive playground
// ---------------------------------------------------------------------------

/// Run an animated playground that also accepts widget key input.
///
/// Like `run_animated`, but unrecognized keys are forwarded to `apply`
/// so the widget can handle interactive input (e.g. selection changes).
///
/// Controls:
/// - **Space**: pause / resume
/// - **Right / l**: step forward one frame (when paused)
/// - **Left / h**: step backward one frame (when paused)
/// - **q / Esc**: quit
/// - All other keys: forwarded to `apply`
pub fn run_animated_interactive<S>(
    state: S,
    title: &str,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick: fn(&mut S, Duration),
    apply: fn(&mut S, &KeyEvent),
    step_size: Duration,
) -> io::Result<()> {
    run_animated_inner(state, title, render_fn, tick, Some(apply), step_size)
}

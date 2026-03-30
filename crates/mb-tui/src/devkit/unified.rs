//! Unified playground that combines multiple scenario catalogs and animated
//! entries into a single tabbed session.
//!
//! Type-erases `ScenarioCatalog<S>` via the `CatalogEntry` trait so entries
//! with different state types coexist in one `Vec<Box<dyn CatalogEntry>>`.

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
use super::playground::{PlaygroundMode, draw_help_overlay_generic};
use crate::input::KeyResult;
use crate::input::modal::{self, HelpContext, KeyBinding, ModalHelp, ModalInput};
use crate::input::{RenderScheduler, RenderStep, apply_render_effect};
use crate::render::{Constraints, LayoutRenderable};
use crate::theme;
use crate::widget::hotkey::HotkeyBarRenderable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnifiedContext {
    HelpOverlay,
    Explorer,
    Live,
    Animated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnifiedView {
    mode: PlaygroundMode,
    show_help: bool,
    interactive: bool,
    animated: bool,
    paused: bool,
    multiple_entries: bool,
}

struct UnifiedModal;

impl ModalInput for UnifiedModal {
    type Intent = ();
    type App = UnifiedView;
    type Context = UnifiedContext;

    fn active_context(app: &Self::App) -> Self::Context {
        if app.show_help {
            UnifiedContext::HelpOverlay
        } else if app.animated {
            UnifiedContext::Animated
        } else {
            match app.mode {
                PlaygroundMode::Explorer => UnifiedContext::Explorer,
                PlaygroundMode::Live => UnifiedContext::Live,
            }
        }
    }

    fn bindings(ctx: &Self::Context) -> Vec<KeyBinding<Self::Intent, Self::App>> {
        match ctx {
            UnifiedContext::HelpOverlay => help_overlay_bindings(),
            UnifiedContext::Explorer => explorer_bindings(),
            UnifiedContext::Live => live_bindings(),
            UnifiedContext::Animated => animated_bindings(),
        }
    }
}

impl ModalHelp for UnifiedModal {
    fn help_contexts() -> Vec<HelpContext<UnifiedContext>> {
        vec![
            HelpContext {
                title: "Explorer",
                context: UnifiedContext::Explorer,
            },
            HelpContext {
                title: "Live",
                context: UnifiedContext::Live,
            },
            HelpContext {
                title: "Animated",
                context: UnifiedContext::Animated,
            },
            HelpContext {
                title: "Help Overlay",
                context: UnifiedContext::HelpOverlay,
            },
        ]
    }
}

fn help_overlay_bindings() -> Vec<KeyBinding<(), UnifiedView>> {
    vec![
        KeyBinding {
            key_label: "esc",
            action: "close",
            description: "Close help overlay",
            resolve: |key| (key.code == KeyCode::Esc).then_some(()),
            guard: None,
        },
        KeyBinding {
            key_label: "?",
            action: "close",
            description: "Close help overlay",
            resolve: |key| matches!(key.code, KeyCode::Char('?')).then_some(()),
            guard: None,
        },
    ]
}

fn explorer_bindings() -> Vec<KeyBinding<(), UnifiedView>> {
    vec![
        KeyBinding {
            key_label: "tab",
            action: "switch",
            description: "Switch to next playground entry",
            resolve: |key| (key.code == KeyCode::Tab).then_some(()),
            guard: Some(|view| view.multiple_entries),
        },
        KeyBinding {
            key_label: "shift-tab",
            action: "switch",
            description: "Switch to previous playground entry",
            resolve: |key| (key.code == KeyCode::BackTab).then_some(()),
            guard: Some(|view| view.multiple_entries),
        },
        KeyBinding {
            key_label: "→/[]",
            action: "next",
            description: "Advance to the next scenario",
            resolve: |key| {
                matches!(
                    key.code,
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(']')
                )
                .then_some(())
            },
            guard: None,
        },
        KeyBinding {
            key_label: "←/[]",
            action: "prev",
            description: "Return to the previous scenario",
            resolve: |key| {
                matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('[')
                )
                .then_some(())
            },
            guard: None,
        },
        KeyBinding {
            key_label: "enter",
            action: "interact",
            description: "Enter live mode for the current scenario",
            resolve: |key| (key.code == KeyCode::Enter).then_some(()),
            guard: Some(|view| view.interactive),
        },
        KeyBinding {
            key_label: "q/esc",
            action: "quit",
            description: "Quit the unified playground",
            resolve: |key| matches!(key.code, KeyCode::Char('q') | KeyCode::Esc).then_some(()),
            guard: None,
        },
        KeyBinding {
            key_label: "?",
            action: "help",
            description: "Toggle the help overlay",
            resolve: |key| matches!(key.code, KeyCode::Char('?')).then_some(()),
            guard: None,
        },
    ]
}

fn live_bindings() -> Vec<KeyBinding<(), UnifiedView>> {
    vec![
        KeyBinding {
            key_label: "tab",
            action: "switch",
            description: "Switch to next playground entry",
            resolve: |key| (key.code == KeyCode::Tab).then_some(()),
            guard: Some(|view| view.multiple_entries),
        },
        KeyBinding {
            key_label: "shift-tab",
            action: "switch",
            description: "Switch to previous playground entry",
            resolve: |key| (key.code == KeyCode::BackTab).then_some(()),
            guard: Some(|view| view.multiple_entries),
        },
        KeyBinding {
            key_label: "esc",
            action: "back",
            description: "Return to explorer mode when the widget yields Esc",
            resolve: |key| (key.code == KeyCode::Esc).then_some(()),
            guard: None,
        },
        KeyBinding {
            key_label: "?",
            action: "help",
            description: "Toggle the help overlay",
            resolve: |key| matches!(key.code, KeyCode::Char('?')).then_some(()),
            guard: None,
        },
    ]
}

fn animated_bindings() -> Vec<KeyBinding<(), UnifiedView>> {
    vec![
        KeyBinding {
            key_label: "tab",
            action: "switch",
            description: "Switch to next playground entry",
            resolve: |key| (key.code == KeyCode::Tab).then_some(()),
            guard: Some(|view| view.multiple_entries),
        },
        KeyBinding {
            key_label: "shift-tab",
            action: "switch",
            description: "Switch to previous playground entry",
            resolve: |key| (key.code == KeyCode::BackTab).then_some(()),
            guard: Some(|view| view.multiple_entries),
        },
        KeyBinding {
            key_label: "space",
            action: "pause",
            description: "Pause or resume the active animation",
            resolve: |key| matches!(key.code, KeyCode::Char(' ')).then_some(()),
            guard: None,
        },
        KeyBinding {
            key_label: "→",
            action: "step",
            description: "Advance one frame while paused",
            resolve: |key| matches!(key.code, KeyCode::Right | KeyCode::Char('l')).then_some(()),
            guard: Some(|view| view.paused),
        },
        KeyBinding {
            key_label: "q/esc",
            action: "quit",
            description: "Quit the unified playground",
            resolve: |key| matches!(key.code, KeyCode::Char('q') | KeyCode::Esc).then_some(()),
            guard: None,
        },
        KeyBinding {
            key_label: "?",
            action: "help",
            description: "Toggle the help overlay",
            resolve: |key| matches!(key.code, KeyCode::Char('?')).then_some(()),
            guard: None,
        },
    ]
}

fn unified_view(entry: &dyn CatalogEntry, entry_count: usize, show_help: bool) -> UnifiedView {
    UnifiedView {
        mode: entry.mode(),
        show_help,
        interactive: entry.is_interactive(),
        animated: entry.is_animated(),
        paused: entry.is_paused(),
        multiple_entries: entry_count > 1,
    }
}

// ---------------------------------------------------------------------------
// CatalogEntry -- type-erased interface for one tab
// ---------------------------------------------------------------------------

/// Type-erased interface for a single playground tab.
///
/// Implemented by `CatalogEntryImpl` (interactive scenario catalogs) and
/// `AnimatedEntryImpl` (tick-driven animated widgets).
pub trait CatalogEntry {
    /// Display name shown in the tab bar.
    fn catalog_name(&self) -> &str;
    /// Handle a key press. Returns `KeyResult` so the unified shell can
    /// detect unhandled keys.
    fn handle_key(&mut self, key: &KeyEvent) -> KeyResult;
    /// Handle a key press and emit render intent for the outer loop.
    fn step_key(&mut self, key: &KeyEvent) -> RenderStep<KeyResult> {
        let action = self.handle_key(key);
        if action == KeyResult::Consumed {
            RenderStep::schedule_render(action)
        } else {
            RenderStep::new(action)
        }
    }
    /// Render the current content into the given area.
    fn render(&self, area: Rect, buf: &mut Buffer);
    /// Current interaction mode (Explorer or Live).
    fn mode(&self) -> PlaygroundMode;
    /// Whether the current entry can enter live mode.
    fn is_interactive(&self) -> bool {
        false
    }
    /// Index of the currently selected scenario.
    fn current_index(&self) -> usize;
    /// Total number of scenarios in this entry.
    fn scenario_count(&self) -> usize;
    /// Name of the current scenario.
    fn scenario_name(&self) -> &str;
    /// Description of the current scenario.
    fn scenario_description(&self) -> &str;
    /// Return to explorer mode, resetting live state.
    fn reset_to_explorer(&mut self);
    /// Advance animated state. No-op for non-animated entries.
    fn tick(&mut self) {}
    /// Whether this entry requires tick-based animation.
    fn is_animated(&self) -> bool {
        false
    }
    /// Whether this entry is currently paused.
    fn is_paused(&self) -> bool {
        false
    }
    /// Context breadcrumb segments for the current state.
    fn context_breadcrumb(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// CatalogEntryImpl -- wraps ScenarioCatalog<S>
// ---------------------------------------------------------------------------

/// Wraps a `ScenarioCatalog<S>` with inline navigation logic.
///
/// Reimplements the simple explorer/live state machine (~40 lines) rather
/// than embedding `PlaygroundController`, which borrows the catalog and
/// would cause self-referential ownership issues.
struct CatalogEntryImpl<S: Clone + 'static> {
    name: &'static str,
    catalog: ScenarioCatalog<S>,
    current: usize,
    live_state: S,
    mode: PlaygroundMode,
}

impl<S: Clone + 'static> CatalogEntryImpl<S> {
    fn new(name: &'static str, catalog: ScenarioCatalog<S>) -> Self {
        let live_state = catalog.initial_state(0).clone();
        Self {
            name,
            catalog,
            current: 0,
            live_state,
            mode: PlaygroundMode::Explorer,
        }
    }

    fn next_scenario(&mut self) {
        let count = self.catalog.len();
        self.current = (self.current + 1) % count;
        self.live_state = self.catalog.initial_state(self.current).clone();
    }

    fn prev_scenario(&mut self) {
        let count = self.catalog.len();
        self.current = (self.current + count - 1) % count;
        self.live_state = self.catalog.initial_state(self.current).clone();
    }
}

impl<S: Clone + 'static> CatalogEntry for CatalogEntryImpl<S> {
    fn catalog_name(&self) -> &str {
        self.name
    }

    fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
        let unmod = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match self.mode {
            PlaygroundMode::Explorer => handle_explorer_key(self, key, unmod),
            PlaygroundMode::Live => {
                // Inner-first: widget gets the key.
                match self.catalog.apply(&mut self.live_state, key) {
                    KeyResult::Consumed => KeyResult::Consumed,
                    KeyResult::Ignored => {
                        // Fallback: check for Esc to exit live mode.
                        if unmod && key.code == KeyCode::Esc {
                            self.mode = PlaygroundMode::Explorer;
                            KeyResult::Consumed
                        } else {
                            KeyResult::Ignored
                        }
                    }
                }
            }
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        match self.mode {
            PlaygroundMode::Explorer => {
                self.catalog.render_into(self.current, area, buf);
            }
            PlaygroundMode::Live => {
                self.catalog.render_state(&self.live_state, area, buf);
            }
        }
    }

    fn mode(&self) -> PlaygroundMode {
        self.mode
    }

    fn is_interactive(&self) -> bool {
        self.catalog.is_interactive()
    }

    fn current_index(&self) -> usize {
        self.current
    }

    fn scenario_count(&self) -> usize {
        self.catalog.len()
    }

    fn scenario_name(&self) -> &str {
        self.catalog.name(self.current)
    }

    fn scenario_description(&self) -> &str {
        self.catalog.description(self.current)
    }

    fn reset_to_explorer(&mut self) {
        self.mode = PlaygroundMode::Explorer;
        self.live_state = self.catalog.initial_state(self.current).clone();
    }

    fn context_breadcrumb(&self) -> Vec<&'static str> {
        self.catalog.context(&self.live_state)
    }
}

/// Explorer-mode key handling extracted to stay within function length limits.
fn handle_explorer_key<S: Clone + 'static>(
    entry: &mut CatalogEntryImpl<S>,
    key: &KeyEvent,
    unmod: bool,
) -> KeyResult {
    if !unmod {
        return KeyResult::Ignored;
    }
    match key.code {
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(']') => {
            entry.next_scenario();
            KeyResult::Consumed
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('[') => {
            entry.prev_scenario();
            KeyResult::Consumed
        }
        KeyCode::Enter if entry.catalog.is_interactive() => {
            entry.mode = PlaygroundMode::Live;
            KeyResult::Consumed
        }
        _ => KeyResult::Ignored,
    }
}

// ---------------------------------------------------------------------------
// AnimatedEntryImpl -- tick-driven animated widget
// ---------------------------------------------------------------------------

/// Wraps an animated widget with a single scenario that ticks over time.
struct AnimatedEntryImpl<S: Clone + 'static> {
    name: &'static str,
    state: S,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick_fn: fn(&mut S, Duration),
    apply_fn: Option<fn(&mut S, &KeyEvent)>,
    paused: bool,
    step_size: Duration,
    last_tick: Instant,
}

impl<S: Clone + 'static> CatalogEntry for AnimatedEntryImpl<S> {
    fn catalog_name(&self) -> &str {
        self.name
    }

    fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
        // Pause/step keys first.
        match key.code {
            KeyCode::Char(' ') => {
                let was_paused = self.paused;
                self.paused = !self.paused;
                // Reset timing on unpause to avoid time jump.
                if was_paused && !self.paused {
                    self.last_tick = Instant::now();
                }
                return KeyResult::Consumed;
            }
            KeyCode::Right | KeyCode::Char('l') if self.paused => {
                (self.tick_fn)(&mut self.state, self.step_size);
                return KeyResult::Consumed;
            }
            _ => {}
        }
        // Delegate to apply_fn for widget-specific keys.
        if let Some(apply) = self.apply_fn {
            (apply)(&mut self.state, key);
            KeyResult::Consumed
        } else {
            KeyResult::Ignored
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        (self.render_fn)(&self.state, area, buf);
    }

    fn mode(&self) -> PlaygroundMode {
        // Animated entries are always conceptually "live".
        PlaygroundMode::Live
    }

    fn current_index(&self) -> usize {
        0
    }

    fn scenario_count(&self) -> usize {
        1
    }

    fn scenario_name(&self) -> &str {
        self.name
    }

    fn scenario_description(&self) -> &str {
        if self.paused { "PAUSED" } else { "PLAYING" }
    }

    fn reset_to_explorer(&mut self) {
        // Animated entries have no explorer mode; pause instead.
        self.paused = true;
    }

    fn tick(&mut self) {
        if !self.paused {
            let now = Instant::now();
            let dt = now - self.last_tick;
            self.last_tick = now;
            (self.tick_fn)(&mut self.state, dt);
        }
    }

    fn is_animated(&self) -> bool {
        true
    }

    fn is_paused(&self) -> bool {
        self.paused
    }
}

// ---------------------------------------------------------------------------
// Public constructors
// ---------------------------------------------------------------------------

/// Create a catalog-based entry for the unified playground.
pub fn entry<S: Clone + 'static>(
    name: &'static str,
    catalog: ScenarioCatalog<S>,
) -> Box<dyn CatalogEntry> {
    assert!(
        !catalog.is_empty(),
        "catalog must have at least one scenario"
    );
    Box::new(CatalogEntryImpl::new(name, catalog))
}

/// Create an animated entry for the unified playground.
pub fn animated_entry<S: Clone + 'static>(
    name: &'static str,
    state: S,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick_fn: fn(&mut S, Duration),
    step_size: Duration,
) -> Box<dyn CatalogEntry> {
    Box::new(AnimatedEntryImpl {
        name,
        state,
        render_fn,
        tick_fn,
        apply_fn: None,
        paused: false,
        step_size,
        last_tick: Instant::now(),
    })
}

/// Create an animated interactive entry for the unified playground.
///
/// Like `animated_entry` but with an `apply_fn` that receives keyboard input,
/// allowing the widget to respond to user interaction beyond pause/step.
pub fn animated_interactive_entry<S: Clone + 'static>(
    name: &'static str,
    state: S,
    render_fn: fn(&S, Rect, &mut Buffer),
    tick_fn: fn(&mut S, Duration),
    apply_fn: fn(&mut S, &KeyEvent),
    step_size: Duration,
) -> Box<dyn CatalogEntry> {
    Box::new(AnimatedEntryImpl {
        name,
        state,
        render_fn,
        tick_fn,
        apply_fn: Some(apply_fn),
        paused: false,
        step_size,
        last_tick: Instant::now(),
    })
}

// ---------------------------------------------------------------------------
// Unified event loop
// ---------------------------------------------------------------------------

/// Run the unified playground with all provided entries in a tabbed session.
///
/// Tab/Shift-Tab switch between entries. q/Esc at the top level quits.
/// Animated entries tick at ~60fps. Catalog entries support explorer/live
/// modes just like the standalone playground.
pub fn run_unified(entries: Vec<Box<dyn CatalogEntry>>) -> io::Result<()> {
    assert!(!entries.is_empty(), "must provide at least one entry");

    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = unified_loop(&mut terminal, entries);

    terminal::disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn unified_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut entries: Vec<Box<dyn CatalogEntry>>,
) -> io::Result<()> {
    let mut active: usize = 0;
    let mut show_help = false;
    let now = Instant::now();
    let mut scheduler = RenderScheduler::new(now);
    scheduler.schedule_render_now(now);

    loop {
        let animate = entries
            .iter()
            .any(|entry| entry.is_animated() && !entry.is_paused());
        let now = Instant::now();
        if scheduler.should_render(now, animate) {
            if animate {
                for entry in &mut entries {
                    entry.tick();
                }
            }

            draw_unified(terminal, &entries, active, show_help)?;
            scheduler.record_render(now);
        }

        let timeout = scheduler
            .time_until_next_render(Instant::now(), animate)
            .unwrap_or(Duration::from_secs(60));

        if !event::poll(timeout)? {
            continue;
        }

        let event = event::read()?;
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let unmod = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        if show_help {
            if unmod && matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                show_help = false;
                scheduler.schedule_render(Instant::now());
            }
            continue;
        }

        if unmod && key.code == KeyCode::Char('?') {
            show_help = true;
            scheduler.schedule_render(Instant::now());
            continue;
        }

        // Top-level keys: tab switching, quit.
        match dispatch_top_level(&key, &mut entries, &mut active) {
            TopAction::Continue => {}
            TopAction::Quit => break,
            TopAction::Handled => {
                scheduler.schedule_render(Instant::now());
                continue;
            }
        }

        let step = entries[active].step_key(&key);
        apply_render_effect(&mut scheduler, Instant::now(), step.effect);
    }

    Ok(())
}

enum TopAction {
    Continue,
    Handled,
    Quit,
}

/// Handle top-level keys: tab switching and quit.
fn dispatch_top_level(
    key: &KeyEvent,
    entries: &mut [Box<dyn CatalogEntry>],
    active: &mut usize,
) -> TopAction {
    let count = entries.len();
    let unmod = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

    match key.code {
        // Quit from explorer mode (or animated entries on q).
        KeyCode::Char('q') if unmod => {
            let mode = entries[*active].mode();
            let is_animated = entries[*active].is_animated();
            if mode == PlaygroundMode::Explorer || is_animated {
                return TopAction::Quit;
            }
            TopAction::Continue
        }
        KeyCode::Esc if unmod => {
            let mode = entries[*active].mode();
            if mode == PlaygroundMode::Explorer {
                return TopAction::Quit;
            }
            TopAction::Continue
        }
        // Tab: next catalog.
        KeyCode::Tab if count > 1 => {
            entries[*active].reset_to_explorer();
            *active = (*active + 1) % count;
            TopAction::Handled
        }
        // Shift-Tab: previous catalog.
        KeyCode::BackTab if count > 1 => {
            entries[*active].reset_to_explorer();
            *active = (*active + count - 1) % count;
            TopAction::Handled
        }
        _ => TopAction::Continue,
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw_unified(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    entries: &[Box<dyn CatalogEntry>],
    active: usize,
    show_help: bool,
) -> io::Result<()> {
    terminal.draw(|frame| {
        let area = frame.area();
        let view = unified_view(&*entries[active], entries.len(), show_help);
        let hints = modal::hints::<UnifiedModal>(&view);
        let hints_bar = HotkeyBarRenderable { hints };
        let footer_height = hints_bar
            .measure(Constraints::loose(area.width, area.height))
            .height
            .max(1);
        let chunks = Layout::vertical([
            Constraint::Length(2),             // tab bar + scenario info
            Constraint::Min(1),                // content
            Constraint::Length(footer_height), // key hints
        ])
        .split(area);

        draw_tab_bar(frame, chunks[0], entries, active);
        entries[active].render(chunks[1], frame.buffer_mut());
        draw_hints_bar(frame, chunks[2], view);
        if show_help {
            draw_help_overlay(frame, area);
        }
    })?;
    Ok(())
}

fn draw_tab_bar(
    frame: &mut ratatui::Frame,
    area: Rect,
    entries: &[Box<dyn CatalogEntry>],
    active: usize,
) {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    // Render tab names.
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(theme::border())));
        }
        let style = if i == active {
            Style::default().fg(theme::focus())
        } else {
            Style::default().fg(theme::dim())
        };
        spans.push(Span::styled(entry.catalog_name(), style));
    }

    // Separator before scenario info.
    spans.push(Span::styled(
        "  \u{2502}  ",
        Style::default().fg(theme::border()),
    ));

    // Scenario info for active entry.
    let entry = &entries[active];
    let idx = entry.current_index();
    let count = entry.scenario_count();
    let name = entry.scenario_name();
    let desc = entry.scenario_description();

    spans.push(Span::styled(
        format!("{name} [{}/{}]", idx + 1, count),
        Style::default().fg(theme::warning()),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(desc, Style::default().fg(theme::dim())));

    // Mode breadcrumb.
    spans.push(Span::raw("  "));
    spans.push(mode_span(&**entry));

    let paragraph =
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(paragraph, area);
}

/// Build a styled span showing the current mode (explorer/live/paused).
fn mode_span(entry: &dyn CatalogEntry) -> Span<'static> {
    if entry.is_animated() {
        let (text, color) = if entry.is_paused() {
            ("[paused]", theme::error())
        } else {
            ("[playing]", theme::success())
        };
        return Span::styled(text, Style::default().fg(color));
    }

    match entry.mode() {
        PlaygroundMode::Explorer => Span::styled("[explorer]", Style::default().fg(theme::dim())),
        PlaygroundMode::Live => {
            let ctx = entry.context_breadcrumb();
            let text = if ctx.is_empty() {
                "[live]".to_string()
            } else {
                format!("[live > {}]", ctx.join(" > "))
            };
            Span::styled(text, Style::default().fg(theme::focus()))
        }
    }
}

fn draw_hints_bar(frame: &mut ratatui::Frame, area: Rect, view: UnifiedView) {
    let hints = modal::hints::<UnifiedModal>(&view);
    HotkeyBarRenderable { hints }.render(area, frame.buffer_mut());
}

fn draw_help_overlay(frame: &mut ratatui::Frame, full_area: Rect) {
    draw_help_overlay_generic::<UnifiedModal>(frame, full_area, " Unified Playground Help ");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devkit::Scenario;
    use crate::input::RenderEffect;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn test_render(_state: &u8, _area: Rect, _buf: &mut Buffer) {}

    fn test_apply(_state: &mut u8, _event: &KeyEvent) -> KeyResult {
        KeyResult::Consumed
    }

    fn ignored_apply(_state: &mut u8, _event: &KeyEvent) -> KeyResult {
        KeyResult::Ignored
    }

    #[test]
    fn explorer_hints_include_navigation_and_help() {
        let view = UnifiedView {
            mode: PlaygroundMode::Explorer,
            show_help: false,
            interactive: true,
            animated: false,
            paused: false,
            multiple_entries: true,
        };

        let hints = modal::hints::<UnifiedModal>(&view);
        assert!(
            hints
                .iter()
                .any(|hint| hint.key == "tab" && hint.action == "switch")
        );
        assert!(
            hints
                .iter()
                .any(|hint| hint.key == "enter" && hint.action == "interact")
        );
        assert!(
            hints
                .iter()
                .any(|hint| hint.key == "?" && hint.action == "help")
        );
    }

    #[test]
    fn live_hints_exclude_quit() {
        let view = UnifiedView {
            mode: PlaygroundMode::Live,
            show_help: false,
            interactive: true,
            animated: false,
            paused: false,
            multiple_entries: false,
        };

        let hints = modal::hints::<UnifiedModal>(&view);
        assert!(
            hints
                .iter()
                .any(|hint| hint.key == "esc" && hint.action == "back")
        );
        assert!(!hints.iter().any(|hint| hint.action == "quit"));
    }

    #[test]
    fn animated_hints_show_step_only_when_paused() {
        let paused_view = UnifiedView {
            mode: PlaygroundMode::Live,
            show_help: false,
            interactive: false,
            animated: true,
            paused: true,
            multiple_entries: false,
        };
        let playing_view = UnifiedView {
            paused: false,
            ..paused_view
        };

        let paused_hints = modal::hints::<UnifiedModal>(&paused_view);
        assert!(paused_hints.iter().any(|hint| hint.action == "step"));

        let playing_hints = modal::hints::<UnifiedModal>(&playing_view);
        assert!(!playing_hints.iter().any(|hint| hint.action == "step"));
    }

    #[test]
    fn step_key_emits_render_when_entry_consumes_input() {
        let mut catalog = ScenarioCatalog::new_interactive(test_render, test_apply);
        catalog.add(Scenario {
            name: "default",
            description: "",
            state: 0,
            inputs: vec![],
        });

        let mut entry = CatalogEntryImpl::new("demo", catalog);
        entry.mode = PlaygroundMode::Live;

        let step = entry.step_key(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

        assert_eq!(step.action, KeyResult::Consumed);
        assert_eq!(step.effect, Some(RenderEffect::ScheduleRender));
    }

    #[test]
    fn step_key_skips_render_when_entry_ignores_input() {
        let mut catalog = ScenarioCatalog::new_interactive(test_render, ignored_apply);
        catalog.add(Scenario {
            name: "default",
            description: "",
            state: 0,
            inputs: vec![],
        });

        let mut entry = CatalogEntryImpl::new("demo", catalog);
        entry.mode = PlaygroundMode::Live;

        let step = entry.step_key(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

        assert_eq!(step.action, KeyResult::Ignored);
        assert_eq!(step.effect, None);
    }
}

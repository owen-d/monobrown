//! Interactive flamegraph exploration of code-loss heatmap results.
//!
//! Converts the hierarchical `HeatmapTreeNode` rollup into a `SpanNode`
//! tree consumable by the `mb_tui` flame graph widget, then runs a
//! full-screen TUI for interactive drill-down.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use mb_tui::highlight::highlight_code;
use mb_tui::input::modal::{self, KeyBinding, ModalInput};
use mb_tui::input::{KeyResult, RenderScheduler, RenderStep, apply_render_effect};
use mb_tui::render::{LayoutPagerView, LayoutRenderable, LayoutRenderableItem, render_pane_frame};
use mb_tui::widget::flame_graph::{
    CostType, FlameGraph, SpanId, SpanNode, SpanNodeBuilder, render_flame_graph_mut,
};
use mb_tui::widget::hotkey::HotkeyBarRenderable;

use crate::HeatmapEntry;
use crate::metrics::ScopeSegment;
use crate::rollup::HeatmapTreeNode;

// ---------------------------------------------------------------------------
// Dimension constants
// ---------------------------------------------------------------------------

/// Unified dimension table: (lookup key, display label, color).
///
/// - **key**: matches `HeatmapTreeNode.dimension_responsibilities` BTreeMap keys.
/// - **label**: short name shown in the flamegraph legend.
/// - **color**: precomputed from evenly-spaced hues (72° apart, base 30°,
///   S=0.65, L=0.60). All pass WCAG AA large-text (3.0:1) against black.
const DIMENSIONS: [(&str, &str, Color); 5] = [
    ("bloat", "bloat", Color::Rgb(219, 153, 87)),
    ("state_cardinality", "state", Color::Rgb(126, 219, 87)),
    ("duplication", "duplication", Color::Rgb(87, 219, 206)),
    ("code_economy", "economy", Color::Rgb(100, 87, 219)),
    ("coupling_density", "coupling", Color::Rgb(219, 87, 180)),
];

const DETAILS_FOCUS_CONTEXT: usize = 4;
const MIN_FLAME_GRAPH_HEIGHT: u16 = 4;
const MIN_STACKED_DETAILS_HEIGHT: u16 = 6;
const MIN_SPLIT_GRAPH_WIDTH: u16 = 32;
const MIN_SPLIT_DETAILS_WIDTH: u16 = 42;

enum DetailsLayout {
    Hidden,
    Stacked { graph: Rect, details: Rect },
    Split { graph: Rect, details: Rect },
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// Cost type descriptors for the flamegraph legend.
pub fn dimension_cost_types() -> Vec<CostType> {
    DIMENSIONS
        .iter()
        .map(|&(_, label, color)| CostType { name: label, color })
        .collect()
}

// ---------------------------------------------------------------------------
// Conversion: HeatmapTreeNode -> SpanNode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct SpanHotspot {
    file: String,
    line: usize,
    function_name: String,
    dimension: String,
    responsibility: f64,
    detail: String,
}

impl From<&HeatmapEntry> for SpanHotspot {
    fn from(value: &HeatmapEntry) -> Self {
        Self {
            file: value.file.clone(),
            line: value.line,
            function_name: value.function_name.clone(),
            dimension: value.dimension.clone(),
            responsibility: value.responsibility,
            detail: value.detail.clone(),
        }
    }
}

/// Convert a rollup tree into a flamegraph span tree.
///
/// When there is exactly one root, it becomes the top-level `SpanNode`
/// directly. When there are multiple roots (or zero), a synthetic wrapper
/// node labeled `path_label` is created.
pub fn heatmap_to_flame_graph(
    roots: &[HeatmapTreeNode],
    path_label: &str,
) -> (SpanNode, Vec<CostType>) {
    let (root, cost_types, _) = heatmap_to_flame_graph_with_hotspots(roots, path_label);
    (root, cost_types)
}

fn heatmap_to_flame_graph_with_hotspots(
    roots: &[HeatmapTreeNode],
    path_label: &str,
) -> (SpanNode, Vec<CostType>, HashMap<SpanId, SpanHotspot>) {
    let mut builder = SpanNodeBuilder::new();
    let cost_types = dimension_cost_types();
    let mut hotspots_by_span = HashMap::new();

    let mut child_hotspots = Vec::new();
    let children: Vec<SpanNode> = roots
        .iter()
        .map(|r| {
            let (span, hotspot) = convert_node(r, &mut builder, &mut hotspots_by_span);
            child_hotspots.push(hotspot);
            span
        })
        .collect();

    let root = if children.len() == 1 {
        match children.into_iter().next() {
            Some(only) => only,
            None => {
                return (
                    builder.leaf(path_label, vec![0.0; DIMENSIONS.len()]),
                    cost_types,
                    hotspots_by_span,
                );
            }
        }
    } else {
        let total: Vec<f64> = (0..DIMENSIONS.len())
            .map(|i| children.iter().map(|c| c.costs.amounts[i]).sum())
            .collect();
        let root = builder.span(path_label, total, children);
        if let Some(hotspot) = child_hotspots.into_iter().flatten().reduce(pick_hotter) {
            hotspots_by_span.insert(root.id, hotspot);
        }
        root
    };

    (root, cost_types, hotspots_by_span)
}

fn convert_node(
    node: &HeatmapTreeNode,
    builder: &mut SpanNodeBuilder,
    hotspots_by_span: &mut HashMap<SpanId, SpanHotspot>,
) -> (SpanNode, Option<SpanHotspot>) {
    let label = segment_display_name(&node.segment);
    let amounts: Vec<f64> = DIMENSIONS
        .iter()
        .map(|&(key, _, _)| {
            node.dimension_responsibilities
                .get(key)
                .copied()
                .unwrap_or(0.0)
        })
        .collect();

    let mut child_hotspots = Vec::new();
    let children: Vec<SpanNode> = node
        .children
        .iter()
        .map(|c| {
            let (child, hotspot) = convert_node(c, builder, hotspots_by_span);
            child_hotspots.push(hotspot);
            child
        })
        .collect();

    let span = builder.span(&label, amounts, children);

    let local_hotspot = node
        .entries
        .iter()
        .map(SpanHotspot::from)
        .reduce(pick_hotter);
    let subtree_hotspot = child_hotspots.into_iter().flatten().reduce(pick_hotter);
    let hottest = match (local_hotspot, subtree_hotspot) {
        (Some(local), Some(child)) => Some(pick_hotter(local, child)),
        (Some(local), None) => Some(local),
        (None, Some(child)) => Some(child),
        (None, None) => None,
    };

    if let Some(hotspot) = hottest.clone() {
        hotspots_by_span.insert(span.id, hotspot);
    }

    (span, hottest)
}

fn pick_hotter(left: SpanHotspot, right: SpanHotspot) -> SpanHotspot {
    match compare_hotspots(&left, &right) {
        Ordering::Greater => left,
        Ordering::Less => right,
        Ordering::Equal => left,
    }
}

fn compare_hotspots(left: &SpanHotspot, right: &SpanHotspot) -> Ordering {
    left.responsibility
        .total_cmp(&right.responsibility)
        .then_with(|| right.file.cmp(&left.file))
        .then_with(|| right.line.cmp(&left.line))
        .then_with(|| right.dimension.cmp(&left.dimension))
        .then_with(|| right.function_name.cmp(&left.function_name))
}

fn segment_display_name(seg: &ScopeSegment) -> String {
    match seg {
        ScopeSegment::Module(n) | ScopeSegment::Type(n) | ScopeSegment::Function(n) => n.clone(),
    }
}

// ---------------------------------------------------------------------------
// Explorer app state
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct DetailsPaneState {
    expanded: bool,
    scroll_offset: usize,
    last_viewport_height: usize,
    pending_focus_line: Option<usize>,
}

impl DetailsPaneState {
    fn focus_line(&mut self, line: usize) {
        self.scroll_offset = line.saturating_sub(DETAILS_FOCUS_CONTEXT);
        self.pending_focus_line = Some(line);
    }

    fn page_height(&self) -> usize {
        self.last_viewport_height.saturating_sub(1).max(1)
    }

    fn half_page_height(&self) -> usize {
        (self.last_viewport_height / 2).max(1)
    }

    fn scroll_by(&mut self, delta: isize) {
        if delta >= 0 {
            self.scroll_offset = self.scroll_offset.saturating_add(delta as usize);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub(delta.unsigned_abs());
        }
    }
}

#[derive(Clone)]
enum SourceCacheEntry {
    Loading,
    Loaded(CachedSource),
    Missing(String),
}

#[derive(Clone)]
struct CachedSource {
    lines: Vec<String>,
    highlighted: Option<Vec<Vec<(String, Style)>>>,
}

struct DetailsDocument {
    title: String,
    header_lines: Vec<Line<'static>>,
    focus_line: usize,
    lines: Vec<Line<'static>>,
}

/// Cached document for a specific span, rebuilt only when the selected span changes.
struct CachedDetailsDocument {
    span_id: SpanId,
    document: DetailsDocument,
}

struct ExploreApp {
    flame_graph: FlameGraph,
    source_root: PathBuf,
    hotspots_by_span: HashMap<SpanId, SpanHotspot>,
    details: DetailsPaneState,
    source_cache: HashMap<String, SourceCacheEntry>,
    source_rx: mpsc::Receiver<(String, SourceCacheEntry)>,
    source_tx: mpsc::Sender<(String, SourceCacheEntry)>,
    cached_document: Option<CachedDetailsDocument>,
}

impl ExploreApp {
    fn new(
        flame_graph: FlameGraph,
        source_root: PathBuf,
        hotspots_by_span: HashMap<SpanId, SpanHotspot>,
    ) -> Self {
        let (source_tx, source_rx) = mpsc::channel();
        Self {
            flame_graph,
            source_root,
            hotspots_by_span,
            details: DetailsPaneState::default(),
            source_cache: HashMap::new(),
            source_tx,
            source_rx,
            cached_document: None,
        }
    }

    fn tick(&mut self, dt: Duration) {
        self.flame_graph.tick(dt);
    }

    fn view(&self) -> ExploreView {
        ExploreView {
            details_expanded: self.details.expanded,
        }
    }

    #[cfg(test)]
    fn handle_key(&mut self, key: &KeyEvent) -> KeyResult {
        let view = self.view();
        match modal::resolve::<ExploreModal>(&view, key) {
            Some(ExploreIntent::Quit) => KeyResult::Consumed,
            Some(intent) => self.apply_intent(intent),
            None => KeyResult::Ignored,
        }
    }

    fn apply_intent(&mut self, intent: ExploreIntent) -> KeyResult {
        match intent {
            ExploreIntent::Quit => KeyResult::Consumed,
            ExploreIntent::ToggleDetails => {
                self.details.expanded = !self.details.expanded;
                if self.details.expanded {
                    self.details.focus_line(self.focus_line_for_selected_span());
                }
                KeyResult::Consumed
            }
            ExploreIntent::ScrollPageDown => {
                self.details.scroll_by(self.details.page_height() as isize);
                KeyResult::Consumed
            }
            ExploreIntent::ScrollPageUp => {
                self.details
                    .scroll_by(-(self.details.page_height() as isize));
                KeyResult::Consumed
            }
            ExploreIntent::ScrollTop => {
                self.details.scroll_offset = 0;
                self.details.pending_focus_line = None;
                KeyResult::Consumed
            }
            ExploreIntent::ScrollBottom => {
                self.details.scroll_offset = usize::MAX;
                self.details.pending_focus_line = None;
                KeyResult::Consumed
            }
            ExploreIntent::ScrollHalfPageDown => {
                self.details
                    .scroll_by(self.details.half_page_height() as isize);
                self.details.pending_focus_line = None;
                KeyResult::Consumed
            }
            ExploreIntent::ScrollHalfPageUp => {
                self.details
                    .scroll_by(-(self.details.half_page_height() as isize));
                self.details.pending_focus_line = None;
                KeyResult::Consumed
            }
            _ => self.apply_flame_intent(intent),
        }
    }

    fn apply_flame_intent(&mut self, intent: ExploreIntent) -> KeyResult {
        let canonical_key = match intent {
            ExploreIntent::MoveUp => KeyCode::Char('k'),
            ExploreIntent::MoveDown => KeyCode::Char('j'),
            ExploreIntent::ExpandOrDescend => KeyCode::Char('l'),
            ExploreIntent::CollapseOrAscend => KeyCode::Char('h'),
            ExploreIntent::Focus => KeyCode::Char('f'),
            ExploreIntent::Unfocus => KeyCode::Char('F'),
            ExploreIntent::Undo => KeyCode::Char('u'),
            ExploreIntent::Redo => KeyCode::Char('r'),
            ExploreIntent::ToggleLegend => KeyCode::Enter,
            _ => return KeyResult::Ignored,
        };
        let key = KeyEvent::new(canonical_key, KeyModifiers::NONE);
        let previous = self.selected_span_id();
        let result = self.flame_graph.handle_key(&key);
        if self.details.expanded && previous != self.selected_span_id() {
            self.details.focus_line(self.focus_line_for_selected_span());
        }
        result
    }

    fn selected_span_id(&self) -> SpanId {
        self.flame_graph
            .path()
            .last()
            .copied()
            .unwrap_or(self.flame_graph.root().id)
    }

    fn selected_hotspot(&self) -> Option<&SpanHotspot> {
        self.hotspots_by_span.get(&self.selected_span_id())
    }

    fn focus_line_for_selected_span(&self) -> usize {
        self.selected_hotspot()
            .map_or(0, |hotspot| hotspot.line.saturating_sub(1))
    }

    fn details_document(&mut self) -> &DetailsDocument {
        let span_id = self.selected_span_id();
        let needs_rebuild = self
            .cached_document
            .as_ref()
            .is_none_or(|c| c.span_id != span_id);

        if needs_rebuild {
            let document = self.build_details_document(span_id);
            self.cached_document = Some(CachedDetailsDocument { span_id, document });
        }

        // SAFETY: we just assigned `Some(...)` above when `needs_rebuild` is true,
        // and on re-entry the cached value is still present.
        #[allow(clippy::expect_used)]
        &self.cached_document.as_ref().expect("just built").document
    }

    fn build_details_document(&mut self, span_id: SpanId) -> DetailsDocument {
        let Some(hotspot) = self.hotspots_by_span.get(&span_id).cloned() else {
            return details_document_no_hotspot();
        };

        if hotspot.file == "<codebase>" || hotspot.line == 0 {
            return details_document_codebase_level(&hotspot);
        }

        self.request_source(&hotspot.file);
        #[allow(clippy::expect_used)]
        let cache_entry = self
            .source_cache
            .get(&hotspot.file)
            .expect("just requested");
        details_document_from_source(&hotspot, cache_entry)
    }

    /// Request source for a file. If not yet cached, spawns a background thread
    /// to load and highlight it.
    fn request_source(&mut self, file: &str) {
        if self.source_cache.contains_key(file) {
            return;
        }
        self.source_cache
            .insert(file.to_string(), SourceCacheEntry::Loading);
        let root = self.source_root.clone();
        let file_key = file.to_string();
        let tx = self.source_tx.clone();
        std::thread::spawn(move || {
            let entry = load_source_cache_entry(&root, &file_key);
            let _ = tx.send((file_key, entry));
        });
    }

    /// Returns `true` if any source files are still loading in the background.
    fn has_pending_source_loads(&self) -> bool {
        self.source_cache
            .values()
            .any(|e| matches!(e, SourceCacheEntry::Loading))
    }

    /// Drain completed background source loads into the cache.
    /// Returns `true` if any new sources arrived (caller should re-render).
    fn drain_source_loads(&mut self) -> bool {
        let mut any = false;
        while let Ok((file, entry)) = self.source_rx.try_recv() {
            self.source_cache.insert(file, entry);
            // Invalidate cached document so it rebuilds with the new source.
            self.cached_document = None;
            any = true;
        }
        any
    }
}

fn details_document_no_hotspot() -> DetailsDocument {
    DetailsDocument {
        title: "details".to_string(),
        focus_line: 0,
        header_lines: vec![Line::from(Span::styled(
            "No hotspot data is available for the selected span.",
            Style::default().fg(mb_tui::theme::dim()),
        ))],
        lines: Vec::new(),
    }
}

fn details_document_codebase_level(hotspot: &SpanHotspot) -> DetailsDocument {
    DetailsDocument {
        title: format!("details {}", hotspot.dimension),
        focus_line: 0,
        header_lines: vec![
            hotspot_summary_line(hotspot),
            hotspot_location_line(hotspot),
            Line::default(),
            Line::from(Span::styled(
                "This hotspot is aggregated at the codebase level, so there is no single source snippet to load.",
                Style::default().fg(mb_tui::theme::dim()),
            )),
        ],
        lines: Vec::new(),
    }
}

fn details_document_from_source(
    hotspot: &SpanHotspot,
    cache_entry: &SourceCacheEntry,
) -> DetailsDocument {
    let title = format!("details {}:{}", hotspot.file, hotspot.line);
    match cache_entry {
        SourceCacheEntry::Loading => DetailsDocument {
            title,
            focus_line: 0,
            header_lines: vec![
                hotspot_summary_line(hotspot),
                hotspot_location_line(hotspot),
                Line::default(),
                Line::from(Span::styled(
                    "Loading source...",
                    Style::default().fg(mb_tui::theme::dim()),
                )),
            ],
            lines: Vec::new(),
        },
        SourceCacheEntry::Loaded(source) => {
            let lines: Vec<Line<'static>> = source
                .lines
                .iter()
                .enumerate()
                .map(|(idx, raw_line)| {
                    let line_number = idx + 1;
                    let highlighted = source.highlighted.as_ref().and_then(|all| all.get(idx));
                    code_line(
                        line_number,
                        raw_line,
                        highlighted,
                        line_number == hotspot.line,
                    )
                })
                .collect();

            DetailsDocument {
                title,
                header_lines: vec![
                    hotspot_summary_line(hotspot),
                    hotspot_location_line(hotspot),
                ],
                focus_line: hotspot.line.saturating_sub(1),
                lines,
            }
        }
        SourceCacheEntry::Missing(error) => DetailsDocument {
            title,
            focus_line: 0,
            header_lines: vec![
                hotspot_summary_line(hotspot),
                hotspot_location_line(hotspot),
                Line::default(),
                Line::from(Span::styled(
                    error.clone(),
                    Style::default().fg(mb_tui::theme::error()),
                )),
            ],
            lines: Vec::new(),
        },
    }
}

fn hotspot_summary_line(hotspot: &SpanHotspot) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("hotspot {} ", hotspot.function_name),
            Style::default()
                .fg(mb_tui::theme::text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("[{}] ", hotspot.dimension),
            Style::default().fg(mb_tui::theme::warning()),
        ),
        Span::styled(
            format!("loss +{:.4}", hotspot.responsibility),
            Style::default().fg(mb_tui::theme::focus()),
        ),
    ])
}

fn hotspot_location_line(hotspot: &SpanHotspot) -> Line<'static> {
    let detail = if hotspot.detail.is_empty() {
        "no extra detail".to_string()
    } else {
        hotspot.detail.clone()
    };

    Line::from(vec![
        Span::styled(
            format!("{}:{} ", hotspot.file, hotspot.line),
            Style::default().fg(mb_tui::theme::dim()),
        ),
        Span::styled(detail, Style::default().fg(mb_tui::theme::text())),
    ])
}

fn code_line(
    line_number: usize,
    raw_line: &str,
    highlighted: Option<&Vec<(String, Style)>>,
    is_hotspot_line: bool,
) -> Line<'static> {
    let prefix_style = if is_hotspot_line {
        Style::default()
            .fg(mb_tui::theme::warning())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(mb_tui::theme::dim())
    };
    let code_emphasis = if is_hotspot_line {
        Some(Modifier::BOLD)
    } else {
        None
    };

    let mut spans = vec![Span::styled(
        format!(
            "{}{line_number:>4} ",
            if is_hotspot_line { ">" } else { " " }
        ),
        prefix_style,
    )];

    if let Some(highlighted) = highlighted {
        spans.extend(highlighted.iter().map(|(text, style)| {
            let style = match code_emphasis {
                Some(modifier) => style.add_modifier(modifier),
                None => *style,
            };
            Span::styled(text.clone(), style)
        }));
    } else {
        let style = match code_emphasis {
            Some(modifier) => Style::default()
                .fg(mb_tui::theme::text())
                .add_modifier(modifier),
            None => Style::default().fg(mb_tui::theme::text()),
        };
        spans.push(Span::styled(raw_line.to_string(), style));
    }

    Line::from(spans)
}

fn load_source_cache_entry(root: &Path, file: &str) -> SourceCacheEntry {
    let resolved = resolve_source_path(root, file);
    match std::fs::read_to_string(&resolved) {
        Ok(source) => SourceCacheEntry::Loaded(CachedSource {
            lines: source.lines().map(str::to_string).collect(),
            highlighted: highlight_code(&source, "rust"),
        }),
        Err(error) => {
            SourceCacheEntry::Missing(format!("Failed to load {}: {error}", resolved.display()))
        }
    }
}

fn resolve_source_path(root: &Path, file: &str) -> PathBuf {
    let file_path = Path::new(file);
    if file_path.is_absolute() {
        return file_path.to_path_buf();
    }

    let base = if root.is_file() {
        root.parent().unwrap_or(root)
    } else {
        root
    };

    let mut fallback = None;
    for ancestor in base.ancestors() {
        let candidate = ancestor.join(file_path);
        if fallback.is_none() {
            fallback = Some(candidate.clone());
        }
        if candidate.exists() {
            return candidate;
        }
    }

    fallback.unwrap_or_else(|| base.join(file_path))
}

// ---------------------------------------------------------------------------
// TUI entry point
// ---------------------------------------------------------------------------

/// Run analysis on `path` and launch the interactive flamegraph explorer.
pub fn run_explore(
    path: &Path,
    policy_path: Option<&Path>,
    semantic: Option<&crate::SemanticOverlay>,
) -> anyhow::Result<()> {
    let mut report = crate::analyze_path(path)?;
    if let Some(overlay) = semantic {
        report.semantic = Some(crate::SemanticSummary::from_overlay(overlay));
    }

    let policy = load_policy(policy_path)?;
    let cr = crate::compute_compliance_with_semantic(&report, &policy, semantic);

    if cr.heatmap.is_empty() {
        println!("No loss hotspots -- all dimensions at 0.0 loss. Nothing to explore.");
        return Ok(());
    }

    let roots = crate::build_heatmap_tree(&cr.heatmap);
    let path_label = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("codebase");
    let (root, cost_types, hotspots_by_span) =
        heatmap_to_flame_graph_with_hotspots(&roots, path_label);
    let app = ExploreApp::new(
        FlameGraph::new(root, cost_types),
        path.to_path_buf(),
        hotspots_by_span,
    );

    run_tui(app)
}

/// Launch the explore TUI with pre-built heatmap tree roots.
pub fn run_explore_with_tree(roots: Vec<crate::rollup::HeatmapTreeNode>) -> anyhow::Result<()> {
    let (root, cost_types, hotspots_by_span) =
        heatmap_to_flame_graph_with_hotspots(&roots, "codebase");
    let app = ExploreApp::new(
        FlameGraph::new(root, cost_types),
        PathBuf::from("."),
        hotspots_by_span,
    );
    run_tui(app)
}

fn load_policy(path: Option<&Path>) -> anyhow::Result<crate::CompliancePolicy> {
    match path {
        Some(p) => Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?),
        None => Ok(crate::CompliancePolicy::default()),
    }
}

// ---------------------------------------------------------------------------
// Terminal setup + event loop
// ---------------------------------------------------------------------------

fn run_tui(mut app: ExploreApp) -> anyhow::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app);

    terminal::disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ---------------------------------------------------------------------------
// ModalInput wiring
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExploreIntent {
    Quit,
    ToggleDetails,
    // Details pane scrolling
    ScrollPageDown,
    ScrollPageUp,
    ScrollTop,
    ScrollBottom,
    ScrollHalfPageDown,
    ScrollHalfPageUp,
    // Flame graph navigation (forwarded to FlameGraph::handle_key)
    MoveUp,
    MoveDown,
    ExpandOrDescend,
    CollapseOrAscend,
    Focus,
    Unfocus,
    Undo,
    Redo,
    ToggleLegend,
}

/// Lightweight view snapshot for guard evaluation.
struct ExploreView {
    details_expanded: bool,
}

struct ExploreModal;

impl ModalInput for ExploreModal {
    type Intent = ExploreIntent;
    type App = ExploreView;
    type Context = ();

    fn active_context(_app: &ExploreView) {}

    fn bindings(_ctx: &()) -> Vec<KeyBinding<ExploreIntent, ExploreView>> {
        explore_bindings()
    }
}

fn no_modifier(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

fn explore_bindings() -> Vec<KeyBinding<ExploreIntent, ExploreView>> {
    let mut bindings = flame_graph_bindings();
    bindings.extend(details_scroll_bindings());
    bindings
}

#[allow(clippy::too_many_lines)]
fn flame_graph_bindings() -> Vec<KeyBinding<ExploreIntent, ExploreView>> {
    vec![
        KeyBinding {
            key_label: "q",
            action: "quit",
            description: "Quit the explorer",
            resolve: |key| {
                (no_modifier(key) && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc))
                    .then_some(ExploreIntent::Quit)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "j/k",
            action: "move",
            description: "Move cursor up/down",
            resolve: |key| {
                if !no_modifier(key) {
                    return None;
                }
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => Some(ExploreIntent::MoveUp),
                    KeyCode::Down | KeyCode::Char('j') => Some(ExploreIntent::MoveDown),
                    _ => None,
                }
            },
            guard: None,
        },
        KeyBinding {
            key_label: "h/l",
            action: "collapse/expand",
            description: "Collapse or expand tree nodes",
            resolve: |key| {
                if !no_modifier(key) {
                    return None;
                }
                match key.code {
                    KeyCode::Left | KeyCode::Char('h') => Some(ExploreIntent::CollapseOrAscend),
                    KeyCode::Right | KeyCode::Char('l') => Some(ExploreIntent::ExpandOrDescend),
                    _ => None,
                }
            },
            guard: None,
        },
        KeyBinding {
            key_label: "f/F",
            action: "focus",
            description: "Focus or unfocus on selected node",
            resolve: |key| {
                if !no_modifier(key) {
                    return None;
                }
                match key.code {
                    KeyCode::Char('f') => Some(ExploreIntent::Focus),
                    KeyCode::Char('F') => Some(ExploreIntent::Unfocus),
                    _ => None,
                }
            },
            guard: None,
        },
        KeyBinding {
            key_label: "enter",
            action: "legend",
            description: "Toggle cost legend for selected node",
            resolve: |key| {
                (no_modifier(key) && key.code == KeyCode::Enter)
                    .then_some(ExploreIntent::ToggleLegend)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "d",
            action: "details",
            description: "Toggle details pane",
            resolve: |key| {
                (key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Char('d'))
                    .then_some(ExploreIntent::ToggleDetails)
            },
            guard: None,
        },
        KeyBinding {
            key_label: "u/r",
            action: "undo/redo",
            description: "Undo or redo navigation",
            resolve: |key| {
                if !no_modifier(key) {
                    return None;
                }
                match key.code {
                    KeyCode::Char('u') => Some(ExploreIntent::Undo),
                    KeyCode::Char('r') => Some(ExploreIntent::Redo),
                    _ => None,
                }
            },
            guard: None,
        },
    ]
}

fn details_scroll_bindings() -> Vec<KeyBinding<ExploreIntent, ExploreView>> {
    vec![
        KeyBinding {
            key_label: "PgDn",
            action: "page down",
            description: "Scroll details pane down",
            resolve: |key| {
                (no_modifier(key) && matches!(key.code, KeyCode::PageDown | KeyCode::Char(' ')))
                    .then_some(ExploreIntent::ScrollPageDown)
            },
            guard: Some(|v| v.details_expanded),
        },
        KeyBinding {
            key_label: "PgUp",
            action: "page up",
            description: "Scroll details pane up",
            resolve: |key| {
                (no_modifier(key) && key.code == KeyCode::PageUp)
                    .then_some(ExploreIntent::ScrollPageUp)
            },
            guard: Some(|v| v.details_expanded),
        },
        KeyBinding {
            key_label: "g/G",
            action: "top/bottom",
            description: "Scroll to top or bottom of details",
            resolve: |key| {
                if !no_modifier(key) {
                    return None;
                }
                match key.code {
                    KeyCode::Home | KeyCode::Char('g') => Some(ExploreIntent::ScrollTop),
                    KeyCode::End | KeyCode::Char('G') => Some(ExploreIntent::ScrollBottom),
                    _ => None,
                }
            },
            guard: Some(|v| v.details_expanded),
        },
        KeyBinding {
            key_label: "C-d/u",
            action: "half-page",
            description: "Scroll details half page down/up",
            resolve: |key| {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    return None;
                }
                match key.code {
                    KeyCode::Char('d') => Some(ExploreIntent::ScrollHalfPageDown),
                    KeyCode::Char('u') => Some(ExploreIntent::ScrollHalfPageUp),
                    _ => None,
                }
            },
            guard: Some(|v| v.details_expanded),
        },
    ]
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExploreAction {
    Continue,
    Quit,
}

fn step_key(app: &mut ExploreApp, key: &KeyEvent) -> RenderStep<ExploreAction> {
    let view = app.view();
    match modal::resolve::<ExploreModal>(&view, key) {
        Some(ExploreIntent::Quit) => RenderStep::new(ExploreAction::Quit),
        Some(intent) => match app.apply_intent(intent) {
            KeyResult::Consumed => RenderStep::schedule_render(ExploreAction::Continue),
            KeyResult::Ignored => RenderStep::new(ExploreAction::Continue),
        },
        None => RenderStep::new(ExploreAction::Continue),
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ExploreApp,
) -> anyhow::Result<()> {
    let mut last_tick = Instant::now();
    let mut scheduler = RenderScheduler::new(last_tick);
    scheduler.schedule_render_now(last_tick);

    loop {
        // Drain completed background source loads.
        if app.drain_source_loads() {
            scheduler.schedule_render_now(Instant::now());
        }

        let now = Instant::now();
        let animate = app.flame_graph.needs_idle_render();
        if scheduler.should_render(now, animate) {
            if animate {
                app.tick(now - last_tick);
            }
            last_tick = now;

            draw(terminal, app)?;
            scheduler.record_render(now);
        }

        let has_pending_loads = app.has_pending_source_loads();
        let timeout = scheduler
            .time_until_next_render(Instant::now(), app.flame_graph.needs_idle_render())
            .unwrap_or(if has_pending_loads {
                Duration::from_millis(16)
            } else {
                Duration::from_secs(60)
            });

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let was_animating = app.flame_graph.needs_idle_render();
            let step = step_key(app, &key);
            let now = Instant::now();
            if !was_animating && app.flame_graph.needs_idle_render() {
                last_tick = now;
            }
            apply_render_effect(&mut scheduler, now, step.effect);

            if step.action == ExploreAction::Quit {
                break;
            }
        }
    }

    Ok(())
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ExploreApp,
) -> anyhow::Result<()> {
    terminal.draw(|frame| {
        render_screen(app, frame.area(), frame.buffer_mut());
    })?;
    Ok(())
}

fn render_screen(app: &mut ExploreApp, area: Rect, buf: &mut Buffer) {
    let outer = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    let title = Paragraph::new(Line::from(Span::styled(
        " code-loss explore",
        Style::default().fg(mb_tui::theme::warning()),
    )));
    Widget::render(title, outer[0], buf);

    render_body(app, outer[1], buf);

    let view = app.view();
    let hints = modal::hints::<ExploreModal>(&view);
    HotkeyBarRenderable { hints }.render(outer[2], buf);
}

fn render_body(app: &mut ExploreApp, area: Rect, buf: &mut Buffer) {
    match compute_details_layout(area, app.details.expanded) {
        DetailsLayout::Hidden => render_flame_graph_mut(&mut app.flame_graph, area, buf),
        DetailsLayout::Stacked { graph, details } | DetailsLayout::Split { graph, details } => {
            render_flame_graph_mut(&mut app.flame_graph, graph, buf);
            render_details_pane(app, details, buf);
        }
    }
}

fn compute_details_layout(area: Rect, details_expanded: bool) -> DetailsLayout {
    if !details_expanded {
        return DetailsLayout::Hidden;
    }

    if let Some(split) = compute_split_details_layout(area) {
        return split;
    }

    compute_stacked_details_layout(area).unwrap_or(DetailsLayout::Hidden)
}

fn compute_split_details_layout(area: Rect) -> Option<DetailsLayout> {
    if area.width < MIN_SPLIT_GRAPH_WIDTH.saturating_add(MIN_SPLIT_DETAILS_WIDTH) {
        return None;
    }

    let graph_width = ((area.width as u32 * 44) / 100) as u16;
    let graph_width = graph_width.clamp(
        MIN_SPLIT_GRAPH_WIDTH,
        area.width.saturating_sub(MIN_SPLIT_DETAILS_WIDTH),
    );

    let split = Layout::horizontal([
        Constraint::Length(graph_width),
        Constraint::Min(MIN_SPLIT_DETAILS_WIDTH),
    ])
    .split(area);

    Some(DetailsLayout::Split {
        graph: split[0],
        details: split[1],
    })
}

fn compute_stacked_details_layout(area: Rect) -> Option<DetailsLayout> {
    if area.height <= MIN_FLAME_GRAPH_HEIGHT + 3 {
        return None;
    }

    let max_details = area.height.saturating_sub(MIN_FLAME_GRAPH_HEIGHT);
    let details_height = (area.height / 3)
        .clamp(MIN_STACKED_DETAILS_HEIGHT, 12)
        .min(max_details);

    let split = Layout::vertical([
        Constraint::Length(area.height.saturating_sub(details_height)),
        Constraint::Length(details_height),
    ])
    .split(area);

    Some(DetailsLayout::Stacked {
        graph: split[0],
        details: split[1],
    })
}

fn render_details_pane(app: &mut ExploreApp, area: Rect, buf: &mut Buffer) {
    // Build/reuse the cached document.
    app.details_document();

    #[allow(clippy::expect_used)]
    let doc = &app.cached_document.as_ref().expect("just built").document;
    let Some(inner) = render_pane_frame(area, buf, &doc.title, true) else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let header_height = (doc.header_lines.len() as u16).min(inner.height);
    if header_height > 0 {
        let header_area = Rect::new(inner.x, inner.y, inner.width, header_height);
        Widget::render(Paragraph::new(doc.header_lines.clone()), header_area, buf);
    }

    let pager_area = Rect::new(
        inner.x,
        inner.y.saturating_add(header_height),
        inner.width,
        inner.height.saturating_sub(header_height),
    );

    if pager_area.height == 0 || doc.lines.is_empty() {
        app.details.last_viewport_height = pager_area.height as usize;
        return;
    }

    let focus_line = doc.focus_line;
    let mut pager = LayoutPagerView::new(
        doc.lines
            .iter()
            .map(|line| LayoutRenderableItem::Borrowed(line))
            .collect(),
        app.details.scroll_offset,
    );
    if let Some(line) = app.details.pending_focus_line.take() {
        pager.scroll_chunk_into_view(line.min(focus_line));
    }
    pager.render(pager_area, buf);
    app.details.scroll_offset = pager.scroll_offset;
    app.details.last_viewport_height = pager_area.height as usize;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::compliance::HeatmapEntry;
    use crate::metrics::ScopeSegment;
    use crate::rollup::build_heatmap_tree;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use ratatui::buffer::Buffer;
    use tempfile::tempdir;
    use mb_tui::devkit::buffer_to_text;

    fn entry(
        function_name: &str,
        dimension: &str,
        responsibility: f64,
        scope_path: Vec<ScopeSegment>,
    ) -> HeatmapEntry {
        HeatmapEntry {
            file: "a.rs".into(),
            line: 1,
            function_name: function_name.into(),
            dimension: dimension.into(),
            responsibility,
            detail: String::new(),
            scope_path,
        }
    }

    #[test]
    fn conversion_preserves_total_responsibility() {
        let entries = vec![
            entry(
                "foo",
                "bloat",
                0.3,
                vec![
                    ScopeSegment::Module("m".into()),
                    ScopeSegment::Function("foo".into()),
                ],
            ),
            entry(
                "bar",
                "state_cardinality",
                0.2,
                vec![
                    ScopeSegment::Module("m".into()),
                    ScopeSegment::Function("bar".into()),
                ],
            ),
            entry(
                "baz",
                "coupling_density",
                0.1,
                vec![
                    ScopeSegment::Module("n".into()),
                    ScopeSegment::Function("baz".into()),
                ],
            ),
        ];
        let original_total: f64 = entries.iter().map(|e| e.responsibility).sum();

        let roots = build_heatmap_tree(&entries);
        let (root, _) = heatmap_to_flame_graph(&roots, "test");

        let span_total: f64 = root.costs.amounts.iter().sum();
        assert!(
            (span_total - original_total).abs() < 1e-10,
            "span total {span_total} != original {original_total}"
        );
    }

    #[test]
    fn single_root_not_wrapped() {
        let entries = vec![entry(
            "foo",
            "bloat",
            0.5,
            vec![
                ScopeSegment::Module("only".into()),
                ScopeSegment::Function("foo".into()),
            ],
        )];
        let roots = build_heatmap_tree(&entries);
        assert_eq!(roots.len(), 1);

        let (root, _) = heatmap_to_flame_graph(&roots, "wrapper");
        // The root label should be the single tree node's name, not "wrapper".
        assert_eq!(root.label, "only");
    }

    #[test]
    fn multiple_roots_get_synthetic_wrapper() {
        let entries = vec![
            entry(
                "a_fn",
                "bloat",
                0.3,
                vec![ScopeSegment::Module("alpha".into())],
            ),
            entry(
                "b_fn",
                "bloat",
                0.2,
                vec![ScopeSegment::Module("beta".into())],
            ),
        ];
        let roots = build_heatmap_tree(&entries);
        assert_eq!(roots.len(), 2);

        let (root, _) = heatmap_to_flame_graph(&roots, "my_crate");
        assert_eq!(root.label, "my_crate");
        assert_eq!(root.children.len(), 2);
    }

    #[test]
    fn absent_dimensions_are_zero() {
        // Entry has only "bloat" dimension.
        let entries = vec![entry(
            "foo",
            "bloat",
            0.4,
            vec![ScopeSegment::Function("foo".into())],
        )];
        let roots = build_heatmap_tree(&entries);
        let (root, _) = heatmap_to_flame_graph(&roots, "test");

        // bloat is at index 0, all others should be 0.0.
        assert!((root.costs.amounts[0] - 0.4).abs() < 1e-10);
        for (i, &val) in root.costs.amounts.iter().enumerate() {
            if i != 0 {
                assert!(
                    val.abs() < 1e-10,
                    "dimension index {i} ({}) should be 0.0, got {val}",
                    DIMENSIONS[i].0
                );
            }
        }
    }

    #[test]
    fn empty_input_produces_zero_leaf() {
        let roots = build_heatmap_tree(&[]);
        let (root, _) = heatmap_to_flame_graph(&roots, "empty");

        assert_eq!(root.label, "empty");
        assert!(root.children.is_empty());
        assert_eq!(root.costs.amounts.len(), DIMENSIONS.len());
        let total: f64 = root.costs.amounts.iter().sum();
        assert!(total.abs() < 1e-10);
    }

    #[test]
    fn subtree_hotspot_tracks_hottest_descendant() {
        let entries = vec![
            HeatmapEntry {
                file: "src/lib.rs".into(),
                line: 4,
                function_name: "alpha".into(),
                dimension: "bloat".into(),
                responsibility: 0.7,
                detail: "20 lines".into(),
                scope_path: vec![
                    ScopeSegment::Module("app".into()),
                    ScopeSegment::Function("alpha".into()),
                ],
            },
            HeatmapEntry {
                file: "src/lib.rs".into(),
                line: 20,
                function_name: "beta".into(),
                dimension: "duplication".into(),
                responsibility: 0.2,
                detail: "clone group".into(),
                scope_path: vec![
                    ScopeSegment::Module("app".into()),
                    ScopeSegment::Function("beta".into()),
                ],
            },
        ];

        let roots = build_heatmap_tree(&entries);
        let (root, _, hotspots) = heatmap_to_flame_graph_with_hotspots(&roots, "demo");

        let hotspot = hotspots
            .get(&root.id)
            .expect("root span should have a subtree hotspot");
        assert_eq!(hotspot.function_name, "alpha");
        assert_eq!(hotspot.line, 4);
    }

    // -- Palette expectation tests ------------------------------------------

    /// WCAG 2.0 relative luminance (piecewise sRGB linearization).
    fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
        let to_linear = |c: u8| {
            let v = f64::from(c) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * to_linear(r) + 0.7152 * to_linear(g) + 0.0722 * to_linear(b)
    }

    #[test]
    fn palette_colors_meet_wcag_aa_large_contrast() {
        let cost_types = dimension_cost_types();
        for ct in &cost_types {
            let Color::Rgb(r, g, b) = ct.color else {
                panic!("{} uses a non-RGB color -- cannot verify contrast", ct.name);
            };
            let luminance = relative_luminance(r, g, b);
            let ratio = (luminance + 0.05) / 0.05; // against black
            assert!(
                ratio >= 3.0,
                "{} has contrast ratio {ratio:.2} < 3.0 against black",
                ct.name
            );
        }
    }

    #[test]
    fn palette_colors_are_perceptually_distinct() {
        // Hues were constructed at 72° intervals: 30, 102, 174, 246, 318.
        let hues = [30.0_f64, 102.0, 174.0, 246.0, 318.0];
        for i in 0..hues.len() {
            for j in (i + 1)..hues.len() {
                let diff = (hues[i] - hues[j]).abs();
                let angular_distance = diff.min(360.0 - diff);
                assert!(
                    angular_distance >= 60.0,
                    "hues {} and {} are only {angular_distance}° apart",
                    hues[i],
                    hues[j]
                );
            }
        }
    }

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn finish_animations(app: &mut ExploreApp) {
        for _ in 0..60 {
            app.tick(Duration::from_millis(16));
        }
    }

    /// Block until all pending background source loads complete.
    fn flush_source_loads(app: &mut ExploreApp) {
        while app.has_pending_source_loads() {
            std::thread::sleep(Duration::from_millis(1));
            app.drain_source_loads();
        }
    }

    fn render_app(app: &mut ExploreApp, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        // Render once to trigger any source load requests, then flush and
        // re-render so the loaded source content is visible.
        let mut buf = Buffer::empty(area);
        render_screen(app, area, &mut buf);
        if app.has_pending_source_loads() {
            flush_source_loads(app);
            buf = Buffer::empty(area);
            render_screen(app, area, &mut buf);
        }
        buffer_to_text(&buf)
    }

    fn write_fixture_file(root: &Path) -> PathBuf {
        let src_dir = root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        let file = src_dir.join("lib.rs");
        let mut lines = Vec::new();
        for line in 1..=28 {
            let content = match line {
                4 => "fn alpha() {",
                5 => "    let mut total = 0;",
                6 => "    total += expensive_call();",
                7 => "}",
                18 => "fn beta() {",
                19 => "    if duplicate_logic() {",
                20 => "        duplicated_branch();",
                21 => "    }",
                22 => "}",
                _ => "// filler",
            };
            lines.push(content.to_string());
        }
        std::fs::write(&file, lines.join("\n")).expect("write fixture");
        file
    }

    fn make_details_app(root: &Path) -> ExploreApp {
        let entries = vec![
            HeatmapEntry {
                file: "src/lib.rs".into(),
                line: 4,
                function_name: "alpha".into(),
                dimension: "bloat".into(),
                responsibility: 0.7,
                detail: "20 lines".into(),
                scope_path: vec![
                    ScopeSegment::Module("app".into()),
                    ScopeSegment::Function("alpha".into()),
                ],
            },
            HeatmapEntry {
                file: "src/lib.rs".into(),
                line: 20,
                function_name: "beta".into(),
                dimension: "duplication".into(),
                responsibility: 0.5,
                detail: "clone group".into(),
                scope_path: vec![
                    ScopeSegment::Module("app".into()),
                    ScopeSegment::Function("beta".into()),
                ],
            },
        ];

        let roots = build_heatmap_tree(&entries);
        let (root_span, cost_types, hotspots) =
            heatmap_to_flame_graph_with_hotspots(&roots, "fixture");
        ExploreApp::new(
            FlameGraph::new(root_span, cost_types),
            root.to_path_buf(),
            hotspots,
        )
    }

    fn make_codebase_app(root: &Path) -> ExploreApp {
        let mut builder = SpanNodeBuilder::new();
        let root_span = builder.leaf("codebase", vec![1.0]);
        let root_id = root_span.id;
        ExploreApp::new(
            FlameGraph::new(root_span, dimension_cost_types()),
            root.to_path_buf(),
            HashMap::from([(
                root_id,
                SpanHotspot {
                    file: "<codebase>".into(),
                    line: 0,
                    function_name: "coupling_density".into(),
                    dimension: "coupling_density".into(),
                    responsibility: 0.3,
                    detail: "cross-module edges".into(),
                },
            )]),
        )
    }

    fn render_after_inputs(
        mut app: ExploreApp,
        inputs: &[KeyEvent],
        width: u16,
        height: u16,
    ) -> String {
        for key in inputs {
            app.handle_key(key);
        }
        render_app(&mut app, width, height)
    }

    #[test]
    fn details_pane_loads_selected_hotspot_snippet() {
        let temp = tempdir().expect("tempdir");
        write_fixture_file(temp.path());
        let mut app = make_details_app(temp.path());

        app.handle_key(&make_key(KeyCode::Char('d')));
        let text = render_app(&mut app, 120, 24);

        assert!(text.contains("hotspot alpha"));
        assert!(text.contains("src/lib.rs:4"));
        assert!(
            text.contains(">   4 fn alpha() {"),
            "rendered text:\n{text}"
        );
    }

    #[test]
    fn details_pane_follows_span_selection() {
        let temp = tempdir().expect("tempdir");
        write_fixture_file(temp.path());
        let mut app = make_details_app(temp.path());

        app.handle_key(&make_key(KeyCode::Char('d')));
        app.handle_key(&make_key(KeyCode::Right));
        finish_animations(&mut app);
        app.handle_key(&make_key(KeyCode::Down));
        finish_animations(&mut app);

        let text = render_app(&mut app, 120, 24);
        assert!(text.contains("hotspot beta"), "rendered text:\n{text}");
        assert!(text.contains("src/lib.rs:20"));
        assert!(
            text.contains(">  20         duplicated_branch();"),
            "rendered text:\n{text}"
        );
    }

    #[test]
    fn details_pane_scrolls_like_a_pager() {
        let temp = tempdir().expect("tempdir");
        write_fixture_file(temp.path());
        let mut app = make_details_app(temp.path());

        app.handle_key(&make_key(KeyCode::Char('d')));
        let initial = render_app(&mut app, 120, 24);
        assert!(initial.contains(">   4 fn alpha() {"));

        app.handle_key(&make_key(KeyCode::PageDown));
        app.handle_key(&make_key(KeyCode::PageDown));
        let scrolled = render_app(&mut app, 120, 24);
        assert!(
            scrolled.contains("  12 // filler"),
            "rendered text:\n{scrolled}"
        );
        assert!(
            !scrolled.contains(">   4 fn alpha() {"),
            "hotspot line should have scrolled out of view:\n{scrolled}"
        );
    }

    #[test]
    fn codebase_hotspot_shows_fallback_message() {
        let app_root = tempdir().expect("tempdir");
        let mut app = make_codebase_app(app_root.path());

        app.handle_key(&make_key(KeyCode::Char('d')));
        let text = render_app(&mut app, 120, 24);
        assert!(text.contains("aggregated at the codebase level"));
    }

    #[test]
    fn resolve_source_path_uses_workspace_ancestor_for_workspace_relative_hotspot() {
        let temp = tempdir().expect("tempdir");
        let crate_root = temp.path().join("crates/code-loss");
        let file_path = crate_root.join("src/compliance.rs");
        std::fs::create_dir_all(file_path.parent().expect("parent")).expect("create dirs");
        std::fs::write(&file_path, "// test").expect("write file");

        let resolved = resolve_source_path(&crate_root, "crates/code-loss/src/compliance.rs");
        assert_eq!(resolved, file_path);
    }

    #[test]
    fn wide_layout_places_details_to_the_right() {
        let temp = tempdir().expect("tempdir");
        write_fixture_file(temp.path());
        let text = render_after_inputs(
            make_details_app(temp.path()),
            &[make_key(KeyCode::Char('d'))],
            120,
            24,
        );
        let title_row = text
            .lines()
            .find(|line| line.contains("┌details src/lib.rs:4"))
            .expect("details border row should be present");
        assert!(
            title_row.contains("app"),
            "wide layout should render graph row and details border on the same line:\n{text}"
        );
    }

    #[test]
    fn narrow_layout_falls_back_to_stacked_details() {
        let temp = tempdir().expect("tempdir");
        write_fixture_file(temp.path());
        let text = render_after_inputs(
            make_details_app(temp.path()),
            &[make_key(KeyCode::Char('d'))],
            72,
            24,
        );
        let title_row = text
            .lines()
            .find(|line| line.contains("┌details src/lib.rs:4"))
            .expect("details border row should be present");
        assert!(
            !title_row.contains("app"),
            "narrow layout should stack details below the graph instead of squeezing both into one row:\n{text}"
        );
    }

    #[test]
    fn details_visual_snapshots_after_inputs() {
        let temp = tempdir().expect("tempdir");
        write_fixture_file(temp.path());

        let cases = [
            (
                "details_root_after_120x24",
                render_after_inputs(
                    make_details_app(temp.path()),
                    &[make_key(KeyCode::Char('d'))],
                    120,
                    24,
                ),
            ),
            (
                "details_second_child_after_120x24",
                render_after_inputs(
                    make_details_app(temp.path()),
                    &[
                        make_key(KeyCode::Char('d')),
                        make_key(KeyCode::Right),
                        make_key(KeyCode::Down),
                    ],
                    120,
                    24,
                ),
            ),
            (
                "details_scrolled_after_120x24",
                render_after_inputs(
                    make_details_app(temp.path()),
                    &[
                        make_key(KeyCode::Char('d')),
                        make_key(KeyCode::PageDown),
                        make_key(KeyCode::PageDown),
                    ],
                    120,
                    24,
                ),
            ),
            (
                "details_codebase_after_120x24",
                render_after_inputs(
                    make_codebase_app(temp.path()),
                    &[make_key(KeyCode::Char('d'))],
                    120,
                    24,
                ),
            ),
            (
                "details_root_narrow_after_72x24",
                render_after_inputs(
                    make_details_app(temp.path()),
                    &[make_key(KeyCode::Char('d'))],
                    72,
                    24,
                ),
            ),
        ];

        for (name, text) in cases {
            insta::assert_snapshot!(name, text);
        }
    }

    #[test]
    fn step_key_quit_does_not_schedule_render() {
        let temp = tempdir().expect("tempdir");
        let mut app = make_codebase_app(temp.path());

        let step = step_key(&mut app, &make_key(KeyCode::Char('q')));

        assert_eq!(step.action, ExploreAction::Quit);
        assert_eq!(step.effect, None);
    }

    #[test]
    fn step_key_consumed_navigation_schedules_render() {
        let temp = tempdir().expect("tempdir");
        let mut app = make_codebase_app(temp.path());

        let step = step_key(&mut app, &make_key(KeyCode::Enter));

        assert_eq!(step.action, ExploreAction::Continue);
        assert!(matches!(
            step.effect,
            Some(mb_tui::input::RenderEffect::ScheduleRender)
        ));
    }
}

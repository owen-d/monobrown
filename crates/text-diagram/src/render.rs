//! Text renderers for normalized diagram specs.
//!
//! Rendering is pure and deterministic: no terminal probing, no I/O, and no
//! ambient width detection. Callers provide a width, then snapshot the string.

use std::collections::BTreeSet;

use crate::error::DiagramError;
use crate::spec::{
    CellAlign, CellDepth, CellFill, CellSpec, DiagramBlock, DiagramNodeId, DiagramSpec, EdgeSpec,
    GraphSpec, RowFrame, RowSpec, RowsSpec,
};

/// Presentation policy for cells with visual depth.
///
/// Depth is stored on cells because it describes the diagram subject, but this
/// enum belongs to render configuration because the same depth can be drawn as
/// compact inline marks or as a taller projected shape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DepthStyle {
    /// Draw depth on the right edge only, preserving the row's normal height.
    #[default]
    InlineRight,
    /// Draw depth down and right, spending extra row height for a 3D cue.
    Projected3d,
}

/// Corner glyph family for Unicode boxes and depth projections.
///
/// This is separate from `DepthStyle` because corner shape is visual tone, not
/// layout geometry. ASCII rendering intentionally collapses both styles to the
/// same portable `+`/`-`/`|` vocabulary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CornerStyle {
    /// Use square box-drawing corners for unambiguous joins and projections.
    #[default]
    Sharp,
    /// Use rounded Unicode corners when a softer visual style is preferred.
    Rounded,
}

/// Positive fixed render width in columns.
///
/// Fixed width makes generated diagrams stable in comments and tests. This is
/// a simple character budget, not terminal display-width measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderWidth(u16);

impl RenderWidth {
    /// Default width for chat, comment, and doc snippets.
    pub const DEFAULT_COLUMNS: u16 = 80;

    /// Creates a positive render width.
    ///
    /// Zero is rejected because truncation and padding rules need a real
    /// boundary to stay deterministic.
    pub fn new(columns: u16) -> Result<Self, DiagramError> {
        if columns == 0 {
            return Err(DiagramError::InvalidRenderWidth { columns });
        }
        Ok(Self(columns))
    }

    /// Returns the configured column budget.
    ///
    /// Renderers use this as a deterministic text budget, independent from any
    /// terminal or editor state.
    pub fn columns(self) -> u16 {
        self.0
    }
}

impl Default for RenderWidth {
    fn default() -> Self {
        Self(Self::DEFAULT_COLUMNS)
    }
}

/// Shared renderer configuration.
///
/// The config exists so ASCII and Unicode renderers can evolve together without
/// coupling their glyph choices or layout details.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderConfig {
    width: RenderWidth,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
}

impl RenderConfig {
    /// Creates config with an explicit width.
    ///
    /// Width is the first shared policy because deterministic output is the
    /// core ergonomic requirement for docs and snapshot tests.
    pub fn new(width: RenderWidth) -> Self {
        Self {
            width,
            depth_style: DepthStyle::default(),
            corner_style: CornerStyle::default(),
        }
    }

    /// Returns a copy with an explicit depth presentation style.
    ///
    /// Keeping this in renderer config lets callers change the visual language
    /// without changing the diagram's structural cells or replica labels.
    pub fn with_depth_style(mut self, depth_style: DepthStyle) -> Self {
        self.depth_style = depth_style;
        self
    }

    /// Returns a copy with an explicit Unicode corner style.
    ///
    /// Keeping this separate from depth style avoids coupling "how many layers
    /// are visible" to "which corner glyph family should be used".
    pub fn with_corner_style(mut self, corner_style: CornerStyle) -> Self {
        self.corner_style = corner_style;
        self
    }

    /// Returns the configured fixed width.
    ///
    /// Renderers should not observe terminal state as a fallback.
    pub fn width(self) -> RenderWidth {
        self.width
    }

    /// Returns the configured depth presentation style.
    ///
    /// Renderers use this to decide whether depth costs horizontal space only
    /// or both horizontal and vertical space.
    pub fn depth_style(self) -> DepthStyle {
        self.depth_style
    }

    /// Returns the configured corner glyph family.
    ///
    /// Unicode renderers use this for boxes and projections; ASCII renderers
    /// keep their portable fallback glyphs.
    pub fn corner_style(self) -> CornerStyle {
        self.corner_style
    }
}

/// Renders a normalized diagram spec into text.
///
/// The trait is intentionally output-only: renderers should not validate graph
/// structure or mutate specs. Construction owns invariants; rendering owns
/// presentation.
pub trait DiagramRenderer {
    /// Produces deterministic text for the supplied spec.
    ///
    /// Implementations return a string rather than writing to I/O so callers can
    /// use the same API in comments, tests, logs, and docs.
    fn render(&self, spec: &DiagramSpec) -> String;
}

/// Plain ASCII renderer for maximum portability.
///
/// Use this when output may land in tooling that cannot preserve box-drawing
/// characters. It should remain conservative and dependency-free.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AsciiRenderer {
    config: RenderConfig,
}

impl AsciiRenderer {
    /// Creates an ASCII renderer from shared config.
    ///
    /// The renderer owns glyph policy; the config owns only shared constraints.
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    /// Creates an ASCII renderer with an explicit width.
    ///
    /// This keeps the common fixed-width case terse at call sites.
    pub fn with_width(width: RenderWidth) -> Self {
        Self::new(RenderConfig::new(width))
    }

    /// Creates an ASCII renderer with explicit width and depth style.
    ///
    /// This is the ergonomic path for tests that want to pin projected depth
    /// output without constructing a separate config value.
    pub fn with_depth_style(width: RenderWidth, depth_style: DepthStyle) -> Self {
        Self::new(RenderConfig::new(width).with_depth_style(depth_style))
    }
}

impl DiagramRenderer for AsciiRenderer {
    fn render(&self, spec: &DiagramSpec) -> String {
        render_blocks(
            spec,
            self.config.width(),
            GlyphSet::Ascii,
            self.config.depth_style(),
            self.config.corner_style(),
        )
    }
}

/// Unicode renderer using box-drawing and arrow glyphs.
///
/// Use this when the text surface preserves Unicode. It is richer than the
/// ASCII renderer, but still renders from the same normalized spec.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UnicodeRenderer {
    config: RenderConfig,
}

impl UnicodeRenderer {
    /// Creates a Unicode renderer from shared config.
    ///
    /// Keeping config separate lets Unicode presentation change without
    /// changing graph adaptation or spec construction.
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    /// Creates a Unicode renderer with an explicit width.
    ///
    /// This supports snapshot tests that need narrower fixtures than the
    /// default comment width.
    pub fn with_width(width: RenderWidth) -> Self {
        Self::new(RenderConfig::new(width))
    }

    /// Creates a Unicode renderer with explicit width and depth style.
    ///
    /// Projected depth is most useful with this renderer because box-drawing
    /// glyphs can express the down/right projection cleanly.
    pub fn with_depth_style(width: RenderWidth, depth_style: DepthStyle) -> Self {
        Self::new(RenderConfig::new(width).with_depth_style(depth_style))
    }

    /// Creates a Unicode renderer with explicit width and corner style.
    ///
    /// This keeps visual tone configurable without changing diagram structure
    /// or depth geometry.
    pub fn with_corner_style(width: RenderWidth, corner_style: CornerStyle) -> Self {
        Self::new(RenderConfig::new(width).with_corner_style(corner_style))
    }
}

impl DiagramRenderer for UnicodeRenderer {
    fn render(&self, spec: &DiagramSpec) -> String {
        render_blocks(
            spec,
            self.config.width(),
            GlyphSet::Unicode,
            self.config.depth_style(),
            self.config.corner_style(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlyphSet {
    Ascii,
    Unicode,
}

// One dispatcher keeps block ordering policy independent from glyph policy.
fn render_blocks(
    spec: &DiagramSpec,
    width: RenderWidth,
    glyphs: GlyphSet,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
) -> String {
    let mut lines = Vec::new();

    for block in spec.blocks() {
        if !lines.is_empty() {
            lines.push(String::new());
        }

        match block {
            DiagramBlock::Graph(graph) => {
                render_graph(graph, width, glyphs, corner_style, &mut lines);
            }
            DiagramBlock::Rows(rows) => {
                render_rows(rows, width, glyphs, depth_style, corner_style, &mut lines);
            }
        }
    }

    lines.join("\n")
}

// Graph rendering stays intentionally modest until a real layout policy exists.
fn render_graph(
    graph: &GraphSpec,
    width: RenderWidth,
    glyphs: GlyphSet,
    corner_style: CornerStyle,
    lines: &mut Vec<String>,
) {
    match glyphs {
        GlyphSet::Ascii => render_graph_ascii(graph, width, lines),
        GlyphSet::Unicode => render_graph_unicode(graph, width, corner_style, lines),
    }
}

// Row/span diagrams use a shared logical grid across all rows.
fn render_rows(
    rows: &RowsSpec,
    width: RenderWidth,
    glyphs: GlyphSet,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
    lines: &mut Vec<String>,
) {
    let span_total_max = rows.span_total_max();
    if span_total_max == 0 {
        return;
    }

    let shadow_width_max = rows_shadow_width_max(rows);
    let unit_width = row_unit_width(width, span_total_max, shadow_width_max);
    for row in rows.rows() {
        render_row(
            row,
            unit_width,
            width,
            glyphs,
            depth_style,
            corner_style,
            lines,
        );
    }
}

// Unit width is derived from the fixed render width, never from terminal state.
fn row_unit_width(width: RenderWidth, span_total_max: u32, shadow_width_max: u32) -> u16 {
    let columns = u32::from(width.columns());
    let available = columns.saturating_sub(1 + shadow_width_max);
    let unit = available / span_total_max;
    let content = unit.saturating_sub(1).max(1);
    u16::try_from(content).unwrap_or(u16::MAX)
}

// Stacked cells add right-side shadow glyphs that must be budgeted up front.
fn rows_shadow_width_max(rows: &RowsSpec) -> u32 {
    let mut shadow_width_max = 0u32;
    for row in rows.rows() {
        shadow_width_max = shadow_width_max.max(row_shadow_width(row));
    }
    shadow_width_max
}

// Row shadow width is the sum of per-cell visual overlap marks.
fn row_shadow_width(row: &RowSpec) -> u32 {
    let mut shadow_width = 0u32;
    for cell in row.cells() {
        shadow_width += u32::from(cell.depth().layers().saturating_sub(1));
    }
    shadow_width
}

// Each row owns whether it is framed; cells own labels, spans, and fill.
fn render_row(
    row: &RowSpec,
    unit_width: u16,
    width: RenderWidth,
    glyphs: GlyphSet,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
    lines: &mut Vec<String>,
) {
    if row.cells().is_empty() {
        return;
    }

    match row.frame() {
        RowFrame::Boxed => {
            lines.push(fit_line(
                &render_row_rule(
                    row,
                    unit_width,
                    glyphs,
                    RowRule::Top,
                    depth_style,
                    corner_style,
                ),
                width,
                truncation_marker(glyphs),
            ));
            lines.push(fit_line(
                &render_row_content(row, unit_width, glyphs, true),
                width,
                truncation_marker(glyphs),
            ));
            lines.push(fit_line(
                &render_row_rule(
                    row,
                    unit_width,
                    glyphs,
                    RowRule::Bottom,
                    depth_style,
                    corner_style,
                ),
                width,
                truncation_marker(glyphs),
            ));
            render_row_projection(
                row,
                unit_width,
                width,
                glyphs,
                depth_style,
                corner_style,
                lines,
            );
        }
        RowFrame::Plain => {
            lines.push(fit_line(
                &render_row_content(row, unit_width, glyphs, false),
                width,
                truncation_marker(glyphs),
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowRule {
    Top,
    Bottom,
}

// Rules are derived from cell spans so multi-span cells hide internal joins.
fn render_row_rule(
    row: &RowSpec,
    unit_width: u16,
    glyphs: GlyphSet,
    rule: RowRule,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
) -> String {
    let row_glyphs = row_glyphs(glyphs, rule, corner_style);
    let mut line = String::new();
    line.push(row_glyphs.left);

    for (cell_index, cell) in row.cells().iter().enumerate() {
        let width = cell_width(cell, unit_width);
        line.push_str(&row_glyphs.horizontal.to_string().repeat(width));
        line.push(row_rule_boundary_glyph(
            cell,
            cell_index + 1 == row.cells().len(),
            row_glyphs,
            glyphs,
            rule,
            depth_style,
            corner_style,
        ));
        line.push_str(&cell_depth_shadow(
            cell.depth(),
            glyphs,
            rule,
            depth_style,
            corner_style,
        ));
    }

    line
}

// Projected depth turns a bottom table join into a front-face corner.
fn row_rule_boundary_glyph(
    cell: &CellSpec,
    is_last: bool,
    row_glyphs: RowGlyphs,
    glyphs: GlyphSet,
    rule: RowRule,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
) -> char {
    if depth_style == DepthStyle::Projected3d
        && rule == RowRule::Bottom
        && cell.depth().layers() > 1
    {
        return projection_front_bottom_glyph(glyphs, corner_style);
    }

    if is_last {
        return row_glyphs.right;
    }
    row_glyphs.join
}

// Content rows use either frame separators or plain spaces between cells.
fn render_row_content(row: &RowSpec, unit_width: u16, glyphs: GlyphSet, framed: bool) -> String {
    let row_glyphs = row_content_glyphs(glyphs, framed);
    let mut line = String::new();
    line.push(row_glyphs.left);

    for (cell_index, cell) in row.cells().iter().enumerate() {
        let width = cell_width(cell, unit_width);
        line.push_str(&cell_content(cell, width, glyphs));
        if cell_index + 1 == row.cells().len() {
            line.push(row_glyphs.right);
        } else {
            line.push(row_glyphs.join);
        }
        line.push_str(&cell_depth_content_shadow(cell.depth(), glyphs, framed));
    }

    line
}

// A spanning cell absorbs the separators inside the logical units it covers.
fn cell_width(cell: &CellSpec, unit_width: u16) -> usize {
    let span = usize::from(cell.span().units());
    let unit_width = usize::from(unit_width);
    span * unit_width + span.saturating_sub(1)
}

// Depth shadows are intentionally tiny: they suggest overlap without layout.
fn cell_depth_shadow(
    depth: CellDepth,
    glyphs: GlyphSet,
    rule: RowRule,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
) -> String {
    let count = usize::from(depth.layers().saturating_sub(1));
    let glyph = match (glyphs, rule) {
        (GlyphSet::Ascii, RowRule::Top) => '+',
        (GlyphSet::Ascii, RowRule::Bottom) => '+',
        (GlyphSet::Unicode, RowRule::Top) => unicode_top_right_corner(corner_style),
        (GlyphSet::Unicode, RowRule::Bottom) => match depth_style {
            DepthStyle::InlineRight => unicode_bottom_right_corner(corner_style),
            DepthStyle::Projected3d => '│',
        },
    };
    glyph.to_string().repeat(count)
}

// Projected depth spends vertical space only when callers opt into the mode.
fn render_row_projection(
    row: &RowSpec,
    unit_width: u16,
    width: RenderWidth,
    glyphs: GlyphSet,
    depth_style: DepthStyle,
    corner_style: CornerStyle,
    lines: &mut Vec<String>,
) {
    if depth_style != DepthStyle::Projected3d {
        return;
    }

    let projection_count = row_projection_count(row);
    for projection_index in 1..=projection_count {
        let line =
            render_row_projection_line(row, unit_width, glyphs, corner_style, projection_index);
        lines.push(fit_line(&line, width, truncation_marker(glyphs)));
    }
}

// Projection height is driven by the deepest cell in the row.
fn row_projection_count(row: &RowSpec) -> u8 {
    let mut projection_count = 0u8;
    for cell in row.cells() {
        projection_count = projection_count.max(cell.depth().layers().saturating_sub(1));
    }
    projection_count
}

// Projection lines are overlaid into a blank row so neighboring cells compose.
fn render_row_projection_line(
    row: &RowSpec,
    unit_width: u16,
    glyphs: GlyphSet,
    corner_style: CornerStyle,
    projection_index: u8,
) -> String {
    let line_width = row_render_width(row, unit_width);
    let mut line = vec![' '; line_width];
    let mut cursor = 1usize;

    for cell in row.cells() {
        let width = cell_width(cell, unit_width);
        let depth_count = cell.depth().layers().saturating_sub(1);
        if projection_index <= depth_count {
            let border_start = cursor.saturating_sub(1);
            overlay_projection_cell(
                &mut line,
                border_start,
                width,
                depth_count,
                projection_index,
                glyphs,
                corner_style,
            );
        }
        cursor += width + 1 + usize::from(depth_count);
    }

    trim_right_spaces(line)
}

// Row width matches the normal rendered row before final clipping.
fn row_render_width(row: &RowSpec, unit_width: u16) -> usize {
    let mut width = 1usize;
    for cell in row.cells() {
        width += cell_width(cell, unit_width);
        width += 1;
        width += usize::from(cell.depth().layers().saturating_sub(1));
    }
    width
}

// Each projected layer shifts one column right and closes one shadow layer.
fn overlay_projection_cell(
    line: &mut [char],
    border_start: usize,
    cell_width: usize,
    depth_count: u8,
    projection_index: u8,
    glyphs: GlyphSet,
    corner_style: CornerStyle,
) {
    let start = border_start + usize::from(projection_index);
    let right = start + cell_width + 1;
    set_char(line, start, projection_left_glyph(glyphs, corner_style));
    for index in (start + 1)..right {
        set_char(line, index, projection_horizontal_glyph(glyphs));
    }
    set_char(line, right, projection_right_glyph(glyphs, corner_style));

    let remaining = depth_count.saturating_sub(projection_index);
    for offset in 1..=remaining {
        set_char(
            line,
            right + usize::from(offset),
            content_side_glyph(glyphs),
        );
    }
}

// Bounds checks keep projection overlays deterministic under narrow widths.
fn set_char(line: &mut [char], index: usize, value: char) {
    if index < line.len() {
        line[index] = value;
    }
}

// Projection lines should not carry useless trailing blanks into snapshots.
fn trim_right_spaces(line: Vec<char>) -> String {
    let mut end = line.len();
    while end > 0 && line[end - 1] == ' ' {
        end -= 1;
    }
    line.into_iter().take(end).collect()
}

// Unicode keeps projection corners aligned with the box-drawing frame style.
fn projection_left_glyph(glyphs: GlyphSet, corner_style: CornerStyle) -> char {
    match glyphs {
        GlyphSet::Ascii => '\\',
        GlyphSet::Unicode => unicode_bottom_left_corner(corner_style),
    }
}

// Projection rules share the row's horizontal glyph family.
fn projection_horizontal_glyph(glyphs: GlyphSet) -> char {
    match glyphs {
        GlyphSet::Ascii => '-',
        GlyphSet::Unicode => '─',
    }
}

// Unicode keeps projection corners aligned with the box-drawing frame style.
fn projection_right_glyph(glyphs: GlyphSet, corner_style: CornerStyle) -> char {
    match glyphs {
        GlyphSet::Ascii => '/',
        GlyphSet::Unicode => unicode_bottom_right_corner(corner_style),
    }
}

// The front face should close before the projected side continues downward.
fn projection_front_bottom_glyph(glyphs: GlyphSet, corner_style: CornerStyle) -> char {
    match glyphs {
        GlyphSet::Ascii => '/',
        GlyphSet::Unicode => unicode_bottom_right_corner(corner_style),
    }
}

// Content shadows keep stacked rows visually connected between top and bottom.
fn cell_depth_content_shadow(depth: CellDepth, glyphs: GlyphSet, framed: bool) -> String {
    if !framed {
        return String::new();
    }
    let count = usize::from(depth.layers().saturating_sub(1));
    let glyph = content_side_glyph(glyphs);
    glyph.to_string().repeat(count)
}

// Fill-only cells render ranges; labeled cells render text over optional fill.
fn cell_content(cell: &CellSpec, width: usize, glyphs: GlyphSet) -> String {
    match cell.fill() {
        Some(fill) => filled_cell_content(cell.label(), width, fill, cell.align(), glyphs),
        None => aligned_text(cell.label(), width, cell.align(), truncation_marker(glyphs)),
    }
}

// Filled labels are useful for interval annotations such as "(-inf,k)".
fn filled_cell_content(
    label: &str,
    width: usize,
    fill: CellFill,
    align: CellAlign,
    glyphs: GlyphSet,
) -> String {
    let fill_char = fill_glyph(fill, glyphs);
    if label.is_empty() {
        return fill_char.to_string().repeat(width);
    }

    let label = truncate_text(label, width, truncation_marker(glyphs));
    let label_width = label.chars().count();
    let fill_width = width.saturating_sub(label_width);
    let (left_width, right_width) = aligned_padding(fill_width, align);
    let mut content = fill_char.to_string().repeat(left_width);
    content.push_str(&label);
    content.push_str(&fill_char.to_string().repeat(right_width));
    content
}

// Alignment pads after truncation so each cell keeps its exact width.
fn aligned_text(text: &str, width: usize, align: CellAlign, marker: &str) -> String {
    let text = truncate_text(text, width, marker);
    let text_width = text.chars().count();
    let space_width = width.saturating_sub(text_width);
    let (left_width, right_width) = aligned_padding(space_width, align);
    let mut content = " ".repeat(left_width);
    content.push_str(&text);
    content.push_str(&" ".repeat(right_width));
    content
}

// Padding is the only place where alignment branches.
fn aligned_padding(space_width: usize, align: CellAlign) -> (usize, usize) {
    match align {
        CellAlign::Left => (0, space_width),
        CellAlign::Center => {
            let left_width = space_width / 2;
            (left_width, space_width - left_width)
        }
        CellAlign::Right => (space_width, 0),
    }
}

// Cell truncation differs from full-line truncation only by width type.
fn truncate_text(text: &str, width: usize, marker: &str) -> String {
    let text_width = text.chars().count();
    if text_width <= width {
        return text.to_owned();
    }

    let marker_width = marker.chars().count();
    if width <= marker_width {
        return marker.chars().take(width).collect();
    }

    let keep_width = width - marker_width;
    let mut truncated = text.chars().take(keep_width).collect::<String>();
    truncated.push_str(marker);
    truncated
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RowGlyphs {
    left: char,
    join: char,
    right: char,
    horizontal: char,
}

// Top and bottom rules differ only in corner/join glyphs.
fn row_glyphs(glyphs: GlyphSet, rule: RowRule, corner_style: CornerStyle) -> RowGlyphs {
    match (glyphs, rule) {
        (GlyphSet::Ascii, RowRule::Top) => ascii_row_rule_glyphs(),
        (GlyphSet::Ascii, RowRule::Bottom) => ascii_row_rule_glyphs(),
        (GlyphSet::Unicode, RowRule::Top) => RowGlyphs {
            left: unicode_top_left_corner(corner_style),
            join: '┬',
            right: unicode_top_right_corner(corner_style),
            horizontal: '─',
        },
        (GlyphSet::Unicode, RowRule::Bottom) => RowGlyphs {
            left: unicode_bottom_left_corner(corner_style),
            join: '┴',
            right: unicode_bottom_right_corner(corner_style),
            horizontal: '─',
        },
    }
}

// Corner style is Unicode-only; ASCII has no distinct rounded box vocabulary.
fn unicode_top_left_corner(corner_style: CornerStyle) -> char {
    match corner_style {
        CornerStyle::Sharp => '┌',
        CornerStyle::Rounded => '╭',
    }
}

// Corner style is Unicode-only; ASCII has no distinct rounded box vocabulary.
fn unicode_top_right_corner(corner_style: CornerStyle) -> char {
    match corner_style {
        CornerStyle::Sharp => '┐',
        CornerStyle::Rounded => '╮',
    }
}

// Corner style is Unicode-only; ASCII has no distinct rounded box vocabulary.
fn unicode_bottom_left_corner(corner_style: CornerStyle) -> char {
    match corner_style {
        CornerStyle::Sharp => '└',
        CornerStyle::Rounded => '╰',
    }
}

// Corner style is Unicode-only; ASCII has no distinct rounded box vocabulary.
fn unicode_bottom_right_corner(corner_style: CornerStyle) -> char {
    match corner_style {
        CornerStyle::Sharp => '┘',
        CornerStyle::Rounded => '╯',
    }
}

// ASCII boxes use one rule shape so fallback output stays plain.
fn ascii_row_rule_glyphs() -> RowGlyphs {
    RowGlyphs {
        left: '+',
        join: '+',
        right: '+',
        horizontal: '-',
    }
}

// Plain rows keep span spacing but omit visual box boundaries.
fn row_content_glyphs(glyphs: GlyphSet, framed: bool) -> RowGlyphs {
    if framed {
        return RowGlyphs {
            left: content_side_glyph(glyphs),
            join: content_side_glyph(glyphs),
            right: content_side_glyph(glyphs),
            horizontal: ' ',
        };
    }

    RowGlyphs {
        left: ' ',
        join: ' ',
        right: ' ',
        horizontal: ' ',
    }
}

// Unicode boxed rows use box-drawing sides; ASCII uses portable pipes.
fn content_side_glyph(glyphs: GlyphSet) -> char {
    match glyphs {
        GlyphSet::Ascii => '|',
        GlyphSet::Unicode => '│',
    }
}

// Fill glyphs are renderer-local policy over renderer-neutral fill semantics.
fn fill_glyph(fill: CellFill, glyphs: GlyphSet) -> char {
    match (fill, glyphs) {
        (CellFill::Solid, GlyphSet::Ascii) => '#',
        (CellFill::Medium, GlyphSet::Ascii) => '=',
        (CellFill::Light, GlyphSet::Ascii) => '-',
        (CellFill::Dash, GlyphSet::Ascii) => '-',
        (CellFill::Solid, GlyphSet::Unicode) => '█',
        (CellFill::Medium, GlyphSet::Unicode) => '▓',
        (CellFill::Light, GlyphSet::Unicode) => '▒',
        (CellFill::Dash, GlyphSet::Unicode) => '─',
    }
}

// The marker follows the renderer's glyph budget.
fn truncation_marker(glyphs: GlyphSet) -> &'static str {
    match glyphs {
        GlyphSet::Ascii => "...",
        GlyphSet::Unicode => "…",
    }
}

// ASCII output favors compact edge-list readability over visual layout.
fn render_graph_ascii(graph: &GraphSpec, width: RenderWidth, lines: &mut Vec<String>) {
    let connected = connected_nodes(graph);

    for edge in graph.edges() {
        let source = label_for(graph, edge.source());
        let target = label_for(graph, edge.target());
        let arrow = ascii_arrow(edge);
        let line = format!("[{source}] {arrow} [{target}]");
        lines.push(fit_line(&line, width, "..."));
    }

    for node in graph.nodes() {
        if connected.contains(node.id()) {
            continue;
        }
        let line = format!("[{}]", node.label());
        lines.push(fit_line(&line, width, "..."));
    }
}

// Unicode output favors a richer per-edge box shape without solving layout.
fn render_graph_unicode(
    graph: &GraphSpec,
    width: RenderWidth,
    corner_style: CornerStyle,
    lines: &mut Vec<String>,
) {
    let connected = connected_nodes(graph);

    for edge in graph.edges() {
        let source = unicode_box(label_for(graph, edge.source()), corner_style);
        let target = unicode_box(label_for(graph, edge.target()), corner_style);
        let arrow = unicode_arrow(edge);
        let spacer = " ".repeat(arrow.chars().count());

        lines.push(fit_line(
            &format!("{} {} {}", source.top, spacer, target.top),
            width,
            "…",
        ));
        lines.push(fit_line(
            &format!("{} {} {}", source.middle, arrow, target.middle),
            width,
            "…",
        ));
        lines.push(fit_line(
            &format!("{} {} {}", source.bottom, spacer, target.bottom),
            width,
            "…",
        ));
    }

    for node in graph.nodes() {
        if connected.contains(node.id()) {
            continue;
        }
        let node_box = unicode_box(node.label(), corner_style);
        lines.push(fit_line(&node_box.top, width, "…"));
        lines.push(fit_line(&node_box.middle, width, "…"));
        lines.push(fit_line(&node_box.bottom, width, "…"));
    }
}

// Lookup fallback is defensive; GraphSpec validation should make it unreachable.
fn label_for<'a>(graph: &'a GraphSpec, id: &'a DiagramNodeId) -> &'a str {
    match graph.node_label(id) {
        Some(label) => label,
        None => id.as_str(),
    }
}

// Connected-node tracking lets isolated nodes remain visible in graph output.
fn connected_nodes(graph: &GraphSpec) -> BTreeSet<DiagramNodeId> {
    let mut connected = BTreeSet::new();
    for edge in graph.edges() {
        connected.insert(edge.source().clone());
        connected.insert(edge.target().clone());
    }
    connected
}

// ASCII arrows are intentionally boring so fallback output stays portable.
fn ascii_arrow(edge: &EdgeSpec) -> String {
    match edge.label() {
        Some(label) => format!("--{label}-->"),
        None => "-->".to_owned(),
    }
}

// Unicode arrows carry the same semantic label with box-drawing glyphs.
fn unicode_arrow(edge: &EdgeSpec) -> String {
    match edge.label() {
        Some(label) => format!("─ {label} ─▶"),
        None => "──▶".to_owned(),
    }
}

struct UnicodeBox {
    top: String,
    middle: String,
    bottom: String,
}

// The simple box primitive is private until row/span layout exists.
fn unicode_box(label: &str, corner_style: CornerStyle) -> UnicodeBox {
    let inner_width = label.chars().count() + 2;
    let rule = "─".repeat(inner_width);
    let top = row_glyphs(GlyphSet::Unicode, RowRule::Top, corner_style);
    let bottom = row_glyphs(GlyphSet::Unicode, RowRule::Bottom, corner_style);
    UnicodeBox {
        top: format!("{}{rule}{}", top.left, top.right),
        middle: format!("│ {label} │"),
        bottom: format!("{}{rule}{}", bottom.left, bottom.right),
    }
}

// Truncation happens after formatting so renderers keep one width policy.
fn fit_line(line: &str, width: RenderWidth, marker: &str) -> String {
    let max = usize::from(width.columns());
    let line_len = line.chars().count();
    if line_len <= max {
        return line.to_owned();
    }

    let marker_len = marker.chars().count();
    if max <= marker_len {
        return marker.chars().take(max).collect();
    }

    let keep = max - marker_len;
    let mut fitted = line.chars().take(keep).collect::<String>();
    fitted.push_str(marker);
    fitted
}

#[cfg(test)]
mod tests {
    use crate::{CellFill, CellSpan, CellSpec, ConcreteGraph, DiagramSpec, RowSpec, RowsSpec};

    use super::*;

    #[test]
    fn ascii_renderer_outputs_stable_edge_list() -> Result<(), DiagramError> {
        let mut graph = ConcreteGraph::new();
        let input = graph.add_node("input")?;
        let parse = graph.add_node("parse")?;
        let render = graph.add_node("render")?;
        graph.add_labeled_edge(input, parse, "load")?;
        graph.add_edge(parse, render)?;

        let spec = DiagramSpec::from_graph(&graph)?;
        let text = AsciiRenderer::default().render(&spec);

        assert_eq!(text, "[input] --load--> [parse]\n[parse] --> [render]");
        Ok(())
    }

    #[test]
    fn unicode_renderer_outputs_box_drawing_edges() -> Result<(), DiagramError> {
        let mut graph = ConcreteGraph::new();
        let input = graph.add_node("input")?;
        let parse = graph.add_node("parse")?;
        graph.add_labeled_edge(input, parse, "load")?;

        let spec = DiagramSpec::from_graph(&graph)?;
        let text = UnicodeRenderer::default().render(&spec);

        assert_eq!(
            text,
            "┌───────┐           ┌───────┐\n\
             │ input │ ─ load ─▶ │ parse │\n\
             └───────┘           └───────┘"
        );
        Ok(())
    }

    #[test]
    fn unicode_renderer_can_use_rounded_graph_corners() -> Result<(), DiagramError> {
        let mut graph = ConcreteGraph::new();
        let input = graph.add_node("input")?;
        let parse = graph.add_node("parse")?;
        graph.add_labeled_edge(input, parse, "load")?;

        let spec = DiagramSpec::from_graph(&graph)?;
        let text = UnicodeRenderer::with_corner_style(RenderWidth::default(), CornerStyle::Rounded)
            .render(&spec);

        assert_eq!(
            text,
            "╭───────╮           ╭───────╮\n\
             │ input │ ─ load ─▶ │ parse │\n\
             ╰───────╯           ╰───────╯"
        );
        Ok(())
    }

    #[test]
    fn renderer_respects_fixed_width() -> Result<(), DiagramError> {
        let mut graph = ConcreteGraph::new();
        let left = graph.add_node("left")?;
        let right = graph.add_node("right")?;
        graph.add_labeled_edge(left, right, "very-long-label")?;

        let spec = DiagramSpec::from_graph(&graph)?;
        let width = RenderWidth::new(12)?;
        let text = AsciiRenderer::with_width(width).render(&spec);

        assert_eq!(text, "[left] --...");
        Ok(())
    }

    #[test]
    fn row_span_renderer_respects_fixed_width() -> Result<(), DiagramError> {
        let rows = RowsSpec::new(vec![RowSpec::new(vec![CellSpec::new(
            "a very long label",
            span(4)?,
        )])])?;
        let spec = DiagramSpec::from_rows(rows);
        let width = RenderWidth::new(10)?;
        let text = UnicodeRenderer::with_width(width).render(&spec);

        for line in text.lines() {
            assert!(line.chars().count() <= usize::from(width.columns()));
        }
        Ok(())
    }

    #[test]
    fn row_span_renderers_model_compaction_like_diagram() -> Result<(), DiagramError> {
        let rows = RowsSpec::new(vec![
            RowSpec::new(vec![CellSpec::new("input: SR(9)", span(6)?)]),
            RowSpec::new(vec![
                CellSpec::new("SST(91)", span(2)?),
                CellSpec::new("SST(92)", span(2)?),
                CellSpec::new("SST(93)", span(2)?),
            ]),
            RowSpec::new(vec![
                CellSpec::filled(span(2)?, CellFill::Solid),
                CellSpec::filled(span(2)?, CellFill::Medium),
                CellSpec::filled(span(2)?, CellFill::Light),
            ]),
            RowSpec::new(vec![
                CellSpec::new("subcompaction A", span(3)?),
                CellSpec::new("subcompaction B", span(3)?),
            ]),
            RowSpec::plain(vec![
                CellSpec::new("(-inf,k)", span(3)?).with_fill(CellFill::Dash),
                CellSpec::new("(k,+inf)", span(3)?).with_fill(CellFill::Dash),
            ]),
            RowSpec::new(vec![
                CellSpec::new("SST(93)", span(2)?),
                CellSpec::new("SST(94)", span(1)?),
                CellSpec::new("SST(95)", span(2)?),
                CellSpec::new("SST(96)", span(1)?),
            ]),
            RowSpec::new(vec![
                CellSpec::filled(span(2)?, CellFill::Solid),
                CellSpec::filled(span(1)?, CellFill::Medium),
                CellSpec::filled(span(2)?, CellFill::Medium),
                CellSpec::filled(span(1)?, CellFill::Light),
            ]),
            RowSpec::new(vec![CellSpec::new("output: SR(10)", span(6)?)]),
        ])?;
        let spec = DiagramSpec::from_rows(rows);
        let width = RenderWidth::new(61)?;

        let unicode = UnicodeRenderer::with_width(width).render(&spec);
        assert_eq!(unicode.lines().count(), 22);
        assert!(unicode.contains("subcompaction A"));
        assert!(unicode.contains("─(-inf,k)"));
        assert!(unicode.contains("│███████████████████│"));
        assert!(unicode.contains("│▓▓▓▓▓▓▓▓▓│"));
        assert!(unicode.contains("output: SR(10)"));

        let ascii = AsciiRenderer::with_width(width).render(&spec);
        assert_eq!(ascii.lines().count(), 22);
        assert!(ascii.contains("+-------------------+"));
        assert!(ascii.contains("|###################|"));
        assert!(ascii.contains("--(-inf,k)"));
        Ok(())
    }

    fn span(units: u16) -> Result<CellSpan, DiagramError> {
        CellSpan::new(units)
    }
}

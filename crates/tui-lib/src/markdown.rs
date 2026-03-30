//! Markdown-to-ratatui renderer for the chat view.
//!
//! Parses markdown text via `pulldown-cmark` and produces word-wrapped,
//! styled [`Line`]s suitable for rendering in a TUI pane.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::highlight;
use crate::theme;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse `text` as markdown and return styled, word-wrapped ratatui [`Line`]s.
///
/// `max_width` is the available character width for wrapping. `base_style` is
/// applied to all normal text (e.g. `theme::assistant()` for assistant messages).
pub fn render_markdown(text: &str, max_width: usize, base_style: Style) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![Line::default()];
    }

    let opts = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(text, opts);

    let mut renderer = MdRenderer {
        base_style,
        max_width,
        lines: Vec::new(),

        // Inline accumulation.
        spans: Vec::new(),
        style_stack: vec![base_style],

        // Block tracking.
        in_code_block: false,
        code_buf: String::new(),
        code_lang: None,
        list_stack: Vec::new(),
        blockquote_depth: 0,
        pending_item_marker: None,
    };

    renderer.walk(parser);

    // Guarantee at least one line so callers never get an empty vec.
    if renderer.lines.is_empty() {
        renderer.lines.push(Line::default());
    }

    // Trim a single trailing blank line that paragraph-end always emits.
    if renderer.lines.len() > 1
        && let Some(last) = renderer.lines.last()
        && (last.spans.is_empty() || last.spans.iter().all(|s| s.content.is_empty()))
    {
        renderer.lines.pop();
    }

    renderer.lines
}

// ---------------------------------------------------------------------------
// Renderer state machine
// ---------------------------------------------------------------------------

/// Tracks a list nesting level.
#[derive(Clone)]
struct ListInfo {
    /// `None` for unordered, `Some(next_number)` for ordered.
    ordered: Option<u64>,
}

struct MdRenderer {
    base_style: Style,
    max_width: usize,
    lines: Vec<Line<'static>>,

    // -- inline accumulation --------------------------------------------------
    /// Styled spans collected for the current paragraph / heading.
    spans: Vec<(String, Style)>,
    /// Stack of styles. Pushed on bold/italic/code open, popped on close.
    style_stack: Vec<Style>,

    // -- block state ----------------------------------------------------------
    in_code_block: bool,
    code_buf: String,
    code_lang: Option<String>,
    list_stack: Vec<ListInfo>,
    blockquote_depth: usize,
    /// The marker string to prepend to the *next* paragraph inside a list item.
    pending_item_marker: Option<String>,
}

impl MdRenderer {
    /// Current effective style (top of the style stack).
    fn current_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or(self.base_style)
    }

    /// Push a style modifier onto the stack, combining with the current top.
    fn push_modifier(&mut self, modifier: Modifier) {
        let top = self.current_style().add_modifier(modifier);
        self.style_stack.push(top);
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    /// Available width after accounting for blockquote prefixes.
    fn content_width(&self) -> usize {
        // Each blockquote depth costs "> " (2 chars).
        self.max_width
            .saturating_sub(self.blockquote_depth.saturating_mul(2))
    }

    /// Flush accumulated inline spans as wrapped lines, prepending blockquote
    /// markers and an optional list marker.
    fn flush_paragraph(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        let marker = self.pending_item_marker.take();

        let marker_width = marker.as_ref().map_or(0, String::len);
        let avail = self.content_width();

        // First line gets the marker; continuation lines get hang indent.
        let first_width = avail;
        let cont_width = avail.saturating_sub(marker_width);

        let wrapped = wrap_spans_with_hang(spans, first_width, cont_width);

        for (i, mut line) in wrapped.into_iter().enumerate() {
            // Prepend list marker or hang indent.
            if marker_width > 0 {
                if i == 0 {
                    if let Some(ref m) = marker {
                        line.insert(0, (m.clone(), self.base_style));
                    }
                } else {
                    line.insert(0, (" ".repeat(marker_width), self.base_style));
                }
            }
            self.emit_line(line);
        }
    }

    /// Emit a single line (vec of styled fragments), prepending blockquote
    /// markers if needed.
    fn emit_line(&mut self, fragments: Vec<(String, Style)>) {
        let mut out_spans: Vec<Span<'static>> = Vec::new();

        // Blockquote prefix.
        let bq_style = Style::default().fg(theme::success());
        for _ in 0..self.blockquote_depth {
            out_spans.push(Span::styled("> ".to_string(), bq_style));
        }

        for (text, style) in fragments {
            out_spans.push(Span::styled(text, style));
        }
        self.lines.push(Line::from(out_spans));
    }

    /// Emit a blank separator line (with blockquote markers if applicable).
    fn emit_blank(&mut self) {
        self.emit_line(vec![]);
    }

    /// Walk pulldown-cmark events and build output lines.
    fn walk<'a>(&mut self, parser: impl Iterator<Item = Event<'a>>) {
        for event in parser {
            match event {
                // -- block start ------------------------------------------
                Event::Start(tag) => self.start_tag(tag),

                // -- block end --------------------------------------------
                Event::End(tag) => self.end_tag(tag),

                // -- inline text ------------------------------------------
                Event::Text(text) => {
                    if self.in_code_block {
                        self.code_buf.push_str(&text);
                    } else {
                        let style = self.current_style();
                        self.spans.push((text.to_string(), style));
                    }
                }

                Event::Code(code) => {
                    let style = self.current_style().fg(theme::focus());
                    self.spans.push((code.to_string(), style));
                }

                Event::SoftBreak => {
                    if self.in_code_block {
                        self.code_buf.push('\n');
                    } else {
                        // Treat soft breaks as spaces in inline context.
                        let style = self.current_style();
                        self.spans.push((" ".to_string(), style));
                    }
                }

                Event::HardBreak => {
                    if self.in_code_block {
                        self.code_buf.push('\n');
                    } else {
                        self.flush_paragraph();
                    }
                }

                // Horizontal rule — render as a line of dashes.
                Event::Rule => {
                    let avail = self.content_width();
                    let ruler = "\u{2500}".repeat(avail.min(40));
                    self.emit_line(vec![(ruler, Style::default().fg(theme::dim()))]);
                }

                _ => {}
            }
        }

        // Flush any trailing inline content.
        self.flush_paragraph();
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { .. } => {
                self.push_modifier(Modifier::BOLD);
            }
            Tag::BlockQuote => {
                self.blockquote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.code_buf.clear();
                self.code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        // Extract the first token (before comma, space, or tab).
                        let token = lang
                            .split([',', ' ', '\t'])
                            .next()
                            .unwrap_or_default()
                            .trim();
                        if token.is_empty() {
                            None
                        } else {
                            Some(token.to_string())
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::List(start) => {
                self.list_stack.push(ListInfo { ordered: start });
            }
            Tag::Item => {
                // Compute marker for this item.
                let marker = if let Some(info) = self.list_stack.last_mut() {
                    if let Some(ref mut n) = info.ordered {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    } else {
                        "- ".to_string()
                    }
                } else {
                    "- ".to_string()
                };
                self.pending_item_marker = Some(marker);
            }
            Tag::Emphasis => self.push_modifier(Modifier::ITALIC),
            Tag::Strong => self.push_modifier(Modifier::BOLD),
            Tag::Strikethrough => self.push_modifier(Modifier::CROSSED_OUT),
            Tag::Link { .. } | Tag::Image { .. } => {}
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_paragraph();
                self.emit_blank();
            }
            TagEnd::Heading(_) => {
                self.flush_paragraph();
                self.emit_blank();
                self.pop_style();
            }
            TagEnd::BlockQuote => {
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                let buf = std::mem::take(&mut self.code_buf);
                let lang = self.code_lang.take();
                let avail = self.content_width();

                // Attempt syntax highlighting if a language was specified.
                let highlighted = lang
                    .as_deref()
                    .and_then(|l| highlight::highlight_code(&buf, l));

                if let Some(styled_lines) = highlighted {
                    for fragments in styled_lines {
                        let char_len: usize =
                            fragments.iter().map(|(t, _)| t.chars().count()).sum();
                        if char_len <= avail {
                            self.emit_line(fragments);
                        } else {
                            self.emit_line(truncate_styled_spans(fragments, avail));
                        }
                    }
                } else {
                    // Fallback: plain cyan.
                    let code_style = Style::default().fg(theme::focus());
                    for line_text in buf.lines() {
                        let truncated = truncate_str(line_text, avail);
                        self.emit_line(vec![(truncated, code_style)]);
                    }
                }
                self.emit_blank();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                // Flush any inline content remaining in this list item.
                self.flush_paragraph();
                // Consume unused marker (e.g. empty item).
                self.pending_item_marker = None;
            }
            TagEnd::Emphasis => self.pop_style(),
            TagEnd::Strong => self.pop_style(),
            TagEnd::Strikethrough => self.pop_style(),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Dash detection (for word-wrapping)
// ---------------------------------------------------------------------------

/// Returns `true` if `word` starts with an em dash, en dash, or ASCII `--`
/// (ignoring any leading whitespace).
///
/// These should never begin a continuation line — a slight overflow is
/// preferable to a dash-initial line.
pub fn starts_with_dash(word: &str) -> bool {
    let w = word.trim_start();
    w.starts_with('\u{2014}') || w.starts_with('\u{2013}') || w.starts_with("--")
}

// ---------------------------------------------------------------------------
// Word-wrapping styled spans
// ---------------------------------------------------------------------------

/// Word-wrap a flat list of `(text, style)` spans into lines, where the first
/// line may use a different width than continuation lines (for hang indent).
///
/// Returns a vec of lines, each being a vec of `(text, style)` fragments.
fn wrap_spans_with_hang(
    spans: Vec<(String, Style)>,
    first_width: usize,
    cont_width: usize,
) -> Vec<Vec<(String, Style)>> {
    if spans.is_empty() {
        return vec![vec![]];
    }

    // Flatten into word-boundary chunks: Vec<(word, style)>.
    let mut chunks: Vec<(String, Style)> = Vec::new();
    for (text, style) in &spans {
        // Split keeping whitespace structure: each space-separated word is a chunk.
        // We preserve leading/trailing spaces within each span boundary as part of
        // the adjacent word to keep things simple.
        let mut first_word = true;
        for word in text.split(' ') {
            if first_word {
                first_word = false;
                chunks.push((word.to_string(), *style));
            } else if word.is_empty() {
                // Consecutive spaces — append a space to the previous chunk.
                if let Some(last) = chunks.last_mut() {
                    last.0.push(' ');
                }
            } else {
                // Normal word boundary — push with a leading space.
                chunks.push((format!(" {word}"), *style));
            }
        }
    }

    let mut lines: Vec<Vec<(String, Style)>> = Vec::new();
    let mut current_line: Vec<(String, Style)> = Vec::new();
    let mut current_len: usize = 0;
    let mut is_first_line = true;

    for (word, style) in chunks {
        let word_len = word.len();
        if word_len == 0 {
            continue;
        }

        let max_w = if is_first_line {
            first_width
        } else {
            cont_width
        };

        if current_len == 0 {
            // Start of a new line — strip any leading space from the chunk.
            let trimmed = word.trim_start().to_string();
            current_len = trimmed.len();
            current_line.push((trimmed, style));
        } else if current_len + word_len <= max_w {
            current_len += word_len;
            current_line.push((word, style));
        } else if starts_with_dash(&word) {
            // Keep dash-prefixed chunks on the current line to avoid
            // a typographically awkward dash at the start of a line.
            current_len += word_len;
            current_line.push((word, style));
        } else {
            // Wrap: push current line, start new one.
            lines.push(std::mem::take(&mut current_line));
            is_first_line = false;
            let trimmed = word.trim_start().to_string();
            current_len = trimmed.len();
            current_line.push((trimmed, style));
        }
    }

    if !current_line.is_empty() || lines.is_empty() {
        lines.push(current_line);
    }

    lines
}

/// Truncate a vec of styled spans to fit within `max_chars` characters,
/// appending an ellipsis if any content was cut.
fn truncate_styled_spans(
    fragments: Vec<(String, Style)>,
    max_chars: usize,
) -> Vec<(String, Style)> {
    let mut remaining = max_chars.saturating_sub(1); // reserve for ellipsis
    let mut out = Vec::new();
    for (text, style) in fragments {
        if remaining == 0 {
            break;
        }
        let char_count = text.chars().count();
        if char_count <= remaining {
            remaining -= char_count;
            out.push((text, style));
        } else {
            let partial: String = text.chars().take(remaining).collect();
            remaining = 0;
            out.push((partial, style));
        }
    }
    out.push(("\u{2026}".to_string(), Style::default()));
    out
}

/// Truncate a string to at most `max_chars` characters, appending `...` if
/// truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else if max_chars > 1 {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    } else {
        "\u{2026}".to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// Helper: render markdown and collect the raw text of each line.
    fn render_text(md: &str, width: usize) -> Vec<String> {
        let base = Style::default().fg(Color::Blue);
        let lines = render_markdown(md, width, base);
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    /// Helper: render and return (text, style) pairs per line for style assertions.
    fn render_styled(md: &str, width: usize) -> Vec<Vec<(String, Style)>> {
        let base = Style::default().fg(Color::Blue);
        let lines = render_markdown(md, width, base);
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| (s.content.to_string(), s.style))
                    .collect()
            })
            .collect()
    }

    // -- Plain text -----------------------------------------------------------

    #[test]
    fn plain_text_wraps() {
        let lines = render_text("hello world", 80);
        assert_eq!(lines, vec!["hello world"]);
    }

    #[test]
    fn plain_text_word_wraps_at_width() {
        let lines = render_text("aaa bbb ccc", 7);
        // "aaa bbb" fits in 7 chars, "ccc" wraps.
        assert_eq!(lines, vec!["aaa bbb", "ccc"]);
    }

    // -- Bold -----------------------------------------------------------------

    #[test]
    fn bold_text_has_bold_modifier() -> Result<(), Box<dyn std::error::Error>> {
        let styled = render_styled("**bold**", 80);
        let first_line = &styled[0];
        let bold_span = first_line
            .iter()
            .find(|(text, _)| text == "bold")
            .ok_or("should have 'bold' span")?;
        assert!(bold_span.1.add_modifier.contains(Modifier::BOLD));
        Ok(())
    }

    // -- Italic ---------------------------------------------------------------

    #[test]
    fn italic_text_has_italic_modifier() -> Result<(), Box<dyn std::error::Error>> {
        let styled = render_styled("*italic*", 80);
        let first_line = &styled[0];
        let italic_span = first_line
            .iter()
            .find(|(text, _)| text == "italic")
            .ok_or("should have 'italic' span")?;
        assert!(italic_span.1.add_modifier.contains(Modifier::ITALIC));
        Ok(())
    }

    // -- Inline code ----------------------------------------------------------

    #[test]
    fn inline_code_is_cyan() -> Result<(), Box<dyn std::error::Error>> {
        let styled = render_styled("use `code` here", 80);
        let first_line = &styled[0];
        let code_span = first_line
            .iter()
            .find(|(text, _)| text == "code")
            .ok_or("should have 'code' span")?;
        assert_eq!(code_span.1.fg, Some(Color::Cyan));
        Ok(())
    }

    // -- Code blocks ----------------------------------------------------------

    #[test]
    fn code_block_preformatted_cyan() {
        let md = "```\nfn main() {}\nlet x = 1;\n```";
        let styled = render_styled(md, 80);
        // Code block lines should be Cyan.
        let code_line = &styled[0];
        assert_eq!(code_line[0].0, "fn main() {}");
        assert_eq!(code_line[0].1.fg, Some(Color::Cyan));
    }

    #[test]
    fn code_block_truncated_not_wrapped() {
        let md = "```\nabcdefghij\n```";
        let lines = render_text(md, 5);
        // Should be truncated to 5 chars (not wrapped). The truncation
        // appends a single-char ellipsis, so char count is 5.
        assert_eq!(lines[0].chars().count(), 5);
    }

    // -- Lists ----------------------------------------------------------------

    #[test]
    fn unordered_list_has_bullet_prefix() {
        let md = "- item one\n- item two";
        let lines = render_text(md, 80);
        assert!(lines[0].starts_with("- "));
        assert!(lines[0].contains("item one"));
    }

    #[test]
    fn ordered_list_has_number_prefix() {
        let md = "1. first\n2. second";
        let lines = render_text(md, 80);
        assert!(lines[0].starts_with("1. "));
        assert!(lines[0].contains("first"));
    }

    #[test]
    fn list_continuation_hang_indented() {
        // A list item that wraps should have hang indent past the marker.
        let md = "- this is a very long item that should wrap at some point in the line";
        let lines = render_text(md, 30);
        // First line has "- " prefix.
        assert!(lines[0].starts_with("- "));
        // Continuation lines should be indented by 2 (width of "- ").
        if lines.len() > 1 && !lines[1].is_empty() {
            assert!(lines[1].starts_with("  "));
        }
    }

    // -- Blockquotes ----------------------------------------------------------

    #[test]
    fn blockquote_has_green_prefix() {
        let md = "> quoted text";
        let styled = render_styled(md, 80);
        let first_line = &styled[0];
        // First span should be "> " in green.
        assert_eq!(first_line[0].0, "> ");
        assert_eq!(first_line[0].1.fg, Some(Color::Green));
    }

    // -- Nested formatting ----------------------------------------------------

    #[test]
    fn nested_bold_italic() -> Result<(), Box<dyn std::error::Error>> {
        let styled = render_styled("***both***", 80);
        let first_line = &styled[0];
        let span = first_line
            .iter()
            .find(|(text, _)| text == "both")
            .ok_or("should have 'both' span")?;
        assert!(span.1.add_modifier.contains(Modifier::BOLD));
        assert!(span.1.add_modifier.contains(Modifier::ITALIC));
        Ok(())
    }

    // -- Edge cases -----------------------------------------------------------

    #[test]
    fn empty_text_returns_single_empty_line() {
        let lines = render_text("", 80);
        // Should produce at least one empty line — not panic.
        assert!(!lines.is_empty());
    }

    #[test]
    fn zero_width_returns_default_line() {
        let base = Style::default().fg(Color::Blue);
        let lines = render_markdown("hello", 0, base);
        assert_eq!(lines.len(), 1);
    }

    // -- Headers --------------------------------------------------------------

    #[test]
    fn heading_is_bold() -> Result<(), Box<dyn std::error::Error>> {
        let styled = render_styled("# Title", 80);
        let first_line = &styled[0];
        let title_span = first_line
            .iter()
            .find(|(text, _)| text == "Title")
            .ok_or("should have 'Title' span")?;
        assert!(title_span.1.add_modifier.contains(Modifier::BOLD));
        Ok(())
    }

    // -- Dash wrapping --------------------------------------------------------

    #[test]
    fn wrap_keeps_double_dash_on_previous_line() {
        // At width 25, "This is a long sentence" (23 chars) fits but adding
        // " -- and" would overflow. The "--" must stay on the first line.
        let lines = render_text("This is a long sentence -- and continues", 25);
        for line in &lines[1..] {
            assert!(
                !line.starts_with("--"),
                "continuation line should not start with '--': {lines:?}"
            );
        }
    }

    #[test]
    fn wrap_keeps_em_dash_on_previous_line() {
        let lines = render_text("Short text here \u{2014}and more words follow", 20);
        for line in &lines[1..] {
            assert!(
                !line.starts_with('\u{2014}'),
                "continuation line should not start with em dash: {lines:?}"
            );
        }
    }

    #[test]
    fn wrap_keeps_en_dash_on_previous_line() {
        let lines = render_text("Short text here \u{2013}and more words follow", 20);
        for line in &lines[1..] {
            assert!(
                !line.starts_with('\u{2013}'),
                "continuation line should not start with en dash: {lines:?}"
            );
        }
    }
}

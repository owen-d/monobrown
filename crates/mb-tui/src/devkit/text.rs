use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// Style attributes for a single cell, used to track style transitions.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    fg: Color,
    bg: Color,
    modifier: Modifier,
}

impl CellStyle {
    fn from_cell(cell: &ratatui::buffer::Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            modifier: cell.modifier,
        }
    }

    fn is_default(&self) -> bool {
        is_default_style(self.fg, self.bg, self.modifier)
    }
}

/// Convert buffer to plain text, stripping trailing whitespace per line
/// and trailing empty lines.
pub fn buffer_to_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut lines = Vec::with_capacity(area.height as usize);

    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            let cell = &buf[(x, y)];
            line.push_str(cell.symbol());
        }
        lines.push(line.trim_end().to_string());
    }

    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    lines.join("\n")
}

/// Convert buffer to styled text with inline annotations.
///
/// Style tags use the format: `<fg:red,bold>text</>`
/// Only styled runs get annotated; unstyled text passes through clean.
pub fn buffer_to_styled_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut lines = Vec::with_capacity(area.height as usize);

    for y in area.y..area.y + area.height {
        lines.push(render_styled_line(buf, y, area.x, area.width));
    }

    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    lines.join("\n")
}

/// Render a single styled line from the buffer, closing open tags
/// and trimming trailing whitespace.
fn render_styled_line(buf: &Buffer, y: u16, start_x: u16, width: u16) -> String {
    let mut line = String::new();
    let mut current_style: Option<CellStyle> = None;

    for x in start_x..start_x + width {
        let cell = &buf[(x, y)];
        let cell_style = CellStyle::from_cell(cell);

        if cell_style.is_default() {
            // Close any open style tag before emitting unstyled content.
            if current_style.is_some() {
                line.push_str("</>");
                current_style = None;
            }
        } else if current_style != Some(cell_style) {
            // Style changed: close previous tag (if any), open new one.
            if current_style.is_some() {
                line.push_str("</>");
            }
            line.push_str(&format_open_tag(
                cell_style.fg,
                cell_style.bg,
                cell_style.modifier,
            ));
            current_style = Some(cell_style);
        }

        line.push_str(cell.symbol());
    }

    if current_style.is_some() {
        line.push_str("</>");
    }

    trim_styled_line_end(&line)
}

/// Trim trailing whitespace from a styled line.
///
/// A naive `trim_end()` can't see through a closing `</>` tag, so we
/// peel it off, trim whitespace, and then decide whether to re-close.
/// If trimming eats all content back to the open tag, we drop both tags.
fn trim_styled_line_end(line: &str) -> String {
    const CLOSE: &str = "</>";

    let Some(inner) = line.strip_suffix(CLOSE) else {
        return line.trim_end().to_string();
    };

    let trimmed = inner.trim_end();

    // Nothing left, or only whitespace was inside the tags.
    if trimmed.is_empty() {
        return String::new();
    }

    // If trimming consumed all content after the open tag, we're left with
    // just `<open_tag>` -- drop the empty span entirely.
    if let Some(open_start) = trimmed.rfind('<') {
        let tag_candidate = &trimmed[open_start..];
        if !tag_candidate.starts_with("</") && tag_candidate.ends_with('>') {
            // The suffix is a bare open tag with no visible content after it.
            let before_tag = trimmed[..open_start].trim_end();
            if before_tag.is_empty() {
                return String::new();
            }
            return before_tag.to_string();
        }
    }

    format!("{trimmed}{CLOSE}")
}

/// Convert buffer to text with ANSI escape codes for terminal rendering.
///
/// Same trimming semantics as `buffer_to_text`: trailing whitespace per line,
/// trailing empty lines removed.
pub fn buffer_to_ansi(buf: &Buffer) -> String {
    let area = buf.area;
    let mut lines = Vec::with_capacity(area.height as usize);

    for y in area.y..area.y + area.height {
        lines.push(render_ansi_line(buf, y, area.x, area.width));
    }

    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    lines.join("\n")
}

/// Render a single line from the buffer with ANSI escape codes.
///
/// Collects `(symbol, style)` pairs, trims trailing whitespace from the
/// symbols, then emits ANSI SGR sequences only for the trimmed content.
fn render_ansi_line(buf: &Buffer, y: u16, start_x: u16, width: u16) -> String {
    // Collect cells: (symbol, style).
    let cells: Vec<(&str, CellStyle)> = (start_x..start_x + width)
        .map(|x| {
            let cell = &buf[(x, y)];
            (cell.symbol(), CellStyle::from_cell(cell))
        })
        .collect();

    // Trim trailing whitespace cells.
    let trimmed = trim_trailing_whitespace(&cells);

    // Emit ANSI codes for the trimmed content.
    emit_ansi_for_cells(trimmed)
}

/// Trim trailing whitespace (space-only symbols with default style, or any
/// style) from the cell slice.
fn trim_trailing_whitespace<'a>(cells: &'a [(&str, CellStyle)]) -> &'a [(&'a str, CellStyle)] {
    let end = cells
        .iter()
        .rposition(|(sym, _)| !sym.trim_end().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    &cells[..end]
}

/// Emit ANSI SGR sequences for a slice of (symbol, style) cells.
fn emit_ansi_for_cells(cells: &[(&str, CellStyle)]) -> String {
    let mut out = String::new();
    let mut current_style: Option<CellStyle> = None;

    for &(sym, style) in cells {
        if style.is_default() {
            if current_style.is_some() {
                out.push_str("\x1b[0m");
                current_style = None;
            }
        } else if current_style != Some(style) {
            out.push_str("\x1b[0m");
            out.push_str(&build_sgr(style));
            current_style = Some(style);
        }
        out.push_str(sym);
    }

    if current_style.is_some() {
        out.push_str("\x1b[0m");
    }

    out
}

/// Build an ANSI SGR (Select Graphic Rendition) escape sequence for a style.
fn build_sgr(style: CellStyle) -> String {
    let mut codes: Vec<String> = Vec::new();

    append_modifier_codes(&mut codes, style.modifier);
    append_fg_code(&mut codes, style.fg);
    append_bg_code(&mut codes, style.bg);

    if codes.is_empty() {
        return String::new();
    }

    format!("\x1b[{}m", codes.join(";"))
}

fn append_fg_code(codes: &mut Vec<String>, fg: Color) {
    if let Some(code) = color_to_ansi(fg, 30) {
        codes.push(code);
    }
}

fn append_bg_code(codes: &mut Vec<String>, bg: Color) {
    if let Some(code) = color_to_ansi(bg, 40) {
        codes.push(code);
    }
}

/// Convert a ratatui Color to its ANSI SGR parameter string.
///
/// `base` selects foreground (30) or background (40). Named colors offset
/// from `base`, bright variants from `base + 60`, and RGB/indexed use the
/// extended prefix `base + 8`.
fn color_to_ansi(color: Color, base: u8) -> Option<String> {
    let code = match color {
        Color::Reset => return None,
        Color::Black => base,
        Color::Red => base + 1,
        Color::Green => base + 2,
        Color::Yellow => base + 3,
        Color::Blue => base + 4,
        Color::Magenta => base + 5,
        Color::Cyan => base + 6,
        Color::Gray => base + 7,
        Color::DarkGray => base + 60,
        Color::LightRed => base + 61,
        Color::LightGreen => base + 62,
        Color::LightYellow => base + 63,
        Color::LightBlue => base + 64,
        Color::LightMagenta => base + 65,
        Color::LightCyan => base + 66,
        Color::White => base + 67,
        Color::Rgb(r, g, b) => return Some(format!("{};2;{r};{g};{b}", base + 8)),
        Color::Indexed(n) => return Some(format!("{};5;{n}", base + 8)),
    };
    Some(code.to_string())
}

fn append_modifier_codes(codes: &mut Vec<String>, modifier: Modifier) {
    const MODIFIER_CODES: &[(Modifier, &str)] = &[
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::SLOW_BLINK, "5"),
        (Modifier::RAPID_BLINK, "6"),
        (Modifier::REVERSED, "7"),
        (Modifier::HIDDEN, "8"),
        (Modifier::CROSSED_OUT, "9"),
    ];
    for &(flag, code) in MODIFIER_CODES {
        if modifier.contains(flag) {
            codes.push(code.to_string());
        }
    }
}

fn is_default_style(fg: Color, bg: Color, modifier: Modifier) -> bool {
    fg == Color::Reset && bg == Color::Reset && modifier.is_empty()
}

/// Build an opening tag like `<fg:red,bold,bg:blue>`.
fn format_open_tag(fg: Color, bg: Color, modifier: Modifier) -> String {
    let mut attrs = Vec::new();

    if fg != Color::Reset {
        attrs.push(format!("fg:{}", format_color(fg)));
    }
    if bg != Color::Reset {
        attrs.push(format!("bg:{}", format_color(bg)));
    }
    append_modifier_attrs(&mut attrs, modifier);

    format!("<{}>", attrs.join(","))
}

fn format_color(color: Color) -> Cow<'static, str> {
    match color {
        Color::Reset => "reset".into(),
        Color::Black => "black".into(),
        Color::Red => "red".into(),
        Color::Green => "green".into(),
        Color::Yellow => "yellow".into(),
        Color::Blue => "blue".into(),
        Color::Magenta => "magenta".into(),
        Color::Cyan => "cyan".into(),
        Color::Gray => "gray".into(),
        Color::DarkGray => "darkgray".into(),
        Color::LightRed => "lightred".into(),
        Color::LightGreen => "lightgreen".into(),
        Color::LightYellow => "lightyellow".into(),
        Color::LightBlue => "lightblue".into(),
        Color::LightMagenta => "lightmagenta".into(),
        Color::LightCyan => "lightcyan".into(),
        Color::White => "white".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}").into(),
        Color::Indexed(n) => format!("idx:{n}").into(),
    }
}

fn append_modifier_attrs(attrs: &mut Vec<String>, modifier: Modifier) {
    const MODIFIER_NAMES: &[(Modifier, &str)] = &[
        (Modifier::BOLD, "bold"),
        (Modifier::DIM, "dim"),
        (Modifier::ITALIC, "italic"),
        (Modifier::UNDERLINED, "underline"),
        (Modifier::SLOW_BLINK, "slow_blink"),
        (Modifier::RAPID_BLINK, "rapid_blink"),
        (Modifier::REVERSED, "reversed"),
        (Modifier::HIDDEN, "hidden"),
        (Modifier::CROSSED_OUT, "strikethrough"),
    ];
    for &(flag, name) in MODIFIER_NAMES {
        if modifier.contains(flag) {
            attrs.push(name.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};

    use super::*;

    fn make_buf(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    #[test]
    fn plain_text_basic() {
        let mut buf = make_buf(10, 2);
        buf.set_stringn(0, 0, "Hello", 10, Style::default());
        buf.set_stringn(0, 1, "World", 10, Style::default());

        let text = buffer_to_text(&buf);
        assert_eq!(text, "Hello\nWorld");
    }

    #[test]
    fn plain_text_strips_trailing_whitespace() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "Hi", 10, Style::default());

        let text = buffer_to_text(&buf);
        assert_eq!(text, "Hi");
    }

    #[test]
    fn plain_text_strips_trailing_empty_lines() {
        let mut buf = make_buf(10, 4);
        buf.set_stringn(0, 0, "Line1", 10, Style::default());
        // Lines 1-3 are empty (spaces)

        let text = buffer_to_text(&buf);
        assert_eq!(text, "Line1");
    }

    #[test]
    fn styled_text_unstyled_passthrough() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "plain", 10, Style::default());

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "plain");
    }

    #[test]
    fn styled_text_fg_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "red", 10, Style::default().fg(Color::Red));

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<fg:red>red</>");
    }

    #[test]
    fn styled_text_bold_and_fg() {
        let mut buf = make_buf(10, 1);
        let style = Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD);
        buf.set_stringn(0, 0, "go", 10, style);

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<fg:green,bold>go</>");
    }

    #[test]
    fn styled_text_mixed_runs() {
        let mut buf = make_buf(20, 1);
        buf.set_stringn(0, 0, "ok ", 20, Style::default());
        buf.set_stringn(3, 0, "err", 17, Style::default().fg(Color::Red));
        buf.set_stringn(6, 0, " fin", 14, Style::default());

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "ok <fg:red>err</> fin");
    }

    #[test]
    fn styled_text_style_change_mid_line() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "AB", 10, Style::default().fg(Color::Red));
        buf.set_stringn(2, 0, "CD", 8, Style::default().fg(Color::Blue));

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<fg:red>AB</><fg:blue>CD</>");
    }

    #[test]
    fn styled_text_bg_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "hi", 10, Style::default().bg(Color::Yellow));

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<bg:yellow>hi</>");
    }

    #[test]
    fn styled_text_rgb_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "x", 10, Style::default().fg(Color::Rgb(255, 128, 0)));

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<fg:#ff8000>x</>");
    }

    #[test]
    fn styled_text_indexed_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "y", 10, Style::default().fg(Color::Indexed(42)));

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<fg:idx:42>y</>");
    }

    #[test]
    fn format_color_all_named() {
        assert_eq!(format_color(Color::Black), "black");
        assert_eq!(format_color(Color::White), "white");
        assert_eq!(format_color(Color::DarkGray), "darkgray");
        assert_eq!(format_color(Color::LightCyan), "lightcyan");
    }

    #[test]
    fn modifier_multiple_flags() {
        let mut attrs = Vec::new();
        let mods = Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED;
        append_modifier_attrs(&mut attrs, mods);
        assert_eq!(attrs, vec!["bold", "italic", "underline"]);
    }

    #[test]
    fn styled_text_trailing_style_trimmed() {
        // If styled spaces at end of line, the closing tag should appear
        // before any trimmed whitespace.
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "hi", 10, Style::default().bold());
        // Remaining 8 cells are default spaces -- those get trimmed.

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<bold>hi</>");
    }

    #[test]
    fn styled_text_trailing_styled_whitespace_trimmed() {
        // When trailing whitespace is *inside* a styled run, the close
        // tag must not prevent trimming.
        let mut buf = make_buf(10, 1);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        // First 2 chars "hi" are bold, remaining 8 cells are also bold spaces.
        buf.set_stringn(0, 0, "hi", 10, bold);
        for x in 2..10 {
            buf[(x, 0)].set_style(bold);
        }

        let text = buffer_to_styled_text(&buf);
        assert_eq!(text, "<bold>hi</>");
    }

    // --- ANSI output tests ---

    #[test]
    fn ansi_unstyled_passthrough() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "plain", 10, Style::default());

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "plain");
    }

    #[test]
    fn ansi_fg_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "red", 10, Style::default().fg(Color::Red));

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[31mred\x1b[0m");
    }

    #[test]
    fn ansi_bold_and_fg() {
        let mut buf = make_buf(10, 1);
        let style = Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD);
        buf.set_stringn(0, 0, "go", 10, style);

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[1;32mgo\x1b[0m");
    }

    #[test]
    fn ansi_mixed_runs() {
        let mut buf = make_buf(20, 1);
        buf.set_stringn(0, 0, "ok ", 20, Style::default());
        buf.set_stringn(3, 0, "err", 17, Style::default().fg(Color::Red));
        buf.set_stringn(6, 0, " fin", 14, Style::default());

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "ok \x1b[0m\x1b[31merr\x1b[0m fin");
    }

    #[test]
    fn ansi_style_change_mid_line() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "AB", 10, Style::default().fg(Color::Red));
        buf.set_stringn(2, 0, "CD", 8, Style::default().fg(Color::Blue));

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[31mAB\x1b[0m\x1b[34mCD\x1b[0m");
    }

    #[test]
    fn ansi_bg_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "hi", 10, Style::default().bg(Color::Yellow));

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[43mhi\x1b[0m");
    }

    #[test]
    fn ansi_rgb_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "x", 10, Style::default().fg(Color::Rgb(255, 128, 0)));

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[38;2;255;128;0mx\x1b[0m");
    }

    #[test]
    fn ansi_indexed_color() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "y", 10, Style::default().fg(Color::Indexed(42)));

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[38;5;42my\x1b[0m");
    }

    #[test]
    fn ansi_trailing_whitespace_trimmed() {
        let mut buf = make_buf(10, 1);
        buf.set_stringn(0, 0, "hi", 10, Style::default().bold());

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn ansi_trailing_styled_whitespace_trimmed() {
        let mut buf = make_buf(10, 1);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        buf.set_stringn(0, 0, "hi", 10, bold);
        for x in 2..10 {
            buf[(x, 0)].set_style(bold);
        }

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn ansi_trailing_empty_lines_trimmed() {
        let mut buf = make_buf(10, 4);
        buf.set_stringn(0, 0, "Line1", 10, Style::default());

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "Line1");
    }

    #[test]
    fn ansi_multiline() {
        let mut buf = make_buf(10, 2);
        buf.set_stringn(0, 0, "Hello", 10, Style::default().fg(Color::Green));
        buf.set_stringn(0, 1, "World", 10, Style::default());

        let text = buffer_to_ansi(&buf);
        assert_eq!(text, "\x1b[0m\x1b[32mHello\x1b[0m\nWorld");
    }
}

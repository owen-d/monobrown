use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Shared overflow behavior for adaptive widgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowBehavior {
    Clip,
    Ellipsis,
    Summary,
}

impl OverflowBehavior {
    /// Inline text treatment associated with this behavior.
    pub const fn text_overflow(self) -> TextOverflow {
        match self {
            Self::Clip => TextOverflow::Clip,
            Self::Ellipsis | Self::Summary => TextOverflow::Ellipsis,
        }
    }
}

/// Shared overflow behavior for single-line text fragments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineOverflow {
    Clip,
    Ellipsis,
}

/// Alias used by the layout tests and widget measurement code.
pub type TextOverflow = InlineOverflow;

/// Display width for inline text.
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Alias for `display_width`.
pub fn text_width(text: &str) -> usize {
    display_width(text)
}

/// Apply the requested inline overflow behavior to `text`.
pub fn overflow_text(text: &str, width: usize, overflow: InlineOverflow) -> String {
    match overflow {
        InlineOverflow::Clip => clip_text(text, width),
        InlineOverflow::Ellipsis => ellipsize_text(text, width),
    }
}

/// Alias for `overflow_text`.
pub fn fit_text(text: &str, width: usize, overflow: TextOverflow) -> String {
    overflow_text(text, width, overflow)
}

/// Clip text to the given display width.
pub fn clip_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

/// Ellipsize text to the given display width.
pub fn ellipsize_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_width(text) <= width {
        return text.to_string();
    }

    const ELLIPSIS: &str = "\u{2026}";
    let ellipsis_width = display_width(ELLIPSIS);
    if width <= ellipsis_width {
        return ELLIPSIS.to_string();
    }

    let mut out = clip_text(text, width - ellipsis_width);
    out.push('\u{2026}');
    out
}

/// Build a summary line that preserves `prefix`, ellipsizes the body, and
/// right-aligns `suffix` when there is room.
pub fn summarize_text(prefix: &str, body: &str, suffix: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let prefix = clip_text(prefix, width);
    let prefix_width = display_width(&prefix);
    if prefix_width >= width {
        return prefix;
    }

    let suffix_width = display_width(suffix).min(width - prefix_width);
    let suffix = clip_text(suffix, suffix_width);
    let remaining = width - prefix_width - suffix_width;
    let body = ellipsize_text(body, remaining);
    let used = prefix_width + display_width(&body) + suffix_width;

    let mut out = String::with_capacity(prefix.len() + body.len() + suffix.len());
    out.push_str(&prefix);
    out.push_str(&body);
    if suffix_width > 0 && used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out.push_str(&suffix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_text_respects_display_width() {
        assert_eq!(clip_text("abcdef", 4), "abcd");
        assert_eq!(clip_text("界界a", 4), "界界");
    }

    #[test]
    fn ellipsize_text_appends_ellipsis() {
        assert_eq!(ellipsize_text("abcdef", 4), "abc\u{2026}");
        assert_eq!(ellipsize_text("abcdef", 1), "\u{2026}");
    }

    #[test]
    fn summarize_text_preserves_suffix() {
        assert_eq!(
            summarize_text("", "alphabet", " 42%", 9),
            "alph\u{2026} 42%"
        );
        assert_eq!(summarize_text("2/3 ", "selected", "", 7), "2/3 se\u{2026}");
    }

    #[test]
    fn summary_maps_to_inline_ellipsis() {
        assert_eq!(
            OverflowBehavior::Summary.text_overflow(),
            TextOverflow::Ellipsis
        );
    }
}

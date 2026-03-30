//! Syntax highlighting for fenced code blocks.
//!
//! Uses the theme module's bundled syntaxes and themes to convert
//! source code into styled ratatui [`Line`]s. Theme selection adapts to the
//! terminal background (light vs dark) via [`crate::theme`].

use ratatui::style::{Color, Style};
use syntect::easy::HighlightLines;
use syntect::parsing::SyntaxReference;

use crate::theme;

// ---------------------------------------------------------------------------
// Guardrail constants
// ---------------------------------------------------------------------------

/// Maximum input size (512 KB) before falling back to plain rendering.
const MAX_BYTES: usize = 512 * 1024;

/// Maximum line count before falling back to plain rendering.
const MAX_LINES: usize = 10_000;

// ---------------------------------------------------------------------------
// Language lookup
// ---------------------------------------------------------------------------

/// Common language aliases not covered by syntect's built-in matching.
fn resolve_alias(lang: &str) -> &str {
    match lang {
        "csharp" => "c#",
        "golang" => "go",
        "js" => "javascript",
        "ts" => "typescript",
        "tsx" => "typescript",
        "jsx" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "rs" => "rust",
        "sh" | "bash" | "zsh" => "bourne again shell (bash)",
        "yml" => "yaml",
        "md" => "markdown",
        "dockerfile" => "docker",
        "tf" => "terraform",
        "proto" => "protobuf",
        "el" | "elisp" => "lisp",
        _ => lang,
    }
}

/// Find a syntax definition for the given language token.
///
/// Tries (in order): exact token match, extension match, alias resolution.
fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let ss = theme::syntax_set();

    // Try exact token match first.
    if let Some(syn) = ss.find_syntax_by_token(lang) {
        return Some(syn);
    }

    // Try by extension.
    if let Some(syn) = ss.find_syntax_by_extension(lang) {
        return Some(syn);
    }

    // Try common aliases.
    let alias = resolve_alias(lang);
    if alias != lang {
        if let Some(syn) = ss.find_syntax_by_token(alias) {
            return Some(syn);
        }
        if let Some(syn) = ss.find_syntax_by_name(alias) {
            return Some(syn);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Core highlighting
// ---------------------------------------------------------------------------

/// Highlight source code and return per-line styled fragments.
///
/// Returns `None` if:
/// - The language is unknown
/// - The input exceeds guardrail limits (512 KB or 10,000 lines)
/// - Highlighting encounters an error
///
/// Each inner `Vec` represents one source line as `(text, ratatui::Style)` pairs.
/// Only foreground color is mapped; background, italic, and underline are
/// skipped for broad terminal compatibility.
pub fn highlight_code(code: &str, lang: &str) -> Option<Vec<Vec<(String, Style)>>> {
    // Guardrails.
    if code.len() > MAX_BYTES || code.lines().count() > MAX_LINES {
        return None;
    }

    let syntax = find_syntax(lang)?;
    let mut highlighter = HighlightLines::new(syntax, theme::code_theme());

    let mut result = Vec::new();
    for line in code.lines() {
        let ranges = highlighter.highlight_line(line, theme::syntax_set()).ok()?;

        let spans: Vec<(String, Style)> = ranges
            .into_iter()
            .map(|(style, text)| {
                let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                (text.to_string(), Style::default().fg(fg))
            })
            .collect();

        result.push(spans);
    }

    Some(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_syntax_known_languages() {
        // These should all resolve to a syntax definition.
        for lang in &["rust", "python", "javascript", "go", "c", "java", "ruby"] {
            assert!(find_syntax(lang).is_some(), "expected syntax for '{lang}'");
        }
    }

    #[test]
    fn find_syntax_aliases() {
        assert!(find_syntax("rs").is_some());
        assert!(find_syntax("py").is_some());
        assert!(find_syntax("js").is_some());
        assert!(find_syntax("ts").is_some());
        assert!(find_syntax("golang").is_some());
        assert!(find_syntax("csharp").is_some());
        assert!(find_syntax("sh").is_some());
        assert!(find_syntax("bash").is_some());
        assert!(find_syntax("yml").is_some());
    }

    #[test]
    fn find_syntax_unknown_returns_none() {
        assert!(find_syntax("not_a_real_language_xyz").is_none());
    }

    #[test]
    fn highlight_code_rust_snippet() -> Result<(), Box<dyn std::error::Error>> {
        let code = "fn main() {\n    println!(\"hello\");\n}";
        let lines = highlight_code(code, "rust").ok_or("rust should be highlighted")?;
        assert_eq!(lines.len(), 3);
        // Each line should have at least one span.
        for line in &lines {
            assert!(!line.is_empty(), "each line should have spans");
        }
        Ok(())
    }

    #[test]
    fn highlight_code_unknown_lang_returns_none() {
        let result = highlight_code("some code", "not_a_language");
        assert!(result.is_none());
    }

    #[test]
    fn highlight_code_oversized_input_returns_none() {
        // Exceeds MAX_BYTES (512 KB).
        let big = "x".repeat(MAX_BYTES + 1);
        let result = highlight_code(&big, "rust");
        assert!(result.is_none());
    }

    #[test]
    fn highlight_code_too_many_lines_returns_none() {
        // Exceeds MAX_LINES (10,000).
        let many_lines: String = (0..=MAX_LINES)
            .map(|i| format!("let x{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = highlight_code(&many_lines, "rust");
        assert!(result.is_none());
    }
}

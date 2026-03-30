//! Centralized color and theme definitions for the TUI.
//!
//! All UI code should use these functions instead of hard-coding colors.
//! Colors adapt to the terminal background (light vs dark) detected by
//! [`palette`]. Code highlighting themes are also exposed here,
//! delegating to `syntect` / `two_face` under the hood.

pub mod palette;

use std::sync::OnceLock;

use ratatui::style::Color;
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;
use two_face::theme::EmbeddedThemeName;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_light() -> bool {
    palette::is_light()
}

/// Define an adaptive color function that returns one color on light
/// backgrounds and another on dark backgrounds.
macro_rules! adaptive_color {
    ($(#[$meta:meta])* $name:ident, $light:expr, $dark:expr) => {
        $(#[$meta])*
        pub fn $name() -> Color {
            if is_light() { $light } else { $dark }
        }
    };
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

adaptive_color!(
    /// Subdued border color for unfocused panes and separators.
    border, Color::Rgb(180, 180, 180), Color::Rgb(120, 120, 120)
);
adaptive_color!(
    /// Secondary/dim text -- timestamps, metadata, inactive labels,
    /// tool names, status indicators.
    dim, Color::Rgb(100, 100, 100), Color::Rgb(160, 160, 160)
);
adaptive_color!(
    /// Cursor highlight color for tree navigation.
    cursor, Color::Rgb(180, 120, 40), Color::Rgb(210, 160, 80)
);

// ---------------------------------------------------------------------------
// Semantic
// ---------------------------------------------------------------------------

adaptive_color!(
    /// Focus/accent color -- focused borders, active UI elements, user messages.
    focus, Color::Rgb(0, 140, 180), Color::Cyan
);
adaptive_color!(
    /// Success -- completed agents, passing results.
    success, Color::Rgb(0, 140, 60), Color::Green
);
adaptive_color!(
    /// Error -- failed agents, error results.
    error, Color::Rgb(200, 40, 40), Color::Red
);
adaptive_color!(
    /// Warning/in-progress -- running agents, retreat indicators.
    warning, Color::Rgb(180, 130, 0), Color::Yellow
);
adaptive_color!(
    /// Assistant text color.
    assistant, Color::Rgb(30, 80, 180), Color::Blue
);
adaptive_color!(
    /// Primary text color.
    text, Color::Rgb(30, 30, 30), Color::White
);
adaptive_color!(
    /// Text rendered on top of colored accent backgrounds (badges, highlights).
    text_on_accent, Color::White, Color::Black
);

// ---------------------------------------------------------------------------
// Code highlighting (syntect + two_face)
// ---------------------------------------------------------------------------

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

/// Bundled syntax definitions (250+ languages).
pub fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Syntax highlighting theme, chosen adaptively based on terminal background.
pub fn code_theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let theme_set = two_face::theme::extra();
        let name = if is_light() {
            EmbeddedThemeName::CatppuccinLatte
        } else {
            EmbeddedThemeName::CatppuccinMocha
        };
        theme_set.get(name).clone()
    })
}

//! Pure palette selection for TUI rendering.
//!
//! Terminal probing lives in [`crate::terminal::palette`]. Keeping this
//! module pure makes rendering deterministic and prevents OSC responses from
//! entering a consumer's normal key-event stream.

use std::sync::atomic::{AtomicU8, Ordering};

/// A deterministic light or dark rendering palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Palette {
    /// Light foregrounds intended for a dark terminal background.
    #[default]
    Dark,
    /// Dark foregrounds intended for a light terminal background.
    Light,
}

impl Palette {
    /// Select a palette from an observed RGB background.
    pub fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        let luminance = 0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue);
        if luminance > 128.0 {
            Self::Light
        } else {
            Self::Dark
        }
    }

    /// Whether this palette targets a light background.
    pub const fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Set the active palette explicitly before constructing widgets.
pub fn set(palette: Palette) {
    ACTIVE.store(u8::from(palette.is_light()), Ordering::Relaxed);
}

/// Return the active palette, defaulting to dark.
pub fn current() -> Palette {
    if ACTIVE.load(Ordering::Relaxed) == 1 {
        Palette::Light
    } else {
        Palette::Dark
    }
}

/// Whether the active palette targets a light background.
pub fn is_light() -> bool {
    current().is_light()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_selection_is_conventional() {
        assert_eq!(Palette::from_rgb(20, 20, 20), Palette::Dark);
        assert_eq!(Palette::from_rgb(240, 240, 240), Palette::Light);
    }

    #[test]
    fn explicit_selection_is_observable() {
        set(Palette::Light);
        assert_eq!(current(), Palette::Light);
        set(Palette::Dark);
        assert_eq!(current(), Palette::Dark);
    }
}

pub mod event_queue;
pub mod modal;
mod render_effect;
mod render_scheduler;

pub use render_effect::{RenderEffect, RenderStep, apply_render_effect};
pub use render_scheduler::RenderScheduler;

/// Result of a key event being processed by a widget or layer.
///
/// Used at boundaries between nested modal layers (e.g., playground -> widget).
/// Each layer forwards keys inward first; if the inner layer returns `Ignored`,
/// the outer layer gets to handle the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyResult {
    /// The key was consumed by this layer. Outer layers should not handle it.
    Consumed,
    /// The key was not consumed. Outer layers may handle it.
    Ignored,
}

/// Modal editing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "NORMAL"),
            Self::Insert => write!(f, "INSERT"),
        }
    }
}

use std::time::Instant;

use super::render_scheduler::RenderScheduler;

/// Render intent emitted by state/input handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderEffect {
    /// The outer loop should schedule a redraw through `RenderScheduler`.
    ScheduleRender,
}

/// Result of a single state/input transition.
///
/// State handlers return a domain action plus an optional render effect,
/// allowing the driver to remain the only place that actually draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderStep<A> {
    pub action: A,
    pub effect: Option<RenderEffect>,
}

impl<A> RenderStep<A> {
    /// Build a step that does not request a redraw.
    pub const fn new(action: A) -> Self {
        Self {
            action,
            effect: None,
        }
    }

    /// Build a step that requests a redraw.
    pub const fn schedule_render(action: A) -> Self {
        Self {
            action,
            effect: Some(RenderEffect::ScheduleRender),
        }
    }
}

/// Apply a render effect to the shared scheduler.
pub fn apply_render_effect(
    scheduler: &mut RenderScheduler,
    now: Instant,
    effect: Option<RenderEffect>,
) {
    if matches!(effect, Some(RenderEffect::ScheduleRender)) {
        scheduler.schedule_render(now);
    }
}

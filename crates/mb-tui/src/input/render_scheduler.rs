//! Debounced render scheduling for the TUI event loop.
//!
//! `RenderScheduler` caps the render rate to avoid wasting frames when
//! events arrive in bursts, while still keeping spinner animations alive
//! during quiet periods when the driver opts into idle rendering.
//!
//! All time is passed as parameters (`Instant`) — no internal clock reads —
//! enabling deterministic unit tests.

use std::time::{Duration, Instant};

/// Minimum interval between renders (≈60 fps cap).
const DEBOUNCE_PERIOD: Duration = Duration::from_millis(16);

/// How often to re-render when idle (spinner animation).
const IDLE_RENDER_PERIOD: Duration = Duration::from_millis(33);

/// Debounced render scheduler.
///
/// The debounce window is anchored to the *first* event after the last
/// render — subsequent events within the window are batched, not pushed
/// forward.
pub struct RenderScheduler {
    last_render: Instant,
    /// `None` = idle (no pending events), `Some` = anchored deadline.
    render_deadline: Option<Instant>,
}

impl RenderScheduler {
    /// Create a new scheduler, recording the initial render time.
    pub fn new(now: Instant) -> Self {
        Self {
            last_render: now,
            render_deadline: None,
        }
    }

    /// Schedule a redraw in response to state/input work.
    ///
    /// If no deadline is set, anchors one at `now + DEBOUNCE_PERIOD`.
    /// If a deadline is already set, this is a no-op (the existing
    /// deadline is not pushed forward).
    pub fn schedule_render(&mut self, now: Instant) {
        if self.render_deadline.is_none() {
            self.render_deadline = Some(now + DEBOUNCE_PERIOD);
        }
    }

    /// Schedule an immediate redraw.
    pub fn schedule_render_now(&mut self, now: Instant) {
        self.render_deadline = Some(now);
    }

    /// Backwards-compatible alias for `schedule_render`.
    pub fn notify_event(&mut self, now: Instant) {
        self.schedule_render(now);
    }

    /// True when a debounced redraw is pending.
    pub fn has_pending_render(&self) -> bool {
        self.render_deadline.is_some()
    }

    /// Returns `true` if a render should happen now.
    ///
    /// A render is due when:
    /// - The debounce deadline has been reached, OR
    /// - The idle render period has elapsed since the last render and the
    ///   driver has opted into idle animation.
    pub fn should_render(&self, now: Instant, idle_render: bool) -> bool {
        if let Some(deadline) = self.render_deadline {
            now >= deadline
        } else if idle_render {
            now >= self.last_render + IDLE_RENDER_PERIOD
        } else {
            false
        }
    }

    /// Record that a render just happened — updates `last_render` and
    /// clears any pending deadline.
    pub fn record_render(&mut self, now: Instant) {
        self.last_render = now;
        self.render_deadline = None;
    }

    /// Returns the `Instant` at which the next render is expected.
    ///
    /// If a deadline is set, returns it. Otherwise returns
    /// `last_render + IDLE_RENDER_PERIOD` only when idle animation is enabled.
    pub fn next_render_at(&self, idle_render: bool) -> Option<Instant> {
        self.render_deadline
            .or_else(|| idle_render.then_some(self.last_render + IDLE_RENDER_PERIOD))
    }

    /// Returns the delay until the next render, if one is scheduled.
    pub fn time_until_next_render(&self, now: Instant, idle_render: bool) -> Option<Duration> {
        self.next_render_at(idle_render)
            .map(|deadline| deadline.saturating_duration_since(now))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn idle_render_fires_after_idle_period() {
        let t0 = Instant::now();
        let sched = RenderScheduler::new(t0);

        // Not yet due immediately after creation.
        assert!(!sched.should_render(t0, true));
        assert!(!sched.should_render(t0 + Duration::from_millis(20), true));

        // No idle redraw unless the driver opts in.
        assert!(!sched.should_render(t0 + IDLE_RENDER_PERIOD, false));

        // Due after idle period.
        assert!(sched.should_render(t0 + IDLE_RENDER_PERIOD, true));
        assert!(sched.should_render(t0 + Duration::from_millis(50), true));
    }

    #[test]
    fn event_sets_debounce_deadline() {
        let t0 = Instant::now();
        let mut sched = RenderScheduler::new(t0);

        let t1 = t0 + Duration::from_millis(5);
        sched.schedule_render(t1);

        // Not due before deadline.
        assert!(!sched.should_render(t1, false));
        assert!(!sched.should_render(t1 + Duration::from_millis(10), false));

        // Due at deadline.
        assert!(sched.should_render(t1 + DEBOUNCE_PERIOD, false));
    }

    #[test]
    fn second_event_does_not_push_deadline() {
        let t0 = Instant::now();
        let mut sched = RenderScheduler::new(t0);

        let t1 = t0 + Duration::from_millis(5);
        sched.schedule_render(t1);
        let expected_deadline = t1 + DEBOUNCE_PERIOD;

        // Second event arrives 8ms later — deadline should not move.
        let t2 = t1 + Duration::from_millis(8);
        sched.schedule_render(t2);

        assert_eq!(sched.next_render_at(false), Some(expected_deadline));

        // Still not due before the original deadline.
        assert!(!sched.should_render(t2 + Duration::from_millis(5), false));
        // Due at the original deadline.
        assert!(sched.should_render(expected_deadline, false));
    }

    #[test]
    fn record_render_clears_deadline() {
        let t0 = Instant::now();
        let mut sched = RenderScheduler::new(t0);

        let t1 = t0 + Duration::from_millis(5);
        sched.schedule_render(t1);

        let t2 = t1 + DEBOUNCE_PERIOD;
        assert!(sched.should_render(t2, false));

        sched.record_render(t2);

        // After recording, deadline is cleared; next render is idle-based.
        assert!(!sched.should_render(t2, false));
        assert_eq!(sched.next_render_at(false), None);
        assert_eq!(sched.next_render_at(true), Some(t2 + IDLE_RENDER_PERIOD));
    }

    #[test]
    fn next_render_at_returns_deadline_when_set() {
        let t0 = Instant::now();
        let mut sched = RenderScheduler::new(t0);

        // No next render unless the driver opts into idle animation.
        assert_eq!(sched.next_render_at(false), None);
        assert_eq!(sched.next_render_at(true), Some(t0 + IDLE_RENDER_PERIOD));

        let t1 = t0 + Duration::from_millis(5);
        sched.schedule_render(t1);

        // Event mode: next render is at the anchored deadline.
        assert_eq!(sched.next_render_at(false), Some(t1 + DEBOUNCE_PERIOD));
    }

    #[test]
    fn full_cycle_event_render_idle() {
        let t0 = Instant::now();
        let mut sched = RenderScheduler::new(t0);

        // 1. Event arrives.
        let t1 = t0 + Duration::from_millis(10);
        sched.schedule_render(t1);

        // 2. Render fires at deadline.
        let t2 = t1 + DEBOUNCE_PERIOD;
        assert!(sched.should_render(t2, false));
        sched.record_render(t2);

        // 3. Quiet period — no events — idle render fires.
        let t3 = t2 + IDLE_RENDER_PERIOD;
        assert!(sched.should_render(t3, true));
        sched.record_render(t3);

        // 4. Another event starts a fresh debounce.
        let t4 = t3 + Duration::from_millis(5);
        sched.schedule_render(t4);
        assert_eq!(sched.next_render_at(false), Some(t4 + DEBOUNCE_PERIOD));
    }

    #[test]
    fn immediate_render_is_due_right_away() {
        let t0 = Instant::now();
        let mut sched = RenderScheduler::new(t0);

        sched.schedule_render_now(t0);

        assert!(sched.should_render(t0, false));
        assert_eq!(
            sched.time_until_next_render(t0, false),
            Some(Duration::ZERO)
        );
    }
}

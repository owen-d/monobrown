//! Bounded FIFO queue of key events.
//!
//! Bridges input sources (keyboard, predefined scenarios) and input
//! consumers (widgets, modal resolution). Optionally records all events
//! that pass through for replay or assertion purposes.

use std::collections::VecDeque;

use crossterm::event::KeyEvent;

/// A bounded FIFO queue of key events.
///
/// Fed by input sources (keyboard, predefined scenarios). Consumed by
/// widgets or modal resolution. Optionally records all events that pass
/// through.
pub struct EventQueue {
    queue: VecDeque<KeyEvent>,
    capacity: usize,
    recording: Option<Vec<KeyEvent>>,
}

impl EventQueue {
    /// Create a new queue with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "EventQueue capacity must be positive");
        Self {
            queue: VecDeque::with_capacity(capacity),
            capacity,
            recording: None,
        }
    }

    /// Enqueue a single key event.
    ///
    /// # Panics
    ///
    /// Panics if the queue is at capacity (programmer error).
    pub fn push(&mut self, event: KeyEvent) {
        assert!(
            self.queue.len() < self.capacity,
            "EventQueue overflow: capacity {}",
            self.capacity,
        );
        self.queue.push_back(event);
    }

    /// Enqueue a slice of key events.
    ///
    /// # Panics
    ///
    /// Panics if the queue would exceed capacity.
    pub fn push_all(&mut self, events: &[KeyEvent]) {
        assert!(
            self.queue.len() + events.len() <= self.capacity,
            "EventQueue overflow: {} + {} > {}",
            self.queue.len(),
            events.len(),
            self.capacity,
        );
        self.queue.extend(events);
    }

    /// Dequeue the next event.
    ///
    /// If recording is active, appends the event to the recording buffer.
    pub fn pop(&mut self) -> Option<KeyEvent> {
        let event = self.queue.pop_front()?;
        if let Some(ref mut rec) = self.recording {
            rec.push(event);
        }
        Some(event)
    }

    /// Peek at the next event without consuming it.
    pub fn peek(&self) -> Option<&KeyEvent> {
        self.queue.front()
    }

    /// Number of pending events.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Start recording. All subsequent `pop` calls are captured.
    pub fn start_recording(&mut self) {
        self.recording = Some(Vec::new());
    }

    /// Stop recording and return the captured events.
    ///
    /// Returns `None` if recording was not active.
    pub fn stop_recording(&mut self) -> Option<Vec<KeyEvent>> {
        self.recording.take()
    }

    /// Whether recording is active.
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Drain all pending events, returning them as a `Vec`.
    ///
    /// If recording is active, all drained events are captured.
    pub fn drain(&mut self) -> Vec<KeyEvent> {
        let events: Vec<KeyEvent> = self.queue.drain(..).collect();
        if let Some(ref mut rec) = self.recording {
            rec.extend(&events);
        }
        events
    }

    /// Clear all pending events without recording them.
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    /// Helper: construct a `KeyEvent` with no modifiers.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn push_pop_fifo_order() {
        let mut q = EventQueue::new(8);

        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));
        q.push(key(KeyCode::Char('c')));

        assert_eq!(q.len(), 3);

        // Events come out in FIFO order.
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('a'));
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('b'));
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('c'));
        assert!(q.pop().is_none());
        assert!(q.is_empty());
    }

    #[test]
    #[should_panic(expected = "EventQueue overflow")]
    fn capacity_panic() {
        let mut q = EventQueue::new(2);
        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));
        // Third push exceeds capacity.
        q.push(key(KeyCode::Char('c')));
    }

    #[test]
    fn push_all_batch_enqueue() {
        let mut q = EventQueue::new(8);
        let events = [
            key(KeyCode::Char('x')),
            key(KeyCode::Char('y')),
            key(KeyCode::Char('z')),
        ];

        q.push_all(&events);

        assert_eq!(q.len(), 3);
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('x'));
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('y'));
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('z'));
    }

    #[test]
    fn peek_returns_front_without_consuming() {
        let mut q = EventQueue::new(8);
        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));

        // Peek returns front.
        assert_eq!(q.peek().unwrap().code, KeyCode::Char('a'));
        // Queue is unchanged.
        assert_eq!(q.len(), 2);
        // Pop still returns the same front.
        assert_eq!(q.pop().unwrap().code, KeyCode::Char('a'));
    }

    #[test]
    fn recording_captures_popped_events() {
        let mut q = EventQueue::new(8);
        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));
        q.push(key(KeyCode::Char('c')));

        q.start_recording();
        assert!(q.is_recording());

        // Pop two events while recording.
        q.pop();
        q.pop();

        let recorded = q.stop_recording().unwrap();
        assert!(!q.is_recording());
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].code, KeyCode::Char('a'));
        assert_eq!(recorded[1].code, KeyCode::Char('b'));
    }

    #[test]
    fn recording_off_does_not_capture() {
        let mut q = EventQueue::new(8);
        q.push(key(KeyCode::Char('a')));

        // Pop without recording active.
        q.pop();

        // stop_recording returns None when not active.
        assert!(q.stop_recording().is_none());
    }

    #[test]
    fn drain_returns_all_and_empties_queue() {
        let mut q = EventQueue::new(8);
        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));

        let drained = q.drain();

        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].code, KeyCode::Char('a'));
        assert_eq!(drained[1].code, KeyCode::Char('b'));
        assert!(q.is_empty());
    }

    #[test]
    fn drain_with_recording_captures_drained_events() {
        let mut q = EventQueue::new(8);
        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));

        q.start_recording();
        let _drained = q.drain();

        let recorded = q.stop_recording().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].code, KeyCode::Char('a'));
        assert_eq!(recorded[1].code, KeyCode::Char('b'));
    }

    #[test]
    fn clear_does_not_record() {
        let mut q = EventQueue::new(8);
        q.push(key(KeyCode::Char('a')));
        q.push(key(KeyCode::Char('b')));

        q.start_recording();
        q.clear();

        assert!(q.is_empty());
        // Nothing was recorded because clear bypasses recording.
        let recorded = q.stop_recording().unwrap();
        assert!(recorded.is_empty());
    }
}

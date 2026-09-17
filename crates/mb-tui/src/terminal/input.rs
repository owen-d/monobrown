//! Terminal input demultiplexing for capability probes and keyboard input.
//!
//! Terminal replies and keyboard input share one byte stream. [`InputDemux`]
//! removes terminal capability replies while decoding all other bytes into
//! crossterm events. It is deliberately independent of file descriptors so
//! its framing behavior can be tested without a tty.

use std::collections::VecDeque;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use termwiz::input::{InputEvent, InputParser, KeyCode as TermKeyCode, Modifiers};

const MAX_CONTROL_SEQUENCE: usize = 128;

#[cfg(unix)]
const READ_BUFFER_SIZE: usize = 256;

/// Input collected while a terminal capability probe is in flight.
#[derive(Debug)]
pub(crate) struct InputDemux {
    parser: InputParser,
    candidate: Vec<u8>,
    events: VecDeque<Event>,
    response: Option<Vec<u8>>,
    device_attributes: bool,
}

/// Sole owner of the tty input stream for an interactive session.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct InputReader {
    tty: std::fs::File,
    demux: InputDemux,
    buffer: [u8; READ_BUFFER_SIZE],
}

#[cfg(unix)]
impl InputReader {
    pub(crate) fn open() -> io::Result<Self> {
        Ok(Self {
            tty: OpenOptions::new().read(true).write(true).open("/dev/tty")?,
            demux: InputDemux::new(),
            buffer: [0; READ_BUFFER_SIZE],
        })
    }

    pub(crate) fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.tty.write_all(bytes)?;
        self.tty.flush()
    }

    pub(crate) fn response(&self) -> Option<&[u8]> {
        self.demux.response()
    }

    pub(crate) fn device_attributes(&self) -> bool {
        self.demux.device_attributes()
    }

    pub(crate) fn poll_event(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.demux.has_events() {
            return Ok(true);
        }
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            let mut poll_fd = libc::pollfd {
                fd: self.tty.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `poll_fd` points to one valid tty descriptor and the
            // timeout is bounded. No memory is retained by libc after call.
            let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
            if ready < 0 {
                return Err(io::Error::last_os_error());
            }
            if ready == 0 || (poll_fd.revents & libc::POLLIN) == 0 {
                return Ok(false);
            }
            let read = self.tty.read(&mut self.buffer)?;
            if read == 0 {
                return Ok(false);
            }
            self.demux.feed(&self.buffer[..read]);
            if self.demux.has_events() {
                return Ok(true);
            }
        }
    }

    pub(crate) fn read_event(&mut self) -> io::Result<Event> {
        loop {
            if let Some(event) = self.demux.pop_event() {
                return Ok(event);
            }
            let read = self.tty.read(&mut self.buffer)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "terminal input reached EOF",
                ));
            }
            self.demux.feed(&self.buffer[..read]);
        }
    }

    pub(crate) fn poll_read(&mut self, timeout: Duration) -> io::Result<bool> {
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut poll_fd = libc::pollfd {
            fd: self.tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll_fd` points to one valid tty descriptor and the
        // timeout is bounded. No memory is retained by libc after call.
        let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 || (poll_fd.revents & libc::POLLIN) == 0 {
            return Ok(false);
        }
        let read = self.tty.read(&mut self.buffer)?;
        if read == 0 {
            return Ok(false);
        }
        self.demux.feed(&self.buffer[..read]);
        Ok(true)
    }
}

impl Default for InputDemux {
    fn default() -> Self {
        Self {
            parser: InputParser::new(),
            candidate: Vec::new(),
            events: VecDeque::new(),
            response: None,
            device_attributes: false,
        }
    }
}

impl InputDemux {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed bytes read from the tty into the probe/input demultiplexer.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.feed_byte(byte);
        }
    }

    /// Flush a partial keyboard sequence at the end of a read batch.
    #[cfg(test)]
    pub(crate) fn finish(&mut self) {
        // Never replay an incomplete capability response. termwiz quite
        // reasonably ignores the control prefix but can expose its payload
        // as ordinary text, which then appears as phantom keyboard input.
        if self.collecting_probe_response() || self.collecting_device_attributes() {
            self.candidate.clear();
        } else {
            self.flush_candidate();
        }
        let mut events = Vec::new();
        self.parser.parse(&[], |event| events.push(event), false);
        for event in events {
            self.push_event(event);
        }
    }

    pub(crate) fn response(&self) -> Option<&[u8]> {
        self.response.as_deref()
    }

    pub(crate) fn device_attributes(&self) -> bool {
        self.device_attributes
    }

    /// Whether an OSC 11 response has started but not terminated.
    #[cfg(test)]
    pub(crate) fn collecting_probe_response(&self) -> bool {
        self.response.is_none() && self.candidate.starts_with(b"\x1b]11;")
    }

    #[cfg(test)]
    fn collecting_device_attributes(&self) -> bool {
        self.candidate.starts_with(b"\x1b[") && !self.device_attributes
    }

    #[cfg(test)]
    pub(crate) fn into_events(self) -> Vec<Event> {
        self.events.into_iter().collect()
    }

    pub(crate) fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub(crate) fn pop_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    fn feed_byte(&mut self, byte: u8) {
        if self.candidate.is_empty() {
            if byte == 0x1b {
                self.candidate.push(byte);
            } else {
                self.feed_keyboard(&[byte]);
            }
            return;
        }

        self.candidate.push(byte);

        if self.candidate == [0x1b, b']'] || self.candidate == [0x1b, b'['] {
            return;
        }

        if self.candidate.len() == 2 {
            self.flush_candidate();
            return;
        }

        if self.candidate.starts_with(b"\x1b]11;") {
            let len = self.candidate.len();
            if byte == 0x07 || (len >= 2 && self.candidate[len - 2..] == [0x1b, b'\\']) {
                self.response = Some(std::mem::take(&mut self.candidate));
            } else if len > MAX_CONTROL_SEQUENCE {
                self.candidate.clear();
            }
            return;
        }

        if self.candidate.starts_with(b"\x1b[") {
            if byte == b'c'
                && self.candidate.len() >= 3
                && (self.candidate[2] == b'?' || self.candidate[2].is_ascii_digit())
            {
                self.device_attributes = true;
                self.candidate.clear();
            } else if self.candidate.len() > MAX_CONTROL_SEQUENCE {
                self.flush_candidate();
            }
            return;
        }

        // This was an OSC sequence, but not the background-color reply. Keep
        // collecting until its terminator so it can be replayed as input.
        let len = self.candidate.len();
        if byte == 0x07 || (len >= 2 && self.candidate[len - 2..] == [0x1b, b'\\']) {
            self.flush_candidate();
        } else if len > MAX_CONTROL_SEQUENCE {
            self.flush_candidate();
        }
    }

    fn flush_candidate(&mut self) {
        if self.candidate.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.candidate);
        self.feed_keyboard(&bytes);
    }

    fn feed_keyboard(&mut self, bytes: &[u8]) {
        let mut events = Vec::new();
        self.parser.parse(bytes, |event| events.push(event), true);
        for event in events {
            self.push_event(event);
        }
    }

    fn push_event(&mut self, event: InputEvent) {
        if let Some(event) = to_crossterm(event) {
            self.events.push_back(event);
        }
    }
}

fn to_crossterm(event: InputEvent) -> Option<Event> {
    match event {
        InputEvent::Key(key) => Some(Event::Key(KeyEvent {
            code: key_code(key.key)?,
            modifiers: modifiers(key.modifiers),
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })),
        InputEvent::Resized { cols, rows } => {
            Some(Event::Resize(cols.try_into().ok()?, rows.try_into().ok()?))
        }
        InputEvent::Mouse(mouse) => Some(Event::Mouse(MouseEvent {
            kind: mouse_kind(mouse.mouse_buttons),
            column: mouse.x,
            row: mouse.y,
            modifiers: modifiers(mouse.modifiers),
        })),
        InputEvent::Paste(text) => Some(Event::Paste(text)),
        InputEvent::PixelMouse(_) | InputEvent::Wake => None,
    }
}

fn key_code(code: TermKeyCode) -> Option<KeyCode> {
    Some(match code {
        TermKeyCode::Char(value) => KeyCode::Char(value),
        TermKeyCode::Backspace => KeyCode::Backspace,
        TermKeyCode::Tab => KeyCode::Tab,
        TermKeyCode::Enter => KeyCode::Enter,
        TermKeyCode::Escape => KeyCode::Esc,
        TermKeyCode::PageUp => KeyCode::PageUp,
        TermKeyCode::PageDown => KeyCode::PageDown,
        TermKeyCode::End => KeyCode::End,
        TermKeyCode::Home => KeyCode::Home,
        TermKeyCode::LeftArrow => KeyCode::Left,
        TermKeyCode::RightArrow => KeyCode::Right,
        TermKeyCode::UpArrow => KeyCode::Up,
        TermKeyCode::DownArrow => KeyCode::Down,
        TermKeyCode::Insert => KeyCode::Insert,
        TermKeyCode::Delete => KeyCode::Delete,
        TermKeyCode::Function(number) => KeyCode::F(number),
        _ => return None,
    })
}

fn modifiers(value: Modifiers) -> KeyModifiers {
    let mut result = KeyModifiers::NONE;
    if value.contains(Modifiers::SHIFT) {
        result |= KeyModifiers::SHIFT;
    }
    if value.contains(Modifiers::CTRL) {
        result |= KeyModifiers::CONTROL;
    }
    if value.contains(Modifiers::ALT) {
        result |= KeyModifiers::ALT;
    }
    result
}

fn mouse_kind(buttons: termwiz::input::MouseButtons) -> MouseEventKind {
    if buttons.contains(termwiz::input::MouseButtons::VERT_WHEEL) {
        return if buttons.contains(termwiz::input::MouseButtons::WHEEL_POSITIVE) {
            MouseEventKind::ScrollUp
        } else {
            MouseEventKind::ScrollDown
        };
    }
    if buttons.contains(termwiz::input::MouseButtons::HORZ_WHEEL) {
        return if buttons.contains(termwiz::input::MouseButtons::WHEEL_POSITIVE) {
            MouseEventKind::ScrollRight
        } else {
            MouseEventKind::ScrollLeft
        };
    }
    if buttons.contains(termwiz::input::MouseButtons::LEFT) {
        MouseEventKind::Down(MouseButton::Left)
    } else if buttons.contains(termwiz::input::MouseButtons::RIGHT) {
        MouseEventKind::Down(MouseButton::Right)
    } else {
        MouseEventKind::Down(MouseButton::Middle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_osc_reply_and_preserves_keyboard_input() {
        let mut demux = InputDemux::new();
        demux.feed(b"a\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[?62;1;2cb");
        demux.finish();

        assert_eq!(
            demux.response(),
            Some(&b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\"[..])
        );
        assert!(demux.device_attributes());
        let events = demux.into_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            Event::Key(KeyEvent {
                code: KeyCode::Char('a'),
                ..
            })
        ));
        assert!(matches!(
            events[1],
            Event::Key(KeyEvent {
                code: KeyCode::Char('b'),
                ..
            })
        ));
    }

    #[test]
    fn leaves_non_probe_osc_sequences_as_input() {
        let mut demux = InputDemux::new();
        demux.feed(b"\x1b]0;title\x07");
        demux.finish();
        assert!(demux.response().is_none());
    }

    #[test]
    fn recognizes_a_reply_split_across_reads() {
        let mut demux = InputDemux::new();
        demux.feed(b"\x1b]11;rgb:ff/");
        assert!(demux.response().is_none());
        demux.feed(b"ff/ff\x1b\\");
        demux.finish();
        assert_eq!(demux.response(), Some(&b"\x1b]11;rgb:ff/ff/ff\x1b\\"[..]));
    }

    #[test]
    fn drops_incomplete_probe_payload_at_finish() {
        let mut demux = InputDemux::new();
        demux.feed(b"\x1b]11;rgb:2c2c/3434");
        assert!(demux.collecting_probe_response());
        demux.finish();
        assert!(demux.into_events().is_empty());
    }

    #[test]
    fn consumes_a_late_device_attributes_reply_without_leaking_payload() {
        let mut demux = InputDemux::new();
        demux.feed(b"\x1b[?62;1;2c");
        demux.feed(b"\x1b]11;rgb:2c2c/3434/3c3c\x1b\\");
        demux.finish();
        assert!(demux.device_attributes());
        assert!(demux.response().is_some());
        assert!(demux.into_events().is_empty());
    }
}

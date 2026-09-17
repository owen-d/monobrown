//! Terminal input through Termina's typed VT parser.
//!
//! Termina owns the byte-level escape-sequence parser and produces typed key,
//! mouse, resize, paste, OSC, and CSI events. This module owns only the tty
//! read loop, event queue, and adapter to the crossterm event type used by
//! existing consumers. C1 string termination is normalized at the byte
//! boundary because the current Termina parser accepts BEL and 7-bit ST only.

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal;
use termina::Parser;
use termina::escape::{
    csi::{Csi, Device},
    osc::{ColorOrQuery, DynamicColorNumber, Osc},
};
use termina::event::{
    Event as TerminaEvent, KeyCode as TerminaKeyCode, KeyEventKind as TerminaKeyEventKind,
    Modifiers as TerminaModifiers, MouseButton as TerminaMouseButton,
    MouseEventKind as TerminaMouseEventKind,
};

const READ_BUFFER_SIZE: usize = 256;

/// Sole owner of the terminal input stream for an interactive session.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct InputReader {
    tty: std::fs::File,
    parser: Parser,
    events: VecDeque<TerminaEvent>,
    buffer: [u8; READ_BUFFER_SIZE],
}

#[cfg(unix)]
impl InputReader {
    pub(crate) fn open() -> io::Result<Self> {
        let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        terminal::enable_raw_mode()?;
        Ok(Self {
            tty,
            parser: Parser::default(),
            events: VecDeque::new(),
            buffer: [0; READ_BUFFER_SIZE],
        })
    }

    pub(crate) fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.tty.write_all(bytes)?;
        self.tty.flush()
    }

    pub(crate) fn poll_event(&mut self, timeout: Duration) -> io::Result<bool> {
        self.poll_matching(timeout, is_user_event)
    }

    pub(crate) fn read_event(&mut self) -> io::Result<Event> {
        loop {
            if let Some(event) = self
                .take_matching(is_user_event)
                .and_then(|event| to_crossterm(&event))
            {
                return Ok(event);
            }
            self.read_from_tty(None)?;
        }
    }

    pub(crate) fn poll_probe_event(&mut self, timeout: Duration) -> io::Result<bool> {
        self.poll_matching(timeout, is_probe_event)
    }

    pub(crate) fn read_probe_event(&mut self) -> io::Result<TerminaEvent> {
        loop {
            if let Some(event) = self.take_matching(is_probe_event) {
                return Ok(event);
            }
            self.read_from_tty(None)?;
        }
    }

    fn poll_matching(
        &mut self,
        timeout: Duration,
        predicate: fn(&TerminaEvent) -> bool,
    ) -> io::Result<bool> {
        if self.events.iter().any(predicate) {
            return Ok(true);
        }

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            self.read_from_tty(Some(remaining))?;
            if self.events.iter().any(predicate) {
                return Ok(true);
            }
        }
    }

    fn read_from_tty(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        if !wait_for_input(self.tty.as_raw_fd(), timeout)? {
            return Ok(());
        }
        let read = self.tty.read(&mut self.buffer)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "terminal input reached EOF",
            ));
        }
        self.feed(&self.buffer[..read].to_vec());
        Ok(())
    }

    fn feed(&mut self, bytes: &[u8]) {
        // Termina currently accepts BEL and 7-bit ST for OSC strings. C1 ST
        // is the same protocol terminator and must not strand the parser.
        let mut normalized = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if byte == 0x9c {
                normalized.extend_from_slice(b"\x1b\\");
            } else {
                normalized.push(byte);
            }
        }
        self.parser.parse(&normalized, true);
        while let Some(event) = self.parser.pop() {
            self.events.push_back(event);
        }
    }

    fn take_matching(&mut self, predicate: fn(&TerminaEvent) -> bool) -> Option<TerminaEvent> {
        let index = self.events.iter().position(predicate)?;
        self.events.remove(index)
    }
}

#[cfg(unix)]
fn wait_for_input(fd: i32, timeout: Option<Duration>) -> io::Result<bool> {
    let mut readfds = unsafe { std::mem::zeroed::<libc::fd_set>() };
    // SAFETY: `readfds` is local storage and `fd` is an open tty descriptor.
    unsafe {
        libc::FD_SET(fd, &mut readfds);
    }

    let mut time = timeout.map(|duration| libc::timeval {
        tv_sec: duration.as_secs().min(libc::time_t::MAX as u64) as libc::time_t,
        tv_usec: duration.subsec_micros() as libc::suseconds_t,
    });
    let time_ptr = time
        .as_mut()
        .map_or(std::ptr::null_mut(), |time| time as *mut libc::timeval);
    // SAFETY: all pointers refer to local storage for the duration of the call;
    // libc retains none of them after `select` returns.
    let ready = unsafe {
        libc::select(
            fd + 1,
            &mut readfds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            time_ptr,
        )
    };
    if ready < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ready > 0)
}

#[cfg(unix)]
impl Drop for InputReader {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn is_user_event(event: &TerminaEvent) -> bool {
    to_crossterm(event).is_some()
}

fn is_probe_event(event: &TerminaEvent) -> bool {
    matches!(
        event,
        TerminaEvent::Osc(Osc::ChangeDynamicColors(_, _))
            | TerminaEvent::Csi(Csi::Device(Device::DeviceAttributes(())))
    )
}

fn to_crossterm(event: &TerminaEvent) -> Option<Event> {
    match event {
        TerminaEvent::Key(key) if key.kind == TerminaKeyEventKind::Press => {
            Some(Event::Key(KeyEvent {
                code: key_code(key.code)?,
                modifiers: modifiers(key.modifiers),
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
        }
        TerminaEvent::Mouse(mouse) => Some(Event::Mouse(MouseEvent {
            kind: mouse_kind(mouse.kind),
            column: mouse.column,
            row: mouse.row,
            modifiers: modifiers(mouse.modifiers),
        })),
        TerminaEvent::WindowResized(size) => Some(Event::Resize(size.cols, size.rows)),
        TerminaEvent::Paste(text) => Some(Event::Paste(text.clone())),
        _ => None,
    }
}

fn key_code(code: TerminaKeyCode) -> Option<KeyCode> {
    Some(match code {
        TerminaKeyCode::Char(value) => KeyCode::Char(value),
        TerminaKeyCode::Backspace => KeyCode::Backspace,
        TerminaKeyCode::Tab => KeyCode::Tab,
        TerminaKeyCode::Enter => KeyCode::Enter,
        TerminaKeyCode::Escape => KeyCode::Esc,
        TerminaKeyCode::BackTab => KeyCode::BackTab,
        TerminaKeyCode::PageUp => KeyCode::PageUp,
        TerminaKeyCode::PageDown => KeyCode::PageDown,
        TerminaKeyCode::End => KeyCode::End,
        TerminaKeyCode::Home => KeyCode::Home,
        TerminaKeyCode::Left => KeyCode::Left,
        TerminaKeyCode::Right => KeyCode::Right,
        TerminaKeyCode::Up => KeyCode::Up,
        TerminaKeyCode::Down => KeyCode::Down,
        TerminaKeyCode::Insert => KeyCode::Insert,
        TerminaKeyCode::Delete => KeyCode::Delete,
        TerminaKeyCode::Function(number) => KeyCode::F(number),
        _ => return None,
    })
}

fn modifiers(value: TerminaModifiers) -> KeyModifiers {
    let mut result = KeyModifiers::NONE;
    if value.contains(TerminaModifiers::SHIFT) {
        result |= KeyModifiers::SHIFT;
    }
    if value.contains(TerminaModifiers::CONTROL) {
        result |= KeyModifiers::CONTROL;
    }
    if value.contains(TerminaModifiers::ALT) {
        result |= KeyModifiers::ALT;
    }
    if value.contains(TerminaModifiers::SUPER) {
        result |= KeyModifiers::SUPER;
    }
    result
}

fn mouse_kind(kind: TerminaMouseEventKind) -> MouseEventKind {
    match kind {
        TerminaMouseEventKind::Down(button) => MouseEventKind::Down(mouse_button(button)),
        TerminaMouseEventKind::Up(button) => MouseEventKind::Up(mouse_button(button)),
        TerminaMouseEventKind::Drag(button) => MouseEventKind::Drag(mouse_button(button)),
        TerminaMouseEventKind::Moved => MouseEventKind::Moved,
        TerminaMouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
        TerminaMouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
        TerminaMouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
        TerminaMouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
    }
}

fn mouse_button(button: TerminaMouseButton) -> MouseButton {
    match button {
        TerminaMouseButton::Left => MouseButton::Left,
        TerminaMouseButton::Right => MouseButton::Right,
        TerminaMouseButton::Middle => MouseButton::Middle,
    }
}

/// Format the typed palette and device-attribute queries as one terminal write.
#[cfg(unix)]
pub(crate) fn probe_query() -> Vec<u8> {
    let palette = Osc::ChangeDynamicColors(
        DynamicColorNumber::TextBackgroundColor,
        vec![ColorOrQuery::Query],
    );
    let attributes = Csi::Device(Device::RequestPrimaryDeviceAttributes);
    format!("{palette}{attributes}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use termina::Parser;

    #[test]
    fn termina_parses_split_palette_and_key_events() {
        let mut parser = Parser::default();
        let bytes = b"\x1b]11;rgb:2c2c/3434/3c3c\x1b\\q";
        for byte in bytes {
            parser.parse(&[*byte], true);
        }

        let events = std::iter::from_fn(|| parser.pop()).collect::<Vec<_>>();
        assert!(matches!(
            events.first(),
            Some(TerminaEvent::Osc(Osc::ChangeDynamicColors(
                DynamicColorNumber::TextBackgroundColor,
                colors,
            ))) if matches!(colors.as_slice(), [ColorOrQuery::Color(color)] if (color.red, color.green, color.blue) == (0x2c, 0x34, 0x3c))
        ));
        assert!(matches!(
            events.get(1),
            Some(TerminaEvent::Key(key)) if key.code == TerminaKeyCode::Char('q')
        ));
    }

    #[test]
    fn termina_preserves_key_before_probe_responses() {
        let mut parser = Parser::default();
        let bytes = b"q\x1b]11;rgb:2c2c/3434/3c3c\x1b\\\x1b[?62;1;2c";
        parser.parse(bytes, true);
        let events = std::iter::from_fn(|| parser.pop()).collect::<Vec<_>>();
        assert!(
            matches!(events.first(), Some(TerminaEvent::Key(key)) if key.code == TerminaKeyCode::Char('q'))
        );
        assert!(matches!(events.get(1), Some(TerminaEvent::Osc(_))));
        assert!(matches!(events.get(2), Some(TerminaEvent::Csi(_))));
    }

    #[test]
    fn c1_string_terminator_is_normalized_before_termina() {
        let mut parser = Parser::default();
        let bytes = b"\x1b]11;rgb:2c2c/3434/3c3c\x9cq";
        let mut normalized = Vec::new();
        for &byte in bytes {
            if byte == 0x9c {
                normalized.extend_from_slice(b"\x1b\\");
            } else {
                normalized.push(byte);
            }
        }
        parser.parse(&normalized, true);
        let events = std::iter::from_fn(|| parser.pop()).collect::<Vec<_>>();
        assert!(
            matches!(events.get(1), Some(TerminaEvent::Key(key)) if key.code == TerminaKeyCode::Char('q'))
        );
    }

    #[test]
    fn query_uses_typed_termina_sequences() {
        assert_eq!(probe_query(), b"\x1b]11;?\x1b\\\x1b[c".to_vec());
    }
}

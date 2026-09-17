//! Shared ownership of an interactive terminal session.
//!
//! A [`TuiSession`] establishes the terminal boundary before a consumer
//! constructs theme-dependent state, performs the optional palette probe while
//! it owns terminal input, then restores the terminal on every ordinary return
//! and unwind path.

use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::cursor::Show;
use crossterm::event::{self, Event};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod input;
#[cfg(unix)]
use input::InputReader;
/// Optional terminal-owned palette probing.
pub mod palette;

/// A live terminal session shared by interactive Monobrown consumers.
///
/// Construction enters raw mode and the alternate screen, clears the initial
/// frame, probes the terminal palette, and returns a session whose terminal
/// and input can be borrowed by the caller. Dropping the session best-effort
/// restores the terminal even when the consumer returns an error or unwinds.
/// Only one top-level session may be active at a time; nested or concurrent
/// sessions would manipulate the same process terminal state.
pub struct TuiSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    #[cfg(unix)]
    input: Option<InputReader>,
    _restore: TerminalRestore,
}

impl TuiSession {
    /// Run a consumer inside an owned terminal session.
    ///
    /// The initializer runs after terminal setup and palette detection so
    /// theme-dependent state observes the active terminal. Consumers should
    /// use [`Self::poll_event`] and [`Self::read_event`] rather than reading
    /// crossterm directly. Both callbacks share the caller's error type;
    /// terminal errors are converted through `From<io::Error>`.
    pub fn run<State, Error, Init, Body>(init: Init, body: Body) -> Result<(), Error>
    where
        Error: From<io::Error>,
        Init: FnOnce() -> Result<State, Error>,
        Body: FnOnce(&mut Self, State) -> Result<(), Error>,
    {
        let mut session = Self::enter().map_err(Error::from)?;
        let state = init()?;
        body(&mut session, state)
    }

    /// Enter raw mode and the alternate screen, returning owned terminal state.
    fn enter() -> io::Result<Self> {
        if !io::stdout().is_terminal() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "interactive TUI requires terminal stdout",
            ));
        }

        terminal::enable_raw_mode()?;
        let restore = TerminalRestore;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        let probe = palette::probe_background();

        let session = Self {
            terminal,
            #[cfg(unix)]
            input: probe.input,
            _restore: restore,
        };

        if let Some(palette) = probe.palette {
            crate::theme::palette::set(palette);
        }

        Ok(session)
    }

    /// Borrow the Ratatui terminal for consumer rendering and event handling.
    pub fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }

    /// Poll for the next input event through the session-owned byte parser.
    pub fn poll_event(&mut self, timeout: Duration) -> io::Result<bool> {
        #[cfg(unix)]
        if let Some(input) = self.input.as_mut() {
            return input.poll_event(timeout);
        }
        event::poll(timeout)
    }

    /// Read the next input event through the session-owned byte parser.
    pub fn read_event(&mut self) -> io::Result<Event> {
        #[cfg(unix)]
        if let Some(input) = self.input.as_mut() {
            return input.read_event();
        }
        event::read()
    }
}

/// Restore terminal state without allowing cleanup errors to mask the owner.
struct TerminalRestore;

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = stdout.execute(LeaveAlternateScreen);
        let _ = stdout.execute(Show);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Terminal validation must happen before a consumer initializer can run.
    #[test]
    fn run_rejects_non_terminal_before_initializing_state() {
        if io::stdout().is_terminal() {
            return;
        }

        let initialized = Cell::new(false);
        let result = TuiSession::run(
            || {
                initialized.set(true);
                Ok::<_, io::Error>(())
            },
            |_, _| Ok(()),
        );

        assert!(result.is_err());
        assert!(!initialized.get());
    }
}

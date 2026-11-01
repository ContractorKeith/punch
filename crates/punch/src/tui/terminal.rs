//! Terminal ownership: restore on normal exit, errors, editor suspension, and panic.
use super::editor;
use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use punch_core::error::PunchError;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{self, IsTerminal, Stdout};

pub type Screen = Terminal<CrosstermBackend<Stdout>>;
pub fn restore() {
    restore_with(&mut io::stdout(), disable_raw_mode);
}
pub(super) fn restore_with(writer: &mut impl io::Write, disable: impl FnOnce() -> io::Result<()>) {
    let _ = disable();
    let _ = execute!(writer, Show, LeaveAlternateScreen);
}
pub struct Session;
impl Session {
    pub fn start() -> Result<(Self, Screen), PunchError> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(PunchError::usage(
                "the board requires a terminal; use list --json for agents",
            ));
        }
        install_panic_hook(restore);
        // Construct the guard before the first terminal mutation, including partial setup failures.
        let session = Self;
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.clear()?;
        Ok((session, terminal))
    }
    pub fn edit(&self, terminal: &mut Screen, notes: &str) -> Result<String, PunchError> {
        restore();
        let result = std::env::var("EDITOR")
            .map_err(|_| PunchError::usage("set $EDITOR to edit notes"))
            .and_then(|command| editor::edit_with(&command, notes));
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        terminal.clear()?;
        result
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        restore();
    }
}

// The restoration boundary is injectable so panic cleanup needs no real terminal in tests.
pub(super) fn install_panic_hook(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

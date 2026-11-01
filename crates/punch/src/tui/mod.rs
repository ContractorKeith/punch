//! Interactive board adapter. All domain access goes through core operations.
mod editor;
mod model;
mod terminal;
#[cfg(test)]
mod tests;
mod view;

use crossterm::event::{self, Event, KeyEventKind};
use punch_core::{error::PunchError, model::Actor, store::Store};
use std::time::{Duration, Instant};

pub fn run(actor: Actor) -> Result<(), PunchError> {
    let (session, mut terminal) = terminal::Session::start()?;
    let mut store = Store::open()?;
    let mut board = model::Board::load(&mut store, actor)?;
    let mut last_poll = Instant::now();
    while !board.quit {
        terminal.draw(|frame| view::draw(frame, &board))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
        {
            board.key(key, &mut store);
            if let Some(snapshot) = board.edit_request.take() {
                match session.edit(&mut terminal, &snapshot.notes) {
                    Ok(notes) => board.save_notes(&mut store, &snapshot, notes),
                    Err(error) => board.flash(error),
                }
            }
        }
        if last_poll.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = board.poll(&mut store) {
                board.flash(error);
            }
            last_poll = Instant::now();
        }
    }
    Ok(())
}

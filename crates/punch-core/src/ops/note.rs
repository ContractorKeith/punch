//! Append-only timestamped notes.
use super::{project::now, task::{bump_revision, resolve}};
use crate::{error::PunchError, model::*, store::Store};
pub fn note(
    store: &mut Store,
    actor: &Actor,
    reference: &str,
    text: &str,
) -> Result<Outcome<Task>, PunchError> {
    store.transaction(|repo| {
        let mut task = resolve(&repo.tasks()?, reference)?.clone();
        let timestamp = now()?;
        task.notes
            .push_str(&format!("\n[{timestamp} {}] {text}", actor.label()));
        bump_revision(&mut task)?;
        task.updated_at = timestamp;
        repo.save_task(&task)?;
        Ok(Outcome::new(true, task))
    })
}

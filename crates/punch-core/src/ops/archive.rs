//! Person-only archive and unarchive operations.
use super::{project::now, task::{bump_revision, resolve}};
use crate::{error::*, model::*, store::Store};
fn set_archived(
    store: &mut Store,
    actor: &Actor,
    reference: &str,
    archived: bool,
) -> Result<Outcome<Task>, PunchError> {
    store.transaction(|repo| {
        let mut task = resolve(&repo.tasks()?, reference)?.clone();
        if actor.is_agent() {
            return Err(PunchError::refusal(
                AGENT_CANNOT_ARCHIVE,
                reference,
                "agents cannot archive or unarchive",
            ));
        }
        if task.archived == archived {
            return Ok(Outcome::new(false, task));
        }
        task.archived = archived;
        bump_revision(&mut task)?;
        task.updated_at = now()?;
        repo.save_task(&task)?;
        Ok(Outcome::new(true, task))
    })
}
pub fn archive(
    store: &mut Store,
    actor: &Actor,
    reference: &str,
) -> Result<Outcome<Task>, PunchError> {
    set_archived(store, actor, reference, true)
}
pub fn unarchive(
    store: &mut Store,
    actor: &Actor,
    reference: &str,
) -> Result<Outcome<Task>, PunchError> {
    set_archived(store, actor, reference, false)
}

//! Resolve a reference and return complete task detail.
use crate::{error::{PunchError, UNKNOWN_TASK}, model::*, store::Store};
pub fn show(
    store: &mut Store,
    _actor: &Actor,
    reference: &str,
) -> Result<Outcome<Task>, PunchError> {
    store.read_transaction(|repo| {
        let id = if let Some((slug, number)) = reference.split_once('#') {
            match number.parse::<i64>() {
                Ok(number) => repo.github_task_id(slug, number)?,
                Err(_) => None,
            }
        } else {
            reference.strip_prefix('t').unwrap_or(reference).parse::<i64>().ok()
        };
        let task = match id {
            Some(id) => repo.task(id)?,
            None => None,
        }.ok_or_else(|| PunchError::refusal(UNKNOWN_TASK, reference, "unknown task"))?;
        Ok(Outcome::new(false, task))
    })
}

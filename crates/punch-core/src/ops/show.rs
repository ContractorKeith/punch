//! Resolve a reference and return complete task detail.
use super::task::resolve;
use crate::{error::PunchError, model::*, store::Store};
pub fn show(
    store: &mut Store,
    _actor: &Actor,
    reference: &str,
) -> Result<Outcome<Task>, PunchError> {
    store.transaction(|repo| {
        Ok(Outcome::new(
            false,
            resolve(&repo.tasks()?, reference)?.clone(),
        ))
    })
}

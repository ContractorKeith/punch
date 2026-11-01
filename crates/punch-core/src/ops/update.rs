//! Revision-checked updates for adapters holding a task snapshot.
use super::{Edit, archive, edit, move_task};
use crate::{error::PunchError, model::*, store::Store};

pub enum Update {
    Move(Column),
    Edit(Edit),
    Archive,
}
/// Compare the snapshot revision inside the same transaction as one existing op.
/// Only id and revision are taken from the snapshot; all fields are read afresh.
pub fn update_task(
    store: &mut Store,
    actor: &Actor,
    snapshot: &Task,
    update: Update,
) -> Result<Outcome<Task>, PunchError> {
    store.at_revision(snapshot.id, snapshot.revision, |store| match update {
        Update::Move(column) => move_task(store, actor, &snapshot.reference(), column),
        Update::Edit(args) => edit(store, actor, &snapshot.reference(), args),
        Update::Archive => archive(store, actor, &snapshot.reference()),
    })
}

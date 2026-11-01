//! Local column changes with actor and dependency checks.
use super::{project::now, task::resolve};
use crate::{error::*, model::*, store::Store};
pub fn move_task(
    store: &mut Store,
    actor: &Actor,
    reference: &str,
    column: Column,
) -> Result<Outcome<Task>, PunchError> {
    store.transaction(|repo| {
        let tasks = repo.tasks()?;
        let mut task = resolve(&tasks, reference)?.clone();
        if actor.is_agent() && column.finished() {
            return Err(PunchError::refusal(
                AGENT_CANNOT_FINISH,
                reference,
                "agents cannot finish cards",
            ));
        }
        if task.kind != Kind::Local {
            return Err(PunchError::refusal(
                COLUMN_IS_DERIVED,
                reference,
                "GitHub columns are derived",
            ));
        }
        if task.column == column {
            return Ok(Outcome::new(false, task));
        }
        if column == Column::Doing
            && task
                .after
                .iter()
                .any(|id| tasks.iter().any(|t| t.id == *id && !t.column.finished()))
        {
            return Err(PunchError::refusal(
                AFTER_NOT_DONE,
                reference,
                "dependencies are unfinished",
            ));
        }
        task.column = column;
        task.local_column = Some(column);
        task.revision += 1;
        task.updated_at = now()?;
        repo.save_task(&task)?;
        Ok(Outcome::new(true, task))
    })
}

//! Atomic field and dependency edits.
use super::{
    project::now,
    task::{bump_revision, dependencies, resolve, validate_fields},
};
use crate::{error::*, model::*, store::Store};
#[derive(Debug, Clone, Default)]
pub struct Edit {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub priority: Option<Priority>,
    /// None leaves due unchanged; Some(None) clears it.
    pub due: Option<Option<String>>,
    pub after: Option<Vec<String>>,
}
pub fn edit(
    store: &mut Store,
    actor: &Actor,
    reference: &str,
    args: Edit,
) -> Result<Outcome<Task>, PunchError> {
    super::bc::mutate_task(store, |repo| {
        validate_fields(
            args.title.as_deref(),
            args.priority,
            args.due.as_ref().and_then(|d| d.as_deref()),
        )?;
        let tasks = repo.tasks()?;
        let old = resolve(&tasks, reference)?;
        let mut task = old.clone();
        if args.title.is_some() && task.kind != Kind::Local {
            return Err(PunchError::refusal(
                GITHUB_OWNED_FIELD,
                reference,
                "GitHub owns the title",
            ));
        }
        if actor.is_agent() && (args.title.is_some() || args.notes.is_some()) {
            return Err(PunchError::refusal(
                AGENT_CANNOT_EDIT,
                reference,
                "agents may append notes but cannot replace titles or notes",
            ));
        }
        if let Some(title) = args.title {
            task.title = title;
        }
        if let Some(notes) = args.notes {
            task.notes = notes;
        }
        if let Some(priority) = args.priority {
            task.priority = priority;
        }
        if let Some(due) = args.due {
            task.due = due;
        }
        if let Some(after) = args.after {
            task.after = dependencies(&tasks, task.id, task.project_id, &after)?;
        }
        if task == *old {
            return Ok(Outcome::new(false, task));
        }
        bump_revision(&mut task)?;
        task.updated_at = now()?;
        repo.save_task(&task)?;
        if task.after != old.after {
            repo.set_after(task.id, &task.after)?;
        }
        Ok(Outcome::new(true, task))
    })
}

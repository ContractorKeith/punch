//! Idempotent local task creation.
use super::{
    project::{find_project, now},
    task::{dependencies, validate_fields},
};
use crate::{
    error::*,
    model::*,
    store::{Repository, Store},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    #[serde(alias = "p")]
    pub project: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub agent: bool,
}
#[derive(Debug, Serialize)]
pub struct Added {
    pub task: Task,
    pub existing: bool,
}

pub(super) fn add_in(repo: &Repository<'_>, args: &Add) -> Result<Outcome<Added>, PunchError> {
    validate_fields(Some(&args.title), Some(args.priority), args.due.as_deref())?;
    if args.agent && args.assignee.is_none() {
        return Err(PunchError::usage("--agent requires --assignee"));
    }
    let project = find_project(repo, &args.project)?;
    let tasks = repo.tasks()?;
    if let Some(t) = tasks.iter().find(|t| {
        t.project_id == project.id
            && t.kind == Kind::Local
            && !t.archived
            && t.title == args.title
            && t.assignee == args.assignee
    }) {
        return Ok(Outcome::new(
            false,
            Added {
                task: t.clone(),
                existing: true,
            },
        ));
    }
    let timestamp = now()?;
    let position = tasks
        .iter()
        .filter(|t| t.project_id == project.id)
        .map(|t| t.position)
        .fold(0.0, f64::max)
        + 1.0;
    let mut t = Task {
        id: 0,
        project_id: project.id,
        project: project.slug,
        kind: Kind::Local,
        gh_number: None,
        title: args.title.clone(),
        notes: args.notes.clone(),
        local_column: Some(Column::Todo),
        column: Column::Todo,
        priority: args.priority,
        due: args.due.clone(),
        assignee: args.assignee.clone(),
        assignee_kind: args.assignee.as_ref().map(|_| {
            if args.agent {
                AssigneeKind::Agent
            } else {
                AssigneeKind::Person
            }
        }),
        slot: if args.agent {
            args.assignee.clone()
        } else {
            None
        },
        position,
        archived: false,
        revision: 1,
        created_at: timestamp.clone(),
        updated_at: timestamp,
        after: vec![],
        gh_cache: None,
        basecamp_card: None,
    };
    // Validate before writing, including the next SQLite rowid, to keep failed plan items empty.
    t.id = tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    t.after = dependencies(&tasks, t.id, t.project_id, &args.after)?;
    t.id = repo.insert_task(&t)?;
    if !t.after.is_empty() {
        repo.set_after(t.id, &t.after)?;
    }
    Ok(Outcome::new(
        true,
        Added {
            task: t,
            existing: false,
        },
    ))
}
pub fn add(store: &mut Store, _actor: &Actor, args: Add) -> Result<Outcome<Added>, PunchError> {
    store.transaction(|repo| add_in(repo, &args))
}

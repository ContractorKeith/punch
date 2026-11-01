//! Dependency-aware ready queue and stable ordering.
use super::project::find_project;
use crate::{error::PunchError, model::*, store::Store};
#[derive(Debug, Default, Clone)]
pub struct Ready {
    pub project: Option<String>,
    pub limit: Option<usize>,
}
pub fn ready(
    store: &mut Store,
    _actor: &Actor,
    args: Ready,
) -> Result<Outcome<Vec<Task>>, PunchError> {
    store.transaction(|repo| {
        if let Some(p) = &args.project {
            find_project(repo, p)?;
        }
        let projects = repo.projects()?;
        let tasks = repo.tasks()?;
        let mut ready: Vec<_> = tasks
            .iter()
            .filter(|t| {
                t.column == Column::Todo
                    && !t.archived
                    && projects.iter().any(|p| p.id == t.project_id && !p.archived)
                    && args.project.as_ref().is_none_or(|p| p == &t.project)
                    && t.after
                        .iter()
                        .all(|id| tasks.iter().any(|d| d.id == *id && d.column.finished()))
            })
            .cloned()
            .collect();
        ready.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| match (&a.due, &b.due) {
                    (Some(a), Some(b)) => a.cmp(b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.position.total_cmp(&b.position))
                .then(a.id.cmp(&b.id))
        });
        if let Some(limit) = args.limit {
            ready.truncate(limit);
        }
        Ok(Outcome::new(false, ready))
    })
}

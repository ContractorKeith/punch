//! Task filtering, including archived project visibility.
use super::project::find_project;
use crate::{error::PunchError, model::*, store::Store};
#[derive(Debug, Default, Clone)]
pub struct List {
    pub project: Option<String>,
    pub column: Option<Column>,
    pub all: bool,
}
pub fn list(
    store: &mut Store,
    _actor: &Actor,
    args: List,
) -> Result<Outcome<Vec<Task>>, PunchError> {
    store.read_transaction(|repo| {
        if let Some(p) = &args.project {
            find_project(repo, p)?;
        }
        let archived: Vec<_> = repo
            .projects()?
            .into_iter()
            .filter(|p| p.archived)
            .map(|p| p.id)
            .collect();
        let tasks = repo
            .tasks()?
            .into_iter()
            .filter(|t| {
                args.project.as_ref().is_none_or(|p| p == &t.project)
                    && args.column.is_none_or(|c| c == t.column)
                    && (args.all
                        || (!t.archived
                            && !archived.contains(&t.project_id)
                            && !t.column.finished()))
            })
            .collect();
        Ok(Outcome::new(false, tasks))
    })
}

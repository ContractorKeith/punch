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
        Ok(Outcome::new(false, repo.ready(args.project.as_deref(), args.limit)?))
    })
}

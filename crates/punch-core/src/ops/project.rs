//! Project creation, listing, and archival.
use crate::{
    error::*,
    model::*,
    store::{Repository, Store},
};
use serde::Deserialize;

pub(super) fn now() -> Result<String, PunchError> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(PunchError::storage)
}
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectAdd {
    pub slug: String,
    pub name: Option<String>,
    pub github: Option<String>,
    pub scope: Option<SyncScope>,
}
pub(super) fn find_project(repo: &Repository<'_>, slug: &str) -> Result<Project, PunchError> {
    repo.projects()?
        .into_iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| PunchError::refusal(UNKNOWN_PROJECT, slug, "unknown project"))
}
pub fn project_add(
    store: &mut Store,
    actor: &Actor,
    args: ProjectAdd,
) -> Result<Outcome<Project>, PunchError> {
    store.transaction(|repo| {
        if actor.is_agent() {
            return Err(PunchError::refusal(
                AGENT_CANNOT_MANAGE_PROJECTS,
                &args.slug,
                "agents cannot manage projects",
            ));
        }
        if args.slug.is_empty()
            || args.slug.contains('#')
            || args.slug.chars().any(char::is_whitespace)
        {
            return Err(PunchError::usage(
                "project slug must be nonempty without whitespace or #",
            ));
        }
        let name = args.name.as_deref().unwrap_or(&args.slug);
        let scope = args.scope.unwrap_or(SyncScope::Assigned);
        let projects = repo.projects()?;
        if let Some(p) = projects.iter().find(|p| p.slug == args.slug) {
            if p.name == name && p.github_repo == args.github && p.sync_scope == scope {
                return Ok(Outcome::new(false, p.clone()));
            }
            return Err(PunchError::refusal(
                PROJECT_EXISTS,
                &args.slug,
                "project slug already has different fields",
            ));
        }
        if args.github.is_some() && projects.iter().any(|p| p.github_repo == args.github) {
            return Err(PunchError::refusal(
                PROJECT_EXISTS,
                &args.slug,
                "GitHub repository is already linked",
            ));
        }
        repo.insert_project(&args.slug, name, args.github.as_deref(), scope, &now()?)?;
        Ok(Outcome::new(true, find_project(repo, &args.slug)?))
    })
}
pub fn project_list(
    store: &mut Store,
    _actor: &Actor,
) -> Result<Outcome<Vec<Project>>, PunchError> {
    store.read_transaction(|repo| Ok(Outcome::new(false, repo.projects()?)))
}
pub fn project_archive(
    store: &mut Store,
    actor: &Actor,
    slug: &str,
) -> Result<Outcome<Project>, PunchError> {
    store.transaction(|repo| {
        if actor.is_agent() {
            return Err(PunchError::refusal(
                AGENT_CANNOT_MANAGE_PROJECTS,
                slug,
                "agents cannot manage projects",
            ));
        }
        let p = find_project(repo, slug)?;
        if p.archived {
            return Ok(Outcome::new(false, p));
        }
        repo.archive_project(p.id, &now()?)?;
        Ok(Outcome::new(true, find_project(repo, slug)?))
    })
}

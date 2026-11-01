//! Pull each repository atomically; report failures without losing successful projects.
use super::project::{find_project, now};
use crate::{
    error::*,
    github::{Gh, Item},
    model::*,
    store::{Repository, Store},
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct SyncResult {
    pub project: String,
    pub status: &'static str,
    pub changed: bool,
    pub cards: usize,
    pub refusals: Vec<Refusal>,
}
pub fn sync(
    store: &mut Store,
    _actor: &Actor,
    gh: &dyn Gh,
    project: Option<&str>,
) -> Result<Outcome<Vec<SyncResult>>, PunchError> {
    let projects = store.transaction(|repo| {
        if let Some(p) = project {
            find_project(repo, p)?;
        }
        Ok(repo
            .projects()?
            .into_iter()
            .filter(|p| {
                project.is_none_or(|s| p.slug == s) && !p.archived && p.github_repo.is_some()
            })
            .collect::<Vec<_>>())
    })?;
    let mut results = vec![];
    for p in projects {
        let result = store.transaction(|repo| sync_project(repo, gh, &p));
        results.push(match result {
            Ok((changed, cards)) => SyncResult {
                project: p.slug,
                status: "synced",
                changed,
                cards,
                refusals: vec![],
            },
            Err(e) => SyncResult {
                project: p.slug,
                status: "failed",
                changed: false,
                cards: 0,
                refusals: vec![e.detail()],
            },
        });
    }
    let (mirror_changed, warnings) = super::bc::reconcile(store, project)?;
    let mut outcome = Outcome::new(mirror_changed || results.iter().any(|r| r.changed), results);
    outcome.warnings = warnings;
    Ok(outcome)
}
fn sync_project(
    repo: &Repository<'_>,
    gh: &dyn Gh,
    p: &Project,
) -> Result<(bool, usize), PunchError> {
    let github = p
        .github_repo
        .as_deref()
        .ok_or_else(|| PunchError::storage("missing GitHub repo"))?;
    let existing: Vec<_> = repo
        .tasks()?
        .into_iter()
        .filter(|t| t.project_id == p.id && t.kind != Kind::Local)
        .collect();
    let mut items = gh.list(github, p.sync_scope)?;
    for task in &existing {
        let number = task
            .gh_number
            .ok_or_else(|| PunchError::storage("missing GitHub number"))?;
        // Refresh stored numbers even when outside the open/scope query.
        let fresh = gh.get(github, task.kind, number)?;
        items.retain(|i| i.kind != task.kind || i.number != number);
        items.push(fresh);
    }
    let issues: BTreeSet<_> = items
        .iter()
        .filter(|i| i.kind == Kind::Issue)
        .map(|i| i.number)
        .collect();
    let mut changed = false;
    let mut count = 0;
    let timestamp = now()?;
    for item in &items {
        let stored = existing
            .iter()
            .any(|t| t.kind == item.kind && t.gh_number == Some(item.number));
        // Retain a previously imported PR's identity and local metadata; only suppress new cards.
        if item.kind == Kind::Pr && !stored && item.closes.iter().any(|n| issues.contains(n)) {
            continue;
        }
        let linked = if item.kind == Kind::Issue {
            linked_pr(gh, github, item.number)?
        } else {
            None
        };
        changed |= repo.upsert_github(
            p,
            item,
            &item.cache(linked.as_ref(), &timestamp),
            &timestamp,
        )?;
        count += 1;
    }
    Ok((changed, count))
}
pub(super) fn linked_pr(gh: &dyn Gh, repo: &str, issue: i64) -> Result<Option<Item>, PunchError> {
    Ok(gh
        .linked(repo, issue)?
        .into_iter()
        .filter(|p| {
            p.kind == Kind::Pr
                && (p.state == "OPEN" || p.state == "MERGED")
                && p.closes.contains(&issue)
        })
        .max_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then(a.number.cmp(&b.number))
        }))
}

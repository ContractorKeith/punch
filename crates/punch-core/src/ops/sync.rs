//! Pull each repository atomically; report failures without losing successful projects.
use super::project::{find_project, now};
use crate::{
    error::*,
    github::{Gh, Item},
    model::*,
    store::Store,
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
    let projects = store.read_transaction(|repo| {
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
        let result = (|| {
            let existing = store.read_transaction(|repo| Ok(repo.tasks()?.into_iter()
                .filter(|t| t.project_id == p.id && t.kind != Kind::Local).collect::<Vec<_>>()))?;
            let fetched = fetch_project(gh, &p, &existing)?;
            store.transaction(|repo| {
                let timestamp = now()?;
                let mut changed = false;
                for (item, linked) in &fetched {
                    changed |= repo.upsert_github(&p, item, &item.cache(linked.as_ref(), &timestamp), &timestamp)?;
                }
                Ok((changed, fetched.len()))
            })
        })();
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
    Ok(Outcome::new(results.iter().any(|r| r.changed), results))
}
fn fetch_project(
    gh: &dyn Gh,
    p: &Project,
    existing: &[Task],
) -> Result<Vec<(Item, Option<Item>)>, PunchError> {
    let github = p.github_repo.as_deref().ok_or_else(|| PunchError::storage("missing GitHub repo"))?;
    let mut items = gh.list(github, p.sync_scope)?;
    for task in existing {
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
    let mut fetched = vec![];
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
        fetched.push((item.clone(), linked));
    }
    Ok(fetched)
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

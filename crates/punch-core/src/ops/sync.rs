//! Pull each repository atomically; report failures without losing successful projects.
use super::project::{find_project, now};
use crate::{
    error::*,
    github::{Gh, Item},
    model::*,
    store::Store,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

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
            let existing = store.read_transaction(|repo| repo.github_identities(p.id))?;
            let fetched = fetch_project(gh, &p, &existing)?;
            store.transaction(|repo| {
                let timestamp = now()?;
                let mut changed = false;
                let mut position = repo.next_position(p.id)?;
                for (item, linked) in &fetched {
                    changed |= repo.upsert_github(
                        &p,
                        item,
                        &item.cache(linked.as_ref(), &timestamp),
                        &timestamp,
                        position,
                    )?;
                    position += 1.0;
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
    existing: &[(Kind, i64)],
) -> Result<Vec<(Item, Option<Item>)>, PunchError> {
    let github = p
        .github_repo
        .as_deref()
        .ok_or_else(|| PunchError::storage("missing GitHub repo"))?;
    let mut items = vec![];
    let mut indices = BTreeMap::new();
    for item in gh.list(github, p.sync_scope)? {
        item.validate(item.kind, None)?;
        let key = (item.kind.to_string(), item.number);
        if let Some(&index) = indices.get(&key) {
            items[index] = item;
        } else {
            indices.insert(key, items.len());
            items.push(item);
        }
    }
    let stored: BTreeSet<_> = existing
        .iter()
        .map(|(kind, n)| (kind.to_string(), *n))
        .collect();
    for &(kind, number) in existing {
        // Refresh stored numbers even when outside the open/scope query.
        let fresh = gh.get(github, kind, number)?;
        fresh.validate(kind, Some(number))?;
        let key = (kind.to_string(), number);
        if let Some(&index) = indices.get(&key) {
            items[index] = fresh;
        } else {
            indices.insert(key, items.len());
            items.push(fresh);
        }
    }
    let issues: BTreeSet<_> = items
        .iter()
        .filter(|i| i.kind == Kind::Issue)
        .map(|i| i.number)
        .collect();
    let mut fetched = vec![];
    for item in &items {
        let stored = stored.contains(&(item.kind.to_string(), item.number));
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
    let linked = gh.linked(repo, issue)?;
    for item in &linked {
        item.validate(Kind::Pr, None)?;
    }
    Ok(linked
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

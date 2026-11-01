//! Basecamp configuration and the shared task mirror boundary.
use super::project::find_project;
use crate::{error::*, model::*, store::Store};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct BcLink {
    pub project: String,
    pub bucket_id: String,
    pub card_table_id: String,
    pub column_map: BTreeMap<String, String>,
}
#[derive(Debug, Serialize)]
pub struct BcStatus {
    pub project: String,
    pub link: BasecampLink,
    pub dirty_cards: Vec<Task>,
}
pub fn bc_link(
    store: &mut Store,
    actor: &Actor,
    args: BcLink,
) -> Result<Outcome<BasecampLink>, PunchError> {
    if actor.is_agent() {
        return Err(PunchError::refusal(
            AGENT_CANNOT_MANAGE_PROJECTS,
            &args.project,
            "agents cannot link Basecamp",
        ));
    }
    if args.bucket_id.trim().is_empty() || args.card_table_id.trim().is_empty() {
        return Err(PunchError::usage(
            "bucket and card table IDs must not be empty",
        ));
    }
    for (column, id) in &args.column_map {
        column.parse::<Column>()?;
        if id.trim().is_empty() {
            return Err(PunchError::usage("mapped column IDs must not be empty"));
        }
    }
    store.transaction(|repo| {
        let project = find_project(repo, &args.project)?;
        let link = BasecampLink {
            project_id: project.id,
            bucket_id: args.bucket_id,
            card_table_id: args.card_table_id,
            column_map: args.column_map,
        };
        let links = repo.basecamp_links()?;
        let previous = links.iter().find(|l| l.project_id == project.id);
        let changed = previous != Some(&link);
        if changed {
            if previous.is_some_and(|old| {
                old.bucket_id != link.bucket_id || old.card_table_id != link.card_table_id
            }) {
                repo.clear_basecamp_cards(project.id)?;
            }
            repo.save_basecamp_link(&link)?;
        }
        Ok(Outcome::new(changed, link))
    })
}
pub fn bc_status(
    store: &mut Store,
    _actor: &Actor,
    project: Option<&str>,
) -> Result<Outcome<Vec<BcStatus>>, PunchError> {
    store.read_transaction(|repo| {
        if let Some(slug) = project {
            find_project(repo, slug)?;
        }
        let projects = repo.projects()?;
        let tasks = repo.tasks()?;
        let mut status = vec![];
        for link in repo.basecamp_links()? {
            if let Some(p) = projects
                .iter()
                .find(|p| p.id == link.project_id && project.is_none_or(|s| p.slug == s))
            {
                let dirty_cards = tasks
                    .iter()
                    .filter(|t| {
                        t.project_id == p.id && t.basecamp_card.as_ref().is_some_and(|c| c.dirty)
                    })
                    .cloned()
                    .collect();
                status.push(BcStatus {
                    project: p.slug.clone(),
                    link,
                    dirty_cards,
                });
            }
        }
        Ok(Outcome::new(false, status))
    })
}

// All task mutations cross this boundary, including add-plan's single transaction.
pub(super) fn mutate<T>(
    store: &mut Store,
    f: impl FnOnce(&crate::store::Repository<'_>) -> Result<Outcome<T>, PunchError>,
    refresh: fn(&mut T, &[Task]),
) -> Result<Outcome<T>, PunchError> {
    let (mut out, changed) = store.transaction(|repo| {
        let before = repo.tasks()?;
        let out = f(repo)?;
        let changed = repo.tasks()?.into_iter().filter(|t| {
            before.iter().find(|b| b.id == t.id)
                .is_none_or(|b| b.column != t.column || b.title != t.title || b.slot != t.slot)
        }).collect::<Vec<_>>();
        prepare(repo, &changed, true)?;
        Ok((out, changed))
    })?;
    let (_, warnings) = mirror(store, &changed)?;
    out.warnings.extend(warnings);
    store.read_transaction(|repo| {
        refresh(&mut out.data, &repo.tasks()?);
        Ok(())
    })?;
    Ok(out)
}

pub(super) fn refresh_task(data: &mut Task, tasks: &[Task]) {
    if let Some(task) = tasks.iter().find(|t| t.id == data.id) {
        data.basecamp_card = task.basecamp_card.clone();
    }
}
pub(super) fn mutate_task(
    store: &mut Store,
    f: impl FnOnce(&crate::store::Repository<'_>) -> Result<Outcome<Task>, PunchError>,
) -> Result<Outcome<Task>, PunchError> {
    mutate(store, f, refresh_task)
}

fn mirror(
    store: &mut Store,
    tasks: &[Task],
) -> Result<(bool, Vec<String>), PunchError> {
    let bc = store.basecamp.clone();
    let (links, projects) = store.read_transaction(|repo| Ok((repo.basecamp_links()?, repo.projects()?)))?;
    let mut warnings = vec![];
    let mut changed = false;
    for task in tasks
        .iter()
        .filter(|t| !t.archived && projects.iter().any(|p| p.id == t.project_id && !p.archived))
    {
        let Some(link) = links.iter().find(|l| l.project_id == task.project_id) else {
            continue;
        };
        let Some(column) = link.column_map.get(&task.column.to_string()) else {
            warnings.push(format!(
                "{BASECAMP_UNMAPPED}: {}: {}",
                task.reference(),
                task.column
            ));
            continue;
        };
        let reference = task
            .gh_number
            .map_or_else(|| task.reference(), |n| format!("#{n}"));
        let title = if let Some(slot) = task.slot.as_deref().filter(|s| !s.is_empty()) {
            format!("{reference} · {slot} · {}", task.title)
        } else {
            format!("{reference} · {}", task.title)
        };
        let claim = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).map_err(PunchError::storage)?;
        let Some((mut card, claimed)) = store.transaction(|repo| {
            if !repo.basecamp_links()?.contains(link) { return Ok(None); }
            let Some(current) = repo.tasks()?.into_iter().find(|t| t.id == task.id) else { return Ok(None); };
            let card = current.basecamp_card.unwrap_or(BasecampCard { card_id: String::new(), dirty: true, last_error: None });
            let claimed = card.card_id.is_empty();
            if claimed && !repo.claim_basecamp_create(task.id, &claim)? { return Ok(None); }
            Ok(Some((card, claimed)))
        })? else { continue; };
        let mut created = false;
        let result = if card.card_id.is_empty() {
            bc.find_card(link, &title).and_then(|existing| {
                if let Some(id) = existing {
                    card.card_id = id;
                    bc.move_card(link, &card.card_id, column)
                        .and_then(|()| bc.update_title(link, &card.card_id, &title))
                } else {
                    bc.create(link, column, &title).and_then(|id| {
                        if id.is_empty() { Err("Basecamp returned an empty card ID".into()) }
                        else { card.card_id = id; created = true; Ok(()) }
                    })
                }
            })
        } else {
            bc.move_card(link, &card.card_id, column)
                .and_then(|()| bc.update_title(link, &card.card_id, &title))
        };
        match result {
            Ok(()) => {
                card.dirty = false;
                card.last_error = None;
            }
            Err(error) => {
                let error = crate::basecamp::diagnostic(&error);
                warnings.push(crate::basecamp::diagnostic(&format!("{BASECAMP_FAILED}: {}: {error}", task.reference())));
                card.dirty = true;
                card.last_error = Some(error);
            }
        }
        let (saved, discard) = store.transaction(|repo| {
            // Never attach an old destination or overwrite another creator's association.
            let current = repo.tasks()?.into_iter().find(|t| t.id == task.id);
            let conflict = !repo.basecamp_links()?.contains(link)
                || current.as_ref().and_then(|t| t.basecamp_card.as_ref()).is_some_and(|c| !c.card_id.is_empty() && c.card_id != card.card_id)
                || (claimed && !repo.owns_basecamp_create(task.id, &claim)?);
            if conflict {
                let stored_same = repo.basecamp_links()?.contains(link)
                    && current.as_ref().and_then(|t| t.basecamp_card.as_ref())
                        .is_some_and(|c| c.card_id == card.card_id);
                return Ok((false, created && !stored_same));
            }
            let mut card = card.clone();
            if current.is_some_and(|t| t.revision != task.revision) { card.dirty = true; }
            let saved = repo.save_basecamp_card(task.id, &card)?;
            if claimed { repo.release_basecamp_create(task.id, &claim)?; }
            Ok((saved, false))
        })?;
        changed |= saved;
        if discard && let Err(error) = bc.trash_card(link, &card.card_id) {
                warnings.push(crate::basecamp::diagnostic(&format!("{BASECAMP_FAILED}: {}: cannot trash duplicate card: {error}", task.reference())));
        }
    }
    Ok((changed, warnings))
}

pub(super) fn reconcile(
    store: &mut Store,
    project: Option<&str>,
) -> Result<(bool, Vec<String>), PunchError> {
    let tasks = store.transaction(|repo| {
        let tasks = repo.tasks()?.into_iter().filter(|t| project.is_none_or(|p| t.project == p)).collect::<Vec<_>>();
        prepare(repo, &tasks, false)?;
        Ok(tasks)
    })?;
    mirror(store, &tasks)
}

// Save retry intent in the same transaction as the local task mutation.
fn prepare(repo: &crate::store::Repository<'_>, tasks: &[Task], dirty_existing: bool) -> Result<(), PunchError> {
    let links = repo.basecamp_links()?;
    let projects = repo.projects()?;
    for task in tasks.iter().filter(|t| !t.archived && (dirty_existing || t.basecamp_card.is_none())) {
        if !projects.iter().any(|p| p.id == task.project_id && !p.archived)
            || !links.iter().any(|l| l.project_id == task.project_id
                && l.column_map.contains_key(&task.column.to_string())) { continue; }
        let mut card = task.basecamp_card.clone().unwrap_or(BasecampCard {
            card_id: String::new(), dirty: true, last_error: None,
        });
        card.dirty = true;
        repo.save_basecamp_card(task.id, &card)?;
    }
    Ok(())
}

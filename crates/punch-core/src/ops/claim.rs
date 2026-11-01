//! Claim GitHub cards remotely before committing any local state.
use super::{
    project::{find_project, now},
    sync::linked_pr,
    task::resolve,
};
use crate::{error::*, github::Gh, model::*, store::Store};

pub fn claim(
    store: &mut Store,
    actor: &Actor,
    gh: &dyn Gh,
    reference: &str,
    slot: Option<&str>,
) -> Result<Outcome<Task>, PunchError> {
    set_claim(
        store,
        actor,
        gh,
        reference,
        true,
        slot.or(if let Actor::Agent(s) = actor {
            Some(s.as_str())
        } else {
            None
        }),
    )
}
pub fn unclaim(
    store: &mut Store,
    actor: &Actor,
    gh: &dyn Gh,
    reference: &str,
) -> Result<Outcome<Task>, PunchError> {
    set_claim(store, actor, gh, reference, false, None)
}
fn set_claim(
    store: &mut Store,
    actor: &Actor,
    gh: &dyn Gh,
    reference: &str,
    claimed: bool,
    slot: Option<&str>,
) -> Result<Outcome<Task>, PunchError> {
    let old = store.read_transaction(|repo| Ok(resolve(&repo.tasks()?, reference)?.clone()))?;
    let fetched = if old.kind != Kind::Local {
        let project = store.read_transaction(|repo| find_project(repo, &old.project))?;
        let github = project.github_repo.as_deref().ok_or_else(|| {
            PunchError::refusal(GH_FAILED, reference, "card has no GitHub repository")
        })?;
        let number = old
            .gh_number
            .ok_or_else(|| PunchError::storage("GitHub card has no number"))?;
        gh.assign(github, number, claimed)?;
        let item = gh.get(github, old.kind, number)?;
        item.validate(old.kind, Some(number))?;
        let linked = if old.kind == Kind::Issue {
            linked_pr(gh, github, number)?
        } else {
            None
        };
        Some((project, item, linked))
    } else {
        None
    };
    super::bc::mutate_task(store, |repo| {
        // Re-read local fields after the remote call so concurrent local edits survive.

        let old = resolve(&repo.tasks()?, reference)?.clone();
        let mut changed = false;
        if let Some((project, item, linked)) = &fetched {
            let timestamp = now()?;
            changed = repo.upsert_github(
                project,
                item,
                &item.cache(linked.as_ref(), &timestamp),
                &timestamp,
                repo.next_position(project.id)?,
            )?;
        }
        let mut task = resolve(&repo.tasks()?, reference)?.clone();
        let before = task.clone();
        task.slot = slot.map(str::to_owned);
        if task.kind == Kind::Local {
            task.assignee = claimed.then(|| actor.label().to_owned());
            task.assignee_kind = claimed.then_some(if actor.is_agent() {
                AssigneeKind::Agent
            } else {
                AssigneeKind::Person
            });
        }
        if task != before {
            task.revision = old.revision + 1;
            task.updated_at = now()?;
            repo.save_task(&task)?;
            changed = true;
        }
        Ok(Outcome::new(
            changed,
            resolve(&repo.tasks()?, reference)?.clone(),
        ))
    })
}

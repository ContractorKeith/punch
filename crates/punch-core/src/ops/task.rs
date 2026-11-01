//! Shared task reference and input validation.
use crate::{error::*, model::*};

// Refuse exhausted revisions before any task or dependency rows are saved.
pub(crate) fn bump_revision(task: &mut Task) -> Result<(), PunchError> {
    task.revision = task
        .revision
        .checked_add(1)
        .ok_or_else(|| PunchError::storage("task revision exhausted"))?;
    Ok(())
}

pub(super) fn resolve<'a>(tasks: &'a [Task], reference: &str) -> Result<&'a Task, PunchError> {
    let found = if let Some((slug, number)) = reference.split_once('#') {
        number.parse::<i64>().ok().and_then(|n| {
            tasks
                .iter()
                .find(|t| t.project == slug && t.gh_number == Some(n))
        })
    } else {
        reference
            .strip_prefix('t')
            .unwrap_or(reference)
            .parse::<i64>()
            .ok()
            .and_then(|id| tasks.iter().find(|t| t.id == id))
    };
    found.ok_or_else(|| PunchError::refusal(UNKNOWN_TASK, reference, "unknown task"))
}
pub(super) fn validate_fields(
    title: Option<&str>,
    priority: Option<Priority>,
    due: Option<&str>,
) -> Result<(), PunchError> {
    if title.is_some_and(|s| s.trim().is_empty()) {
        return Err(PunchError::usage("title must not be empty"));
    }
    if priority.is_some_and(|p| p.0 > 3) {
        return Err(PunchError::usage("priority must be 0-3"));
    }
    if let Some(due) = due {
        // An exact date shape prevents lexical ordering from differing from date ordering.
        let bytes = due.as_bytes();
        if bytes.len() != 10
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || bytes
                .iter()
                .enumerate()
                .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
        {
            return Err(PunchError::usage("due must be YYYY-MM-DD"));
        }
        let year = due[..4].parse::<i32>().map_err(PunchError::usage)?;
        let month = due[5..7].parse::<u8>().map_err(PunchError::usage)?;
        let day = due[8..].parse::<u8>().map_err(PunchError::usage)?;
        let month = time::Month::try_from(month).map_err(PunchError::usage)?;
        time::Date::from_calendar_date(year, month, day).map_err(PunchError::usage)?;
    }
    Ok(())
}
pub(super) fn dependencies(
    tasks: &[Task],
    id: i64,
    project_id: i64,
    refs: &[String],
) -> Result<Vec<i64>, PunchError> {
    let mut after = vec![];
    for reference in refs {
        // A new task may refer to the id it is about to receive.
        if reference
            .strip_prefix('t')
            .unwrap_or(reference)
            .parse::<i64>()
            .ok()
            == Some(id)
        {
            return Err(PunchError::refusal(
                AFTER_SELF,
                reference,
                "task cannot depend on itself",
            ));
        }
        let dep = resolve(tasks, reference)?;
        if dep.id == id {
            return Err(PunchError::refusal(
                AFTER_SELF,
                reference,
                "task cannot depend on itself",
            ));
        }
        if dep.project_id != project_id {
            return Err(PunchError::refusal(
                AFTER_CROSS_PROJECT,
                reference,
                "dependencies must be in the same project",
            ));
        }
        let mut pending = vec![dep.id];
        let mut visited = std::collections::HashSet::new();
        while let Some(next) = pending.pop() {
            if next == id {
                return Err(PunchError::refusal(
                    AFTER_CYCLE,
                    reference,
                    "dependency would create a cycle",
                ));
            }
            if visited.insert(next)
                && let Some(t) = tasks.iter().find(|t| t.id == next)
            {
                pending.extend(&t.after);
            }
        }
        after.push(dep.id);
    }
    after.sort_unstable();
    after.dedup();
    Ok(after)
}

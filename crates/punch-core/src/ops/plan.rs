//! Partial-validation plan imports in one transaction.
use super::{Add, add::add_in};
use crate::{error::*, model::*, store::Store};
use serde::Serialize;
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PlanItem {
    Created { task: Task },
    Existing { task: Task },
    Failed { refusals: Vec<Refusal> },
}
/// Valid items share one transaction; validation failures never write.
pub fn add_plan(
    store: &mut Store,
    _actor: &Actor,
    items: Vec<Result<Add, PunchError>>,
) -> Result<Outcome<Vec<PlanItem>>, PunchError> {
    super::bc::mutate(
        store,
        |repo| {
            let mut changed = false;
            let mut results = vec![];
            for item in items {
                match item.and_then(|a| add_in(repo, &a)) {
                    Ok(out) => {
                        changed |= out.changed;
                        results.push(if out.data.existing {
                            PlanItem::Existing {
                                task: out.data.task,
                            }
                        } else {
                            PlanItem::Created {
                                task: out.data.task,
                            }
                        });
                    }
                    Err(e) if e.exit_code != 3 => results.push(PlanItem::Failed {
                        refusals: vec![e.detail()],
                    }),
                    Err(e) => return Err(e),
                }
            }
            Ok(Outcome::new(changed, results))
        },
        |items, tasks| {
            for item in items {
                if let PlanItem::Created { task } | PlanItem::Existing { task } = item {
                    super::bc::refresh_task(task, tasks);
                }
            }
        },
    )
}

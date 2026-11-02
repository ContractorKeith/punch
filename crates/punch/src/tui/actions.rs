//! External operations run on a separate connection, never on the terminal thread.
use punch_core::{error::PunchError, github::RealGh, model::*, ops, store::Store};
use std::sync::mpsc::{self, Receiver};

pub enum Action {
    Claim { task: Box<Task>, claimed: bool },
    Sync(Option<String>),
}
pub struct Pending {
    pub label: String,
    pub result: Receiver<Result<Outcome<String>, PunchError>>,
}
impl Action {
    pub fn start(self, actor: Actor) -> Result<Pending, PunchError> {
        let label = match &self {
            Self::Claim { task, claimed } => format!(
                "{} {}…",
                if *claimed { "Claiming" } else { "Unclaiming" },
                task.reference()
            ),
            Self::Sync(project) => {
                format!("Syncing {}…", project.as_deref().unwrap_or("all projects"))
            }
        };
        let (send, result) = mpsc::channel();
        std::thread::Builder::new()
            .name("board-action".into())
            .spawn(move || {
                let out = Store::open().and_then(|mut store| self.run(&mut store, &actor));
                let _ = send.send(out);
            })?;
        Ok(Pending { label, result })
    }
    fn run(self, store: &mut Store, actor: &Actor) -> Result<Outcome<String>, PunchError> {
        match self {
            Self::Claim { task, claimed } => {
                let out = ops::claim_task(store, actor, &RealGh, &task, claimed)?;
                Ok(Outcome {
                    changed: out.changed,
                    data: format!(
                        "{} {}",
                        if claimed { "Claimed" } else { "Unclaimed" },
                        task.reference()
                    ),
                    warnings: out.warnings,
                })
            }
            Self::Sync(project) => {
                let out = ops::sync(store, actor, &RealGh, project.as_deref())?;
                let mut messages = vec![format!(
                    "Sync: {} project(s) synced",
                    out.data.iter().filter(|r| r.status == "synced").count()
                )];
                for result in &out.data {
                    if result.status == "synced" {
                        messages.push(format!("{}: synced", result.project));
                    }
                    for refusal in &result.refusals {
                        messages.push(format!(
                            "{}: {}: {}",
                            result.project, refusal.code, refusal.message
                        ));
                    }
                }
                Ok(Outcome {
                    changed: out.changed,
                    data: messages.join("; "),
                    warnings: out.warnings,
                })
            }
        }
    }
}

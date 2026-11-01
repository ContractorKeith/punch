//! Stable refusal codes and errors shared by all adapters.
use serde::Serialize;

pub const OFFLINE: &str = "offline";
pub const GH_FAILED: &str = "gh-failed";
pub const STALE: &str = "stale";
pub const STORE_TOO_NEW: &str = "store-too-new";
pub const UNKNOWN_PROJECT: &str = "unknown-project";
pub const PROJECT_EXISTS: &str = "project-exists";
pub const UNKNOWN_TASK: &str = "unknown-task";
pub const AGENT_CANNOT_EDIT: &str = "agent-cannot-edit";
pub const AGENT_CANNOT_FINISH: &str = "agent-cannot-finish";
pub const AGENT_CANNOT_ARCHIVE: &str = "agent-cannot-archive";
pub const AGENT_CANNOT_MANAGE_PROJECTS: &str = "agent-cannot-manage-projects";
pub const COLUMN_IS_DERIVED: &str = "column-is-derived";
pub const GITHUB_OWNED_FIELD: &str = "github-owned-field";
pub const AFTER_NOT_DONE: &str = "after-not-done";
pub const AFTER_SELF: &str = "after-self";
pub const AFTER_CROSS_PROJECT: &str = "after-cross-project";
pub const AFTER_CYCLE: &str = "after-cycle";
pub const INVALID_INPUT: &str = "invalid-input";
pub const STORAGE_FAILED: &str = "storage-failed";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Refusal {
    pub code: String,
    pub message: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PunchError {
    pub code: String,
    pub message: String,
    pub reference: String,
    pub exit_code: u8,
}
impl PunchError {
    pub fn refusal(code: &str, reference: impl ToString, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            reference: reference.to_string(),
            message: message.to_string(),
            exit_code: 1,
        }
    }
    pub fn usage(message: impl ToString) -> Self {
        Self {
            exit_code: 2,
            ..Self::refusal(INVALID_INPUT, "", message)
        }
    }
    pub fn storage(message: impl ToString) -> Self {
        Self {
            exit_code: 3,
            ..Self::refusal(STORAGE_FAILED, "", message)
        }
    }
    pub fn detail(&self) -> Refusal {
        Refusal {
            code: self.code.clone(),
            message: self.message.clone(),
            reference: self.reference.clone(),
        }
    }
}
impl From<rusqlite::Error> for PunchError {
    fn from(value: rusqlite::Error) -> Self {
        Self::storage(value)
    }
}
impl From<std::io::Error> for PunchError {
    fn from(value: std::io::Error) -> Self {
        Self::storage(value)
    }
}

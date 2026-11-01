//! GitHub boundary. Authentication and transport belong to the installed gh CLI.
mod real;
use crate::{error::PunchError, model::*};
pub use real::RealGh;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub kind: Kind,
    pub number: i64,
    pub title: String,
    pub state: String,
    pub assignees: Vec<Login>,
    pub url: String,
    pub updated_at: String,
    pub author: Option<Login>,
    pub is_draft: Option<bool>,
    pub review_decision: Option<String>,
    #[serde(default)]
    pub closes: Vec<i64>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Login {
    pub login: String,
}
impl Item {
    pub(crate) fn validate(&self, kind: Kind, number: Option<i64>) -> Result<(), PunchError> {
        if self.kind != kind
            || kind == Kind::Local
            || self.number <= 0
            || number.is_some_and(|n| self.number != n)
            || !matches!(self.state.as_str(), "OPEN" | "CLOSED" | "MERGED")
            || (kind == Kind::Issue && self.state == "MERGED")
        {
            return Err(PunchError::refusal(
                crate::error::GH_FAILED,
                "gh",
                "invalid GitHub card identity or state",
            ));
        }
        Ok(())
    }

    pub(crate) fn cache(&self, linked: Option<&Item>, fetched: &str) -> GhCache {
        let mut assignees: Vec<_> = self.assignees.iter().map(|a| a.login.clone()).collect();
        assignees.sort();
        assignees.dedup();
        GhCache {
            state: self.state.clone(),
            assignees,
            is_draft: self.is_draft,
            review_decision: self.review_decision.clone(),
            linked_pr_number: linked.map(|p| p.number),
            linked_pr_state: linked.map(|p| p.state.clone()),
            linked_pr_draft: linked.and_then(|p| p.is_draft),
            linked_pr_review: linked.and_then(|p| p.review_decision.clone()),
            url: self.url.clone(),
            fetched_at: fetched.into(),
        }
    }
}
pub trait Gh {
    fn list(&self, repo: &str, scope: SyncScope) -> Result<Vec<Item>, PunchError>;
    fn get(&self, repo: &str, kind: Kind, number: i64) -> Result<Item, PunchError>;
    fn linked(&self, repo: &str, issue: i64) -> Result<Vec<Item>, PunchError>;
    fn assign(&self, repo: &str, number: i64, claim: bool) -> Result<(), PunchError>;
}

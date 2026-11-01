//! Domain values and operation payloads; no adapter dependencies.
use crate::error::PunchError;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $value)] $variant),+ }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(match self { $(Self::$variant => $value),+ }) }
        }
        impl FromStr for $name {
            type Err = PunchError;
            fn from_str(s: &str) -> Result<Self, Self::Err> { match s { $($value => Ok(Self::$variant)),+, _ => Err(PunchError::usage(format!("invalid {}: {s}", stringify!($name)))) } }
        }
    }
}
string_enum!(Column { Todo => "todo", Doing => "doing", Review => "review", Blocked => "blocked", Done => "done", Shipped => "shipped" });
impl Column {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Shipped)
    }
}
string_enum!(Kind { Local => "local", Issue => "issue", Pr => "pr" });
string_enum!(AssigneeKind { Person => "person", Agent => "agent" });
string_enum!(SyncScope { Assigned => "assigned", All => "all" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Priority(pub u8);
impl Default for Priority {
    fn default() -> Self {
        Self(2)
    }
}
impl<'de> Deserialize<'de> for Priority {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let n = u8::deserialize(d)?;
        if n <= 3 {
            Ok(Self(n))
        } else {
            Err(serde::de::Error::custom("priority must be 0-3"))
        }
    }
}
impl FromStr for Priority {
    type Err = PunchError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u8>() {
            Ok(n @ 0..=3) => Ok(Self(n)),
            _ => Err(PunchError::usage("priority must be 0-3")),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Person,
    Agent(String),
}
impl Actor {
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent(_))
    }
    pub fn label(&self) -> &str {
        match self {
            Self::Person => "person",
            Self::Agent(slot) => slot,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub github_repo: Option<String>,
    pub sync_scope: SyncScope,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GhCache {
    pub state: String,
    pub assignees: Vec<String>,
    pub is_draft: Option<bool>,
    pub review_decision: Option<String>,
    pub linked_pr_number: Option<i64>,
    pub linked_pr_state: Option<String>,
    pub linked_pr_draft: Option<bool>,
    pub linked_pr_review: Option<String>,
    pub url: String,
    pub fetched_at: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasecampCard {
    pub card_id: String,
    pub dirty: bool,
    pub last_error: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub project_id: i64,
    pub project: String,
    pub kind: Kind,
    pub gh_number: Option<i64>,
    pub title: String,
    pub notes: String,
    pub local_column: Option<Column>,
    pub column: Column,
    pub priority: Priority,
    pub due: Option<String>,
    pub assignee: Option<String>,
    pub assignee_kind: Option<AssigneeKind>,
    pub slot: Option<String>,
    pub position: f64,
    pub archived: bool,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub after: Vec<i64>,
    pub gh_cache: Option<GhCache>,
    pub basecamp_card: Option<BasecampCard>,
}
impl Task {
    pub fn reference(&self) -> String {
        format!("t{}", self.id)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Outcome<T> {
    pub changed: bool,
    pub data: T,
    pub warnings: Vec<String>,
}
impl<T> Outcome<T> {
    pub fn new(changed: bool, data: T) -> Self {
        Self {
            changed,
            data,
            warnings: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasecampLink {
    pub project_id: i64,
    pub bucket_id: String,
    pub card_table_id: String,
    pub column_map: std::collections::BTreeMap<String, String>,
}

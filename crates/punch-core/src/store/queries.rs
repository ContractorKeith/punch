//! Query methods used by domain operations within a single transaction.
use crate::{derive, error::PunchError, model::*};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::str::FromStr;

pub(crate) struct Repository<'a> {
    db: &'a Connection,
}
fn parse<T: FromStr>(s: String) -> rusqlite::Result<T>
where
    T::Err: std::fmt::Display,
{
    s.parse().map_err(|e: T::Err| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(e.to_string())),
        )
    })
}
fn project_row(r: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        slug: r.get(1)?,
        name: r.get(2)?,
        github_repo: r.get(3)?,
        sync_scope: parse(r.get(4)?)?,
        archived: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}
impl<'a> Repository<'a> {
    pub(crate) fn new(db: &'a Connection) -> Self {
        Self { db }
    }
    pub(crate) fn projects(&self) -> Result<Vec<Project>, PunchError> {
        Ok(self.db.prepare("SELECT id,slug,name,github_repo,sync_scope,archived,created_at,updated_at FROM projects ORDER BY slug")?.query_map([], project_row)?.collect::<Result<_,_>>()?)
    }
    pub(crate) fn insert_project(
        &self,
        slug: &str,
        name: &str,
        github: Option<&str>,
        scope: SyncScope,
        now: &str,
    ) -> Result<(), PunchError> {
        self.db.execute("INSERT INTO projects(slug,name,github_repo,sync_scope,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)", params![slug,name,github,scope.to_string(),now])?;
        Ok(())
    }
    pub(crate) fn archive_project(&self, id: i64, now: &str) -> Result<(), PunchError> {
        self.db.execute(
            "UPDATE projects SET archived=1,updated_at=?2 WHERE id=?1",
            params![id, now],
        )?;
        Ok(())
    }
    pub(crate) fn tasks(&self) -> Result<Vec<Task>, PunchError> {
        let mut stmt = self.db.prepare("SELECT t.id,t.project_id,p.slug,t.kind,t.gh_number,t.title,t.notes,t.local_column,t.priority,t.due,t.assignee,t.assignee_kind,t.slot,t.position,t.archived,t.revision,t.created_at,t.updated_at FROM tasks t JOIN projects p ON p.id=t.project_id ORDER BY t.id")?;
        let mut tasks = stmt
            .query_map([], |r| {
                Ok(Task {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    project: r.get(2)?,
                    kind: parse(r.get(3)?)?,
                    gh_number: r.get(4)?,
                    title: r.get(5)?,
                    notes: r.get(6)?,
                    local_column: r.get::<_, Option<String>>(7)?.map(parse).transpose()?,
                    column: Column::Todo,
                    priority: Priority(r.get(8)?),
                    due: r.get(9)?,
                    assignee: r.get(10)?,
                    assignee_kind: r.get::<_, Option<String>>(11)?.map(parse).transpose()?,
                    slot: r.get(12)?,
                    position: r.get(13)?,
                    archived: r.get(14)?,
                    revision: r.get(15)?,
                    created_at: r.get(16)?,
                    updated_at: r.get(17)?,
                    after: vec![],
                    gh_cache: None,
                    basecamp_card: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for task in &mut tasks {
            task.after = self
                .db
                .prepare("SELECT after_id FROM task_after WHERE task_id=?1 ORDER BY after_id")?
                .query_map([task.id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            task.gh_cache = self.db.query_row("SELECT state,assignees,is_draft,review_decision,linked_pr_number,linked_pr_state,linked_pr_draft,linked_pr_review,url,fetched_at FROM gh_cache WHERE task_id=?1", [task.id], |r| {
                let json:String=r.get(1)?;
                let assignees=serde_json::from_str(&json).map_err(|e|rusqlite::Error::FromSqlConversionFailure(1,rusqlite::types::Type::Text,Box::new(e)))?;
                Ok(GhCache { state:r.get(0)?,assignees,is_draft:r.get(2)?,review_decision:r.get(3)?,linked_pr_number:r.get(4)?,linked_pr_state:r.get(5)?,linked_pr_draft:r.get(6)?,linked_pr_review:r.get(7)?,url:r.get(8)?,fetched_at:r.get(9)? })
            }).optional()?;
            task.basecamp_card = self
                .db
                .query_row(
                    "SELECT card_id,dirty,last_error FROM basecamp_cards WHERE task_id=?1",
                    [task.id],
                    |r| {
                        Ok(BasecampCard {
                            card_id: r.get(0)?,
                            dirty: r.get(1)?,
                            last_error: r.get(2)?,
                        })
                    },
                )
                .optional()?;
            task.column = if task.kind == Kind::Local {
                task.local_column
                    .ok_or_else(|| PunchError::storage("local task has no column"))?
            } else {
                derive::column(task.kind, task.gh_cache.as_ref())
            };
        }
        Ok(tasks)
    }
    pub(crate) fn insert_task(&self, task: &Task) -> Result<i64, PunchError> {
        self.db.execute("INSERT INTO tasks(project_id,kind,title,notes,local_column,priority,due,assignee,assignee_kind,slot,position,created_at,updated_at) VALUES(?1,'local',?2,?3,'todo',?4,?5,?6,?7,?8,?9,?10,?10)",params![task.project_id,task.title,task.notes,task.priority.0,task.due,task.assignee,task.assignee_kind.map(|v|v.to_string()),task.slot,task.position,task.created_at])?;
        Ok(self.db.last_insert_rowid())
    }
    pub(crate) fn save_task(&self, t: &Task) -> Result<(), PunchError> {
        self.db.execute("UPDATE tasks SET title=?2,notes=?3,local_column=?4,priority=?5,due=?6,assignee=?7,assignee_kind=?8,slot=?9,archived=?10,revision=?11,updated_at=?12 WHERE id=?1",params![t.id,t.title,t.notes,t.local_column.map(|v|v.to_string()),t.priority.0,t.due,t.assignee,t.assignee_kind.map(|v|v.to_string()),t.slot,t.archived,t.revision,t.updated_at])?;
        Ok(())
    }
    pub(crate) fn set_after(&self, id: i64, after: &[i64]) -> Result<(), PunchError> {
        self.db
            .execute("DELETE FROM task_after WHERE task_id=?1", [id])?;
        for dep in after {
            self.db.execute(
                "INSERT INTO task_after(task_id,after_id) VALUES(?1,?2)",
                params![id, dep],
            )?;
        }
        Ok(())
    }
}

impl Repository<'_> {
    pub(crate) fn upsert_github(
        &self,
        project: &Project,
        item: &crate::github::Item,
        cache: &GhCache,
        now: &str,
    ) -> Result<bool, PunchError> {
        let tasks = self.tasks()?;
        let assignee = cache.assignees.first().cloned();
        let id = if let Some(old) = tasks.iter().find(|t| {
            t.project_id == project.id && t.kind == item.kind && t.gh_number == Some(item.number)
        }) {
            let mut task = old.clone();
            task.title = item.title.clone();
            task.assignee = assignee;
            task.assignee_kind = task.assignee.as_ref().map(|_| AssigneeKind::Person);
            if task == *old && old.gh_cache.as_ref() == Some(cache) {
                return Ok(false);
            }
            task.revision += 1;
            task.updated_at = now.into();
            self.save_task(&task)?;
            task.id
        } else {
            let position = tasks
                .iter()
                .filter(|t| t.project_id == project.id)
                .map(|t| t.position)
                .fold(0.0, f64::max)
                + 1.0;
            self.db.execute("INSERT INTO tasks(project_id,kind,gh_number,title,assignee,assignee_kind,position,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)", params![project.id,item.kind.to_string(),item.number,item.title,assignee,assignee.as_ref().map(|_|"person"),position,now])?;
            self.db.last_insert_rowid()
        };
        self.db.execute("INSERT OR REPLACE INTO gh_cache(task_id,state,assignees,is_draft,review_decision,linked_pr_number,linked_pr_state,linked_pr_draft,linked_pr_review,url,fetched_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![id,cache.state,serde_json::to_string(&cache.assignees).map_err(PunchError::storage)?,cache.is_draft,cache.review_decision,cache.linked_pr_number,cache.linked_pr_state,cache.linked_pr_draft,cache.linked_pr_review,cache.url,cache.fetched_at])?;
        Ok(true)
    }
}

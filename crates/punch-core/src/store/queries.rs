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
        self.query_tasks("", [])
    }
    pub(crate) fn task(&self, id: i64) -> Result<Option<Task>, PunchError> {
        Ok(self.query_tasks("WHERE t.id=?1", [id])?.pop())
    }
    pub(crate) fn github_task_id(
        &self,
        slug: &str,
        number: i64,
    ) -> Result<Option<i64>, PunchError> {
        Ok(self.db.query_row(
            "SELECT t.id FROM tasks t JOIN projects p ON p.id=t.project_id WHERE p.slug=?1 AND t.gh_number=?2 ORDER BY t.id LIMIT 1",
            params![slug, number], |r| r.get(0),
        ).optional()?)
    }
    fn query_tasks(
        &self,
        filter: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<Task>, PunchError> {
        let mut stmt = self.db.prepare(&format!("SELECT t.id,t.project_id,p.slug,t.kind,t.gh_number,t.title,t.notes,t.local_column,t.priority,t.due,t.assignee,t.assignee_kind,t.slot,t.position,t.archived,t.revision,t.created_at,t.updated_at FROM tasks t JOIN projects p ON p.id=t.project_id {filter} ORDER BY t.id"))?;
        let mut tasks = stmt
            .query_map(params, |r| {
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
    pub(crate) fn ready(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Task>, PunchError> {
        // Keep only IDs and derived columns in the shared dependency scan. Notes and
        // auxiliary detail are loaded only after filtering, ordering, and LIMIT.
        let mut stmt = self.db.prepare(
            "WITH task_columns AS (
                SELECT t.id, CASE
                    WHEN t.kind='local' THEN t.local_column
                    WHEN g.task_id IS NULL THEN 'todo'
                    WHEN (t.kind='pr' AND g.state='MERGED') OR
                         (t.kind='issue' AND g.state='CLOSED' AND g.linked_pr_state='MERGED') THEN 'shipped'
                    WHEN g.state='CLOSED' THEN 'done'
                    WHEN t.kind='pr' AND g.state='OPEN' THEN
                        CASE WHEN g.review_decision='CHANGES_REQUESTED' OR g.is_draft=1 THEN 'doing' ELSE 'review' END
                    WHEN t.kind='issue' AND g.linked_pr_state='OPEN' THEN
                        CASE WHEN g.linked_pr_review='CHANGES_REQUESTED' OR g.linked_pr_draft=1 THEN 'doing' ELSE 'review' END
                    WHEN t.kind='issue' AND g.state='OPEN' AND json_array_length(g.assignees)>0 THEN 'doing'
                    ELSE 'todo' END AS column
                FROM tasks t LEFT JOIN gh_cache g ON g.task_id=t.id
            )
            SELECT t.id FROM tasks t JOIN projects p ON p.id=t.project_id
            JOIN task_columns c ON c.id=t.id
            WHERE c.column='todo' AND t.archived=0 AND p.archived=0
                AND (?1 IS NULL OR p.slug=?1)
                AND NOT EXISTS (
                    SELECT 1 FROM task_after a JOIN task_columns d ON d.id=a.after_id
                    WHERE a.task_id=t.id AND d.column NOT IN ('done','shipped')
                )
            ORDER BY t.priority, t.due IS NULL, t.due, t.position, t.id LIMIT ?2"
        )?;
        let limit = limit
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(i64::MAX);
        let ids = stmt
            .query_map(params![project, limit], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                self.task(id)?
                    .ok_or_else(|| PunchError::storage("ready task disappeared"))
            })
            .collect()
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
    pub(crate) fn github_identities(&self, project: i64) -> Result<Vec<(Kind, i64)>, PunchError> {
        Ok(self
            .db
            .prepare(
                "SELECT kind,gh_number FROM tasks WHERE project_id=?1 AND kind IN ('issue','pr')",
            )?
            .query_map([project], |r| Ok((parse(r.get(0)?)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }
    pub(crate) fn next_position(&self, project: i64) -> Result<f64, PunchError> {
        Ok(self.db.query_row(
            "SELECT COALESCE(MAX(position),0)+1 FROM tasks WHERE project_id=?1",
            [project],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn upsert_github(
        &self,
        project: &Project,
        item: &crate::github::Item,
        cache: &GhCache,
        now: &str,
        position: f64,
    ) -> Result<bool, PunchError> {
        let old = self
            .query_tasks(
                "WHERE t.project_id=?1 AND t.kind=?2 AND t.gh_number=?3",
                params![project.id, item.kind.to_string(), item.number],
            )?
            .pop();
        let assignee = cache.assignees.first().cloned();
        let id = if let Some(old) = old.as_ref() {
            let mut task = old.clone();
            task.title = item.title.clone();
            task.assignee = assignee;
            task.assignee_kind = task.assignee.as_ref().map(|_| AssigneeKind::Person);
            // fetched_at alone is not a change: refresh it without a revision bump.
            let same_cache = old.gh_cache.as_ref().is_some_and(|c| {
                GhCache {
                    fetched_at: cache.fetched_at.clone(),
                    ..c.clone()
                } == *cache
            });
            if task == *old && same_cache {
                self.db.execute(
                    "UPDATE gh_cache SET fetched_at=?1 WHERE task_id=?2",
                    params![cache.fetched_at, old.id],
                )?;
                return Ok(false);
            }
            task.revision += 1;
            task.updated_at = now.into();
            self.save_task(&task)?;
            task.id
        } else {
            self.db.execute("INSERT INTO tasks(project_id,kind,gh_number,title,assignee,assignee_kind,position,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)", params![project.id,item.kind.to_string(),item.number,item.title,assignee,assignee.as_ref().map(|_|"person"),position,now])?;
            self.db.last_insert_rowid()
        };
        self.db.execute("INSERT OR REPLACE INTO gh_cache(task_id,state,assignees,is_draft,review_decision,linked_pr_number,linked_pr_state,linked_pr_draft,linked_pr_review,url,fetched_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![id,cache.state,serde_json::to_string(&cache.assignees).map_err(PunchError::storage)?,cache.is_draft,cache.review_decision,cache.linked_pr_number,cache.linked_pr_state,cache.linked_pr_draft,cache.linked_pr_review,cache.url,cache.fetched_at])?;
        Ok(true)
    }
}

impl Repository<'_> {
    pub(crate) fn basecamp_links(&self) -> Result<Vec<BasecampLink>, PunchError> {
        let rows = self.db.prepare("SELECT project_id,bucket_id,card_table_id,column_map FROM basecamp_links ORDER BY project_id")?
            .query_map([], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?
            .collect::<Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(project_id, bucket_id, card_table_id, json)| {
                Ok(BasecampLink {
                    project_id,
                    bucket_id,
                    card_table_id,
                    column_map: serde_json::from_str(&json).map_err(PunchError::storage)?,
                })
            })
            .collect()
    }
    pub(crate) fn save_basecamp_link(&self, link: &BasecampLink) -> Result<(), PunchError> {
        self.db.execute("INSERT INTO basecamp_links(project_id,bucket_id,card_table_id,column_map) VALUES(?1,?2,?3,?4) ON CONFLICT(project_id) DO UPDATE SET bucket_id=excluded.bucket_id,card_table_id=excluded.card_table_id,column_map=excluded.column_map",
            params![link.project_id,link.bucket_id,link.card_table_id,serde_json::to_string(&link.column_map).map_err(PunchError::storage)?])?;
        Ok(())
    }
}

impl Repository<'_> {
    pub(crate) fn save_basecamp_card(
        &self,
        task_id: i64,
        card: &BasecampCard,
    ) -> Result<bool, PunchError> {
        let changed = self.db.execute("INSERT INTO basecamp_cards(task_id,card_id,dirty,last_error) VALUES(?1,?2,?3,?4) ON CONFLICT(task_id) DO UPDATE SET card_id=excluded.card_id,dirty=excluded.dirty,last_error=excluded.last_error WHERE card_id!=excluded.card_id OR dirty!=excluded.dirty OR last_error IS NOT excluded.last_error",
            params![task_id,card.card_id,card.dirty,card.last_error])?;
        Ok(changed != 0)
    }
}

impl Repository<'_> {
    pub(crate) fn clear_basecamp_cards(&self, project_id: i64) -> Result<(), PunchError> {
        self.db.execute("DELETE FROM basecamp_cards WHERE task_id IN (SELECT id FROM tasks WHERE project_id=?1)", [project_id])?;
        Ok(())
    }
}

impl Repository<'_> {
    pub(crate) fn claim_basecamp_create(
        &self,
        task_id: i64,
        claim: &str,
    ) -> Result<bool, PunchError> {
        Ok(self.db.execute("UPDATE basecamp_cards SET create_claim=?2 WHERE task_id=?1 AND card_id='' AND (create_claim IS NULL OR julianday(create_claim) < julianday('now', '-5 minutes'))", params![task_id, claim])? != 0)
    }
    pub(crate) fn owns_basecamp_create(
        &self,
        task_id: i64,
        claim: &str,
    ) -> Result<bool, PunchError> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM basecamp_cards WHERE task_id=?1 AND create_claim=?2)",
            params![task_id, claim],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn release_basecamp_create(
        &self,
        task_id: i64,
        claim: &str,
    ) -> Result<(), PunchError> {
        self.db.execute(
            "UPDATE basecamp_cards SET create_claim=NULL WHERE task_id=?1 AND create_claim=?2",
            params![task_id, claim],
        )?;
        Ok(())
    }
}

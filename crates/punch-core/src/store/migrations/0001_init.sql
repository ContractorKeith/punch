CREATE TABLE projects (
 id INTEGER PRIMARY KEY, slug TEXT UNIQUE NOT NULL, name TEXT NOT NULL,
 github_repo TEXT UNIQUE, sync_scope TEXT NOT NULL DEFAULT 'assigned' CHECK(sync_scope IN ('assigned','all')),
 archived INTEGER NOT NULL DEFAULT 0, created_at TEXT, updated_at TEXT
);
CREATE TABLE tasks (
 id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL REFERENCES projects(id),
 kind TEXT NOT NULL CHECK(kind IN ('local','issue','pr')), gh_number INTEGER,
 title TEXT NOT NULL, notes TEXT NOT NULL DEFAULT '',
 local_column TEXT CHECK(local_column IN ('todo','doing','review','blocked','done','shipped')),
 priority INTEGER NOT NULL DEFAULT 2 CHECK(priority BETWEEN 0 AND 3), due TEXT,
 assignee TEXT, assignee_kind TEXT CHECK(assignee_kind IN ('person','agent')), slot TEXT,
 position REAL NOT NULL, archived INTEGER NOT NULL DEFAULT 0, revision INTEGER NOT NULL DEFAULT 1,
 created_at TEXT, updated_at TEXT, UNIQUE(project_id,kind,gh_number),
 CHECK((kind='local' AND local_column IS NOT NULL AND gh_number IS NULL) OR
       (kind IN ('issue','pr') AND local_column IS NULL AND gh_number IS NOT NULL))
);
CREATE TABLE task_after (
 task_id INTEGER REFERENCES tasks(id), after_id INTEGER REFERENCES tasks(id), PRIMARY KEY(task_id,after_id)
);
CREATE TABLE gh_cache (
 task_id INTEGER PRIMARY KEY REFERENCES tasks(id), state TEXT NOT NULL, assignees TEXT NOT NULL,
 is_draft INTEGER, review_decision TEXT, linked_pr_number INTEGER, linked_pr_state TEXT,
 linked_pr_draft INTEGER, linked_pr_review TEXT, url TEXT NOT NULL, fetched_at TEXT NOT NULL
);
CREATE TABLE basecamp_links (
 project_id INTEGER PRIMARY KEY REFERENCES projects(id), bucket_id TEXT NOT NULL,
 card_table_id TEXT NOT NULL, column_map TEXT NOT NULL
);
CREATE TABLE basecamp_cards (
 task_id INTEGER PRIMARY KEY REFERENCES tasks(id), card_id TEXT NOT NULL,
 dirty INTEGER NOT NULL DEFAULT 0, last_error TEXT
);

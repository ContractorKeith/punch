#![allow(clippy::unwrap_used)]
use super::*;
use crate::{error::*, model::*, ops};
use std::process::Command;

// Each database test runs in its own process with its own PUNCH_STATE_DIR.
// This avoids mutating process-global environment during parallel Rust tests.
fn isolated(f: impl FnOnce(&Path)) {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("PUNCH_TEST_CASE").as_deref() == Ok(&name) {
        f(Path::new(&std::env::var_os("PUNCH_STATE_DIR").unwrap()));
    } else {
        let dir = tempfile::tempdir().unwrap();
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env("PUNCH_STATE_DIR", dir.path())
            .env("PUNCH_TEST_CASE", name)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
fn project(store: &mut Store) {
    ops::project_add(
        store,
        &Actor::Person,
        ops::ProjectAdd {
            slug: "demo".into(),
            name: None,
            github: None,
            scope: None,
        },
    )
    .unwrap();
}
fn add(store: &mut Store, title: &str) -> Task {
    ops::add(
        store,
        &Actor::Person,
        ops::Add {
            project: "demo".into(),
            title: title.into(),
            notes: String::new(),
            priority: Priority(2),
            due: None,
            after: vec![],
            assignee: None,
            agent: false,
        },
    )
    .unwrap()
    .data
    .task
}
#[test]
fn full_schema_pragmas_and_reopen_preserve_data() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        for (pragma, expected) in [
            ("foreign_keys", "1"),
            ("busy_timeout", "5000"),
            ("journal_mode", "wal"),
        ] {
            let got = store
                .connection
                .query_row(&format!("PRAGMA {pragma}"), [], |r| {
                    r.get::<_, rusqlite::types::Value>(0)
                })
                .unwrap();
            let got = match got {
                rusqlite::types::Value::Integer(v) => v.to_string(),
                rusqlite::types::Value::Text(v) => v,
                _ => panic!("unexpected pragma"),
            };
            assert_eq!(got, expected);
        }
        for table in [
            "projects",
            "tasks",
            "task_after",
            "gh_cache",
            "basecamp_links",
            "basecamp_cards",
            "schema_version",
        ] {
            assert_eq!(
                store
                    .connection
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        [table],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
        }
        project(&mut store);
        add(&mut store, "persisted");
        drop(store);
        let mut reopened = Store::open().unwrap();
        assert_eq!(
            ops::show(&mut reopened, &Actor::Person, "t1")
                .unwrap()
                .data
                .title,
            "persisted"
        );
        assert_eq!(
            reopened
                .connection
                .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    });
}
#[test]
fn newer_store_refuses_without_modifying_database() {
    isolated(|path| {
        let db = Connection::open(path.join("punch.db")).unwrap();
        db.execute_batch("CREATE TABLE schema_version(version INTEGER NOT NULL); INSERT INTO schema_version VALUES(2);").unwrap();
        drop(db);
        let before = std::fs::read(path.join("punch.db")).unwrap();
        let err = Store::open().err().unwrap();
        assert_eq!(err.code, STORE_TOO_NEW);
        assert_eq!(err.exit_code, 3);
        assert_eq!(std::fs::read(path.join("punch.db")).unwrap(), before);
    });
}
#[test]
fn noops_execute_no_writes_and_refusals_preserve_revision() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        add(&mut store, "same");
        let before = store.connection.total_changes();
        assert!(
            !ops::move_task(&mut store, &Actor::Person, "t1", Column::Todo)
                .unwrap()
                .changed
        );
        assert!(
            !ops::edit(
                &mut store,
                &Actor::Person,
                "t1",
                ops::Edit {
                    title: Some("same".into()),
                    after: Some(vec![]),
                    ..Default::default()
                }
            )
            .unwrap()
            .changed
        );
        assert!(
            ops::add(
                &mut store,
                &Actor::Person,
                ops::Add {
                    project: "demo".into(),
                    title: "same".into(),
                    notes: "ignored".into(),
                    priority: Priority(0),
                    due: None,
                    after: vec![],
                    assignee: None,
                    agent: false
                }
            )
            .unwrap()
            .data
            .existing
        );
        assert!(
            !ops::project_add(
                &mut store,
                &Actor::Person,
                ops::ProjectAdd {
                    slug: "demo".into(),
                    name: None,
                    github: None,
                    scope: None
                }
            )
            .unwrap()
            .changed
        );
        assert_eq!(store.connection.total_changes(), before);
        let err = ops::edit(
            &mut store,
            &Actor::Person,
            "t1",
            ops::Edit {
                title: Some("changed".into()),
                after: Some(vec!["t1".into()]),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, AFTER_SELF);
        assert_eq!(store.connection.total_changes(), before);
        let task = ops::show(&mut store, &Actor::Person, "t1").unwrap().data;
        assert_eq!(task.revision, 1);
        assert_eq!(task.title, "same");
    });
}
#[test]
fn github_cards_use_derived_columns_and_refuse_owned_writes() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        store.connection.execute_batch("INSERT INTO tasks(project_id,kind,gh_number,title,position,created_at,updated_at) VALUES(1,'issue',566,'issue',1,'2026-11-01T00:00:00Z','2026-11-01T00:00:00Z'); INSERT INTO gh_cache(task_id,state,assignees,url,fetched_at) VALUES(1,'OPEN','[]','https://example.test/566','2026-11-01T00:00:00Z'); INSERT INTO basecamp_cards(task_id,card_id) VALUES(1,'card-1');").unwrap();
        let actor = Actor::Agent("bot".into());
        assert_eq!(
            ops::show(&mut store, &actor, "demo#566")
                .unwrap()
                .data
                .column,
            Column::Todo
        );
        assert_eq!(
            ops::ready(&mut store, &actor, ops::Ready::default())
                .unwrap()
                .data
                .len(),
            1
        );
        assert_eq!(
            ops::move_task(&mut store, &Actor::Person, "t1", Column::Doing)
                .unwrap_err()
                .code,
            COLUMN_IS_DERIVED
        );
        assert_eq!(
            ops::edit(
                &mut store,
                &Actor::Person,
                "t1",
                ops::Edit {
                    title: Some("new".into()),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
            GITHUB_OWNED_FIELD
        );
        store
            .connection
            .execute("UPDATE gh_cache SET assignees='[\"keith\"]'", [])
            .unwrap();
        assert_eq!(
            ops::list(&mut store, &actor, ops::List::default())
                .unwrap()
                .data[0]
                .column,
            Column::Doing
        );
        assert!(
            ops::ready(&mut store, &actor, ops::Ready::default())
                .unwrap()
                .data
                .is_empty()
        );
        let shown = ops::show(&mut store, &actor, "566").unwrap_err();
        assert_eq!(shown.code, UNKNOWN_TASK);
        let shown = ops::show(&mut store, &actor, "demo#566").unwrap().data;
        assert_eq!(shown.gh_cache.unwrap().assignees, ["keith"]);
        assert_eq!(shown.basecamp_card.unwrap().card_id, "card-1");
    });
}
#[test]
fn core_enforces_agent_rules_without_cli() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        add(&mut store, "work");
        let agent = Actor::Agent("test".into());
        for column in [Column::Done, Column::Shipped] {
            assert_eq!(
                ops::move_task(&mut store, &agent, "t1", column)
                    .unwrap_err()
                    .code,
                AGENT_CANNOT_FINISH
            );
        }
        assert_eq!(
            ops::archive(&mut store, &agent, "t1").unwrap_err().code,
            AGENT_CANNOT_ARCHIVE
        );
        assert_eq!(
            ops::unarchive(&mut store, &agent, "t1").unwrap_err().code,
            AGENT_CANNOT_ARCHIVE
        );
        assert_eq!(
            ops::project_archive(&mut store, &agent, "demo")
                .unwrap_err()
                .code,
            AGENT_CANNOT_MANAGE_PROJECTS
        );
    });
}
#[test]
fn storage_failure_rolls_back_the_entire_plan() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        store.connection.execute_batch("CREATE TRIGGER fail_task BEFORE INSERT ON tasks WHEN NEW.title='fail' BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
        let item = |title: &str| {
            Ok(ops::Add {
                project: "demo".into(),
                title: title.into(),
                notes: String::new(),
                priority: Priority(2),
                due: None,
                after: vec![],
                assignee: None,
                agent: false,
            })
        };
        let error = ops::add_plan(
            &mut store,
            &Actor::Person,
            vec![item("first"), item("fail")],
        )
        .unwrap_err();
        assert_eq!(error.exit_code, 3);
        assert!(
            ops::list(&mut store, &Actor::Person, ops::List::default())
                .unwrap()
                .data
                .is_empty()
        );
    });
}

#[test]
fn github_dependencies_use_derived_completion_without_deriving_blocked() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        store.connection.execute_batch("INSERT INTO tasks(project_id,kind,gh_number,title,position,created_at,updated_at) VALUES(1,'issue',566,'issue',1,'2026-11-01T00:00:00Z','2026-11-01T00:00:00Z'); INSERT INTO gh_cache(task_id,state,assignees,linked_pr_number,linked_pr_state,linked_pr_draft,url,fetched_at) VALUES(1,'OPEN','[]',567,'OPEN',0,'https://example.test/566','2026-11-01T00:00:00Z');").unwrap();
        add(&mut store, "local");
        ops::edit(
            &mut store,
            &Actor::Person,
            "t2",
            ops::Edit {
                after: Some(vec!["demo#566".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        ops::edit(
            &mut store,
            &Actor::Person,
            "t1",
            ops::Edit {
                after: Some(vec!["t2".into()]),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "t1")
                .unwrap()
                .data
                .column,
            Column::Review
        );
        assert!(
            ops::ready(&mut store, &Actor::Person, ops::Ready::default())
                .unwrap()
                .data
                .is_empty()
        );
        assert_eq!(
            ops::move_task(&mut store, &Actor::Person, "t2", Column::Doing)
                .unwrap_err()
                .code,
            AFTER_NOT_DONE
        );
        store
            .connection
            .execute(
                "UPDATE gh_cache SET state='CLOSED',linked_pr_state='MERGED'",
                [],
            )
            .unwrap();
        let ready = ops::ready(&mut store, &Actor::Person, ops::Ready::default())
            .unwrap()
            .data;
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, 2);
    });
}

// GitHub cards come from phase 3 sync; insert one directly to test the guards.
fn github_issue(store: &mut Store) {
    store
        .connection
        .execute_batch(
            "INSERT INTO tasks(project_id, kind, gh_number, title, position, created_at, updated_at)
             SELECT id, 'issue', 566, 'From GitHub', 1.0, '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z' FROM projects WHERE slug = 'demo';
             INSERT INTO gh_cache(task_id, state, assignees, url, fetched_at)
             SELECT id, 'OPEN', '[]', 'https://github.com/o/r/issues/566', '2026-11-01T00:00:00Z'
             FROM tasks WHERE gh_number = 566;",
        )
        .unwrap();
}
#[test]
fn github_cards_refuse_manual_moves_and_title_edits() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        github_issue(&mut store);
        let moved =
            ops::move_task(&mut store, &Actor::Person, "demo#566", Column::Doing).unwrap_err();
        assert_eq!(moved.code, COLUMN_IS_DERIVED, "{moved:?}");
        let edited = ops::edit(
            &mut store,
            &Actor::Person,
            "demo#566",
            ops::Edit {
                title: Some("Renamed".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(edited.code, GITHUB_OWNED_FIELD);
        let shown = ops::show(&mut store, &Actor::Person, "demo#566")
            .unwrap()
            .data;
        assert_eq!(shown.title, "From GitHub");
        assert_eq!(shown.column, Column::Todo);
    });
}

#[test]
fn first_wal_switch_waits_for_readers() {
    isolated(|path| {
        // A competing opener can retain a rollback-journal read lock after migration.
        drop(Store::open_at(path).unwrap());
        let db = Connection::open(path.join("punch.db")).unwrap();
        db.pragma_update(None, "journal_mode", "DELETE").unwrap();
        db.execute_batch("BEGIN; SELECT * FROM schema_version;")
            .unwrap();
        let root = path.to_owned();
        let (send, recv) = std::sync::mpsc::channel();
        let opener = std::thread::spawn(move || {
            send.send(Store::open_at(&root).map(|_| ())).unwrap();
        });
        assert!(matches!(
            recv.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        db.execute_batch("COMMIT").unwrap();
        recv.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        opener.join().unwrap();
    });
}

#[test]
fn concurrent_first_opens_serialize() {
    for _ in 0..100 {
        let dir = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let openers: Vec<_> = (0..8)
            .map(|_| {
                let root = dir.path().to_owned();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut store = Store::open_at(&root)?;
                    ops::project_add(
                        &mut store,
                        &Actor::Person,
                        ops::ProjectAdd {
                            slug: "demo".into(),
                            name: None,
                            github: None,
                            scope: None,
                        },
                    )
                })
            })
            .collect();
        for opener in openers {
            opener.join().unwrap().unwrap();
        }
    }
}

#[test]
fn add_at_max_task_id_returns_storage_refusal() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        add(&mut store, "last");
        store
            .connection
            .execute("UPDATE tasks SET id=?1", [i64::MAX])
            .unwrap();
        let error = ops::add(
            &mut store,
            &Actor::Person,
            ops::Add {
                project: "demo".into(),
                title: "next".into(),
                notes: String::new(),
                priority: Priority(2),
                due: None,
                after: vec![],
                assignee: None,
                agent: false,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "storage-failed");
        assert_eq!(error.exit_code, 3);
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    });
}

#[test]
fn archive_and_unarchive_bump_revision_only_on_change() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        assert_eq!(add(&mut store, "work").revision, 1);
        for (archived, revision) in [(true, 2), (false, 3)] {
            for changed in [true, false] {
                let before = store.connection.total_changes();
                let result = if archived {
                    ops::archive(&mut store, &Actor::Person, "t1")
                } else {
                    ops::unarchive(&mut store, &Actor::Person, "t1")
                }
                .unwrap();
                assert_eq!(result.changed, changed);
                assert_eq!(result.data.revision, revision);
                let persisted = ops::show(&mut store, &Actor::Person, "t1").unwrap().data;
                assert_eq!(persisted.revision, revision);
                assert_eq!(persisted.archived, archived);
                if !changed {
                    assert_eq!(store.connection.total_changes(), before);
                }
            }
        }
    });
}

#[test]
fn show_reads_only_the_referenced_task() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        add(&mut store, "wanted");
        add(&mut store, "unrelated");
        // Unrelated detail must not be decoded by a single-task read.
        store
            .connection
            .execute("UPDATE tasks SET notes=x'80' WHERE id=2", [])
            .unwrap();
        for reference in ["t1", "1"] {
            assert_eq!(
                ops::show(&mut store, &Actor::Person, reference)
                    .unwrap()
                    .data
                    .title,
                "wanted"
            );
        }
        github_issue(&mut store);
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "demo#566")
                .unwrap()
                .data
                .gh_number,
            Some(566)
        );
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "missing")
                .unwrap_err()
                .code,
            UNKNOWN_TASK
        );
    });
}

#[test]
fn ready_filters_and_limits_before_loading_details() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        add(&mut store, "blocked");
        add(&mut store, "wanted");
        add(&mut store, "beyond limit");
        store.connection.execute_batch("INSERT INTO task_after VALUES(1,3); UPDATE tasks SET notes=x'80' WHERE id IN (1,3);").unwrap();
        let ready = ops::ready(
            &mut store,
            &Actor::Person,
            ops::Ready {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap()
        .data;
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].title, "wanted");
        assert!(
            ops::ready(
                &mut store,
                &Actor::Person,
                ops::Ready {
                    limit: Some(0),
                    ..Default::default()
                }
            )
            .unwrap()
            .data
            .is_empty()
        );
    });
}

#[test]
fn ready_sql_matches_derived_columns_for_github_states() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        let mut number = 0;
        for kind in ["issue", "pr"] {
            for state in ["OPEN", "CLOSED", "MERGED"] {
                for linked in [None, Some("OPEN"), Some("CLOSED"), Some("MERGED")] {
                    for assigned in ["[]", "[\"keith\"]"] {
                        for draft in [false, true] {
                            for review in [None, Some("CHANGES_REQUESTED")] {
                                number += 1;
                                store.connection.execute("INSERT INTO tasks(project_id,kind,gh_number,title,position,created_at,updated_at) VALUES(1,?1,?2,'card',?2,'now','now')", rusqlite::params![kind,number]).unwrap();
                                let id = store.connection.last_insert_rowid();
                                store.connection.execute("INSERT INTO gh_cache(task_id,state,assignees,is_draft,review_decision,linked_pr_state,linked_pr_draft,linked_pr_review,url,fetched_at) VALUES(?1,?2,?3,?4,?5,?6,?4,?5,'url','now')", rusqlite::params![id,state,assigned,draft,review,linked]).unwrap();
                            }
                        }
                    }
                }
            }
        }
        let expected: Vec<_> = ops::list(
            &mut store,
            &Actor::Person,
            ops::List {
                all: true,
                ..Default::default()
            },
        )
        .unwrap()
        .data
        .into_iter()
        .filter(|t| t.column == Column::Todo)
        .map(|t| t.id)
        .collect();
        let actual: Vec<_> = ops::ready(&mut store, &Actor::Person, ops::Ready::default())
            .unwrap()
            .data
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(actual, expected);
    });
}

#[test]
fn read_ops_succeed_while_another_connection_holds_write_lock() {
    isolated(|path| {
        let mut writer = Store::open_at(path).unwrap();
        project(&mut writer);
        add(&mut writer, "committed");
        let mut reader = Store::open_at(path).unwrap();
        // Fail immediately if an operation attempts to acquire the writer lock.
        reader.connection.busy_timeout(Duration::ZERO).unwrap();
        let tx = writer
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("UPDATE tasks SET title='uncommitted' WHERE id=1", [])
            .unwrap();
        assert_eq!(
            ops::list(&mut reader, &Actor::Person, ops::List::default())
                .unwrap()
                .data[0]
                .title,
            "committed"
        );
        assert_eq!(
            ops::show(&mut reader, &Actor::Person, "t1")
                .unwrap()
                .data
                .title,
            "committed"
        );
        assert_eq!(
            ops::ready(&mut reader, &Actor::Person, ops::Ready::default())
                .unwrap()
                .data[0]
                .title,
            "committed"
        );
        assert_eq!(
            ops::project_list(&mut reader, &Actor::Person).unwrap().data[0].slug,
            "demo"
        );
        // Mutations must still serialize behind the writer.
        assert_eq!(
            ops::archive(&mut reader, &Actor::Person, "t1")
                .unwrap_err()
                .exit_code,
            3
        );
        tx.rollback().unwrap();
    });
}

#[test]
fn opening_current_store_does_not_wait_for_write_lock() {
    isolated(|path| {
        let mut writer = Store::open_at(path).unwrap();
        project(&mut writer);
        let tx = writer
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let start = Instant::now();
        let mut reader = Store::open_at(path).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(
            ops::project_list(&mut reader, &Actor::Person)
                .unwrap()
                .data
                .len(),
            1
        );
        tx.rollback().unwrap();
    });
}

#[test]
fn github_upsert_does_not_load_unrelated_tasks() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        add(&mut store, "unrelated");
        store
            .connection
            .execute("UPDATE tasks SET notes=x'80' WHERE id=1", [])
            .unwrap();
        let items: Vec<crate::github::Item> =
            serde_json::from_str(include_str!("../../tests/fixtures/github/open.json")).unwrap();
        for title in ["insert", "update"] {
            let mut item = items[0].clone();
            item.title = title.into();
            store
                .transaction(|repo| {
                    let p = repo.projects()?.remove(0);
                    repo.upsert_github(
                        &p,
                        &item,
                        &item.cache(None, "2026-11-01T00:00:00Z"),
                        "2026-11-01T00:00:00Z",
                        repo.next_position(p.id)?,
                    )
                })
                .unwrap();
            assert_eq!(
                ops::show(&mut store, &Actor::Person, "demo#1")
                    .unwrap()
                    .data
                    .title,
                title
            );
        }
    });
}

#[test]
fn data_version_changes_only_for_commits_from_another_connection() {
    isolated(|_| {
        let mut watcher = Store::open().unwrap();
        let before = watcher.data_version().unwrap();
        project(&mut watcher);
        assert_eq!(watcher.data_version().unwrap(), before);
        let mut writer = Store::open().unwrap();
        add(&mut writer, "external");
        assert_ne!(watcher.data_version().unwrap(), before);
    });
}

#[test]
fn revision_guard_refuses_stale_moves_edits_archives_and_resets_after_failure() {
    isolated(|_| {
        let mut store = Store::open().unwrap();
        project(&mut store);
        let snapshot = add(&mut store, "original");
        let mut external = Store::open().unwrap();
        ops::edit(
            &mut external,
            &Actor::Person,
            "t1",
            ops::Edit {
                notes: Some("external".into()),
                ..Default::default()
            },
        )
        .unwrap();
        for update in [
            ops::Update::Move(Column::Doing),
            ops::Update::Edit(ops::Edit {
                priority: Some(Priority(0)),
                ..Default::default()
            }),
            ops::Update::Archive,
        ] {
            let error =
                ops::update_task(&mut store, &Actor::Person, &snapshot, update).unwrap_err();
            assert_eq!(error.code, STALE);
        }
        let current = ops::show(&mut store, &Actor::Person, "t1").unwrap().data;
        assert_eq!(current.column, Column::Todo);
        assert_eq!(current.priority, Priority(2));
        assert!(!current.archived);
        assert_eq!(current.notes, "external");
        assert_eq!(current.revision, 2);
        let moved = ops::update_task(
            &mut store,
            &Actor::Person,
            &current,
            ops::Update::Move(Column::Doing),
        )
        .unwrap();
        assert!(moved.changed);
        assert_eq!(moved.data.revision, 3);
        // Existing CLI operations remain unconditional and the failed guard did not leak.
        ops::move_task(&mut store, &Actor::Person, "t1", Column::Review).unwrap();
    });
}

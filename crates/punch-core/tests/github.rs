#![allow(clippy::unwrap_used)]
use punch_core::{
    github::{Gh, Item},
    model::*,
    ops,
    store::Store,
};
use std::cell::RefCell;

struct Fake {
    items: RefCell<Vec<Item>>,
    links: Vec<Item>,
    failure: RefCell<Option<&'static str>>,
    scopes: RefCell<Vec<SyncScope>>,
}
impl Fake {
    fn new(items: Vec<Item>) -> Self {
        Self {
            items: RefCell::new(items),
            links: vec![],
            failure: RefCell::new(None),
            scopes: RefCell::new(vec![]),
        }
    }
    fn check(&self, stage: &str) -> Result<(), punch_core::error::PunchError> {
        if *self.failure.borrow() == Some(stage) {
            Err(punch_core::error::PunchError::refusal(
                "offline",
                "gh",
                "network connection failed: fixture stderr",
            ))
        } else {
            Ok(())
        }
    }
}
impl Gh for Fake {
    fn list(&self, _: &str, scope: SyncScope) -> Result<Vec<Item>, punch_core::error::PunchError> {
        self.check("list")?;
        self.scopes.borrow_mut().push(scope);
        Ok(self
            .items
            .borrow()
            .iter()
            .filter(|i| i.state == "OPEN")
            .cloned()
            .collect())
    }
    fn get(&self, _: &str, kind: Kind, n: i64) -> Result<Item, punch_core::error::PunchError> {
        self.check("get")?;
        Ok(self
            .items
            .borrow()
            .iter()
            .find(|i| i.kind == kind && i.number == n)
            .unwrap()
            .clone())
    }
    fn linked(&self, _: &str, issue: i64) -> Result<Vec<Item>, punch_core::error::PunchError> {
        self.check("linked")?;
        Ok(self
            .links
            .iter()
            .filter(|p| p.closes.contains(&issue))
            .cloned()
            .collect())
    }
    fn assign(
        &self,
        _: &str,
        number: i64,
        claim: bool,
    ) -> Result<(), punch_core::error::PunchError> {
        self.check("assign")?;
        self.items
            .borrow_mut()
            .iter_mut()
            .find(|i| i.number == number)
            .unwrap()
            .assignees = if claim {
            vec![punch_core::github::Login {
                login: "keith".into(),
            }]
        } else {
            vec![]
        };
        Ok(())
    }
}
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    // Explicit path avoids racing process-global environment variables in Rust tests.
    let mut store = Store::open_at(dir.path()).unwrap();
    ops::project_add(
        &mut store,
        &Actor::Person,
        ops::ProjectAdd {
            slug: "demo".into(),
            name: None,
            github: Some("owner/repo".into()),
            scope: Some(SyncScope::All),
        },
    )
    .unwrap();
    (dir, store)
}
#[test]
fn sync_inserts_github_cards_with_readable_timestamps_and_derived_columns() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        let out = ops::sync(&mut store, &Actor::Person, &gh, Some("demo")).unwrap();
        assert!(out.changed);
        let cards = ops::list(&mut store, &Actor::Person, ops::List::default())
            .unwrap()
            .data;
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].column, Column::Todo);
        assert_eq!(cards[1].column, Column::Review);
        assert!(!cards[0].created_at.is_empty());
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "demo#1")
                .unwrap()
                .data
                .column,
            Column::Todo
        );
        assert_eq!(
            ops::ready(&mut store, &Actor::Person, ops::Ready::default())
                .unwrap()
                .data
                .len(),
            1
        );
    });
}

#[test]
fn sync_refreshes_closed_and_merged_cards_without_touching_local_fields() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        ops::edit(
            &mut store,
            &Actor::Person,
            "demo#1",
            ops::Edit {
                notes: Some("keep".into()),
                priority: Some(Priority(0)),
                due: Some(Some("2026-12-01".into())),
                after: None,
                title: None,
            },
        )
        .unwrap();
        gh.items.borrow_mut()[0].state = "CLOSED".into();
        gh.items.borrow_mut()[1].state = "MERGED".into();
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        let issue = ops::show(&mut store, &Actor::Person, "demo#1")
            .unwrap()
            .data;
        assert_eq!(issue.column, Column::Done);
        assert_eq!(issue.notes, "keep");
        assert_eq!(issue.priority, Priority(0));
        assert_eq!(issue.due.as_deref(), Some("2026-12-01"));
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "demo#2")
                .unwrap()
                .data
                .column,
            Column::Shipped
        );
        assert!(
            ops::list(&mut store, &Actor::Person, ops::List::default())
                .unwrap()
                .data
                .is_empty()
        );
        let fetched = ops::show(&mut store, &Actor::Person, "demo#1")
            .unwrap()
            .data
            .gh_cache
            .unwrap()
            .fetched_at;
        // Nothing changed on GitHub: fetched_at refreshes, the sync reports no change.
        assert!(
            !ops::sync(&mut store, &Actor::Person, &gh, None)
                .unwrap()
                .changed
        );
        assert_ne!(
            ops::show(&mut store, &Actor::Person, "demo#1")
                .unwrap()
                .data
                .gh_cache
                .unwrap()
                .fetched_at,
            fetched
        );
    });
}

#[test]
fn local_claim_and_unclaim_are_idempotent_and_agents_use_their_slot() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let gh = Fake::new(vec![]);
        let args: ops::Add =
            serde_json::from_value(serde_json::json!({"project":"demo","title":"local"})).unwrap();
        ops::add(&mut store, &Actor::Person, args).unwrap();
        let actor = Actor::Agent("worker".into());
        let out = ops::claim(&mut store, &actor, &gh, "t1", None).unwrap();
        assert_eq!(out.data.assignee.as_deref(), Some("worker"));
        assert_eq!(out.data.slot.as_deref(), Some("worker"));
        assert_eq!(out.data.assignee_kind, Some(AssigneeKind::Agent));
        assert!(
            !ops::claim(&mut store, &actor, &gh, "t1", None)
                .unwrap()
                .changed
        );
        let out = ops::unclaim(&mut store, &actor, &gh, "t1").unwrap();
        assert!(out.changed);
        assert!(
            out.data.assignee.is_none()
                && out.data.slot.is_none()
                && out.data.assignee_kind.is_none()
        );
        assert!(!ops::unclaim(&mut store, &actor, &gh, "t1").unwrap().changed);
        let out = ops::claim(&mut store, &Actor::Person, &gh, "t1", Some("desk")).unwrap();
        assert_eq!(out.data.assignee.as_deref(), Some("person"));
        assert_eq!(out.data.slot.as_deref(), Some("desk"));
    });
}

#[test]
fn github_claim_refreshes_assignees_and_unclaim_clears_only_the_slot() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        for reference in ["demo#1", "demo#2"] {
            let out = ops::claim(
                &mut store,
                &Actor::Agent("worker".into()),
                &gh,
                reference,
                Some("explicit"),
            )
            .unwrap();
            assert_eq!(out.data.assignee.as_deref(), Some("keith"));
            assert_eq!(out.data.assignee_kind, Some(AssigneeKind::Person));
            assert_eq!(out.data.slot.as_deref(), Some("explicit"));
            assert_eq!(out.data.revision, 2);
            assert_eq!(out.data.gh_cache.unwrap().assignees, ["keith"]);
            let out = ops::unclaim(&mut store, &Actor::Person, &gh, reference).unwrap();
            assert!(out.data.assignee.is_none() && out.data.slot.is_none());
            assert!(out.data.gh_cache.unwrap().assignees.is_empty());
        }
    });
}
#[test]
fn github_write_or_refresh_failure_preserves_every_local_field_and_stderr() {
    isolated(|| {
        for stage in ["assign", "get", "linked"] {
            for unclaim in [false, true] {
                let (_dir, mut store) = setup();
                let gh = Fake::new(
                    serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap(),
                );
                ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
                if unclaim {
                    ops::claim(
                        &mut store,
                        &Actor::Agent("worker".into()),
                        &gh,
                        "demo#1",
                        None,
                    )
                    .unwrap();
                }
                let before = ops::show(&mut store, &Actor::Person, "demo#1")
                    .unwrap()
                    .data;
                *gh.failure.borrow_mut() = Some(stage);
                let err = if unclaim {
                    ops::unclaim(&mut store, &Actor::Person, &gh, "demo#1")
                } else {
                    ops::claim(&mut store, &Actor::Person, &gh, "demo#1", Some("new"))
                }
                .unwrap_err();
                assert_eq!(err.code, "offline");
                assert!(err.message.contains("fixture stderr"));
                assert_eq!(
                    ops::show(&mut store, &Actor::Person, "demo#1")
                        .unwrap()
                        .data,
                    before
                );
            }
        }
    });
}
#[test]
fn sync_scope_failures_and_archived_projects_are_reported_per_project() {
    isolated(|| {
        let (_dir, mut store) = setup();
        ops::project_add(
            &mut store,
            &Actor::Person,
            ops::ProjectAdd {
                slug: "assigned".into(),
                name: None,
                github: Some("other/repo".into()),
                scope: None,
            },
        )
        .unwrap();
        let gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        ops::sync(&mut store, &Actor::Agent("bot".into()), &gh, None).unwrap();
        assert_eq!(*gh.scopes.borrow(), [SyncScope::Assigned, SyncScope::All]);
        ops::project_archive(&mut store, &Actor::Person, "assigned").unwrap();
        *gh.failure.borrow_mut() = Some("linked");
        let before = ops::show(&mut store, &Actor::Person, "demo#1")
            .unwrap()
            .data;
        let out = ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        assert!(!out.changed);
        assert_eq!(out.data.len(), 1);
        assert_eq!(out.data[0].status, "failed");
        assert_eq!(out.data[0].refusals[0].code, "offline");
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "demo#1")
                .unwrap()
                .data,
            before
        );
    });
}

#[test]
fn every_derivation_rule_flows_through_sync_list_show_and_ready() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/github/derivation.json")).unwrap();
        let items: Vec<Item> = serde_json::from_value(fixture["items"].clone()).unwrap();
        let mut initial = items.clone();
        for i in &mut initial {
            i.state = "OPEN".into();
        }
        let mut gh = Fake::new(initial);
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        *gh.items.borrow_mut() = items;
        gh.links = serde_json::from_value(fixture["linked"].clone()).unwrap();
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        let cards = ops::list(
            &mut store,
            &Actor::Person,
            ops::List {
                all: true,
                ..Default::default()
            },
        )
        .unwrap()
        .data;
        assert_eq!(cards.len(), 14);
        for (card, expected) in cards.iter().zip(fixture["columns"].as_array().unwrap()) {
            assert_eq!(
                serde_json::to_value(card.column).unwrap(),
                *expected,
                "{}",
                card.reference()
            );
            assert_eq!(
                ops::show(&mut store, &Actor::Person, &card.reference())
                    .unwrap()
                    .data
                    .column,
                card.column
            );
        }
        let ready = ops::ready(&mut store, &Actor::Person, ops::Ready::default())
            .unwrap()
            .data;
        assert_eq!(
            ready
                .iter()
                .map(|t| t.gh_number.unwrap())
                .collect::<Vec<_>>(),
            [12, 13, 14]
        );
        ops::edit(
            &mut store,
            &Actor::Person,
            "demo#12",
            ops::Edit {
                title: None,
                notes: None,
                priority: None,
                due: None,
                after: Some(vec!["demo#11".into()]),
            },
        )
        .unwrap();
        assert_eq!(
            ops::ready(&mut store, &Actor::Person, ops::Ready::default())
                .unwrap()
                .data
                .len(),
            2
        );
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "demo#12")
                .unwrap()
                .data
                .column,
            Column::Todo
        );
    });
}

#[test]
fn linked_pr_selection_uses_latest_open_or_merged_closing_reference_and_deduplicates() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let mut gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        let mut older = gh.items.borrow()[1].clone();
        older.closes = vec![1];
        older.number = 20;
        older.state = "MERGED".into();
        older.updated_at = "2026-10-30T00:00:00Z".into();
        let mut latest = older.clone();
        latest.number = 21;
        latest.state = "OPEN".into();
        latest.updated_at = "2026-11-01T00:00:00Z".into();
        let mut closed = latest.clone();
        closed.number = 22;
        closed.state = "CLOSED".into();
        closed.updated_at = "2026-11-02T00:00:00Z".into();
        gh.links = vec![closed, latest.clone(), older];
        gh.items.borrow_mut().push(latest);
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        let out = ops::show(&mut store, &Actor::Person, "demo#1")
            .unwrap()
            .data;
        assert_eq!(out.gh_cache.unwrap().linked_pr_number, Some(21));
        assert_eq!(out.column, Column::Review);
        assert!(ops::show(&mut store, &Actor::Person, "demo#21").is_err());
        assert_eq!(
            ops::list(&mut store, &Actor::Person, ops::List::default())
                .unwrap()
                .data
                .len(),
            2
        );
        // A PR already stored before its issue was imported keeps its identity and metadata.
        gh.items.borrow_mut()[1].closes = vec![1];
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        assert!(ops::show(&mut store, &Actor::Person, "demo#2").is_ok());
    });
}

fn isolated(f: impl FnOnce()) {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("PUNCH_TEST_CASE").as_deref() == Ok(&name) {
        f();
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let gh = bin.join("gh");
    std::fs::write(&gh, include_str!("fixtures/github/gh")).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("PUNCH_TEST_CASE", &name)
        .env("PUNCH_STATE_DIR", dir.path())
        .env(
            "PUNCH_GH_FIXTURES",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/github"),
        )
        .env("PATH", std::env::join_paths(paths).unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn real_adapter_paginates_links_scopes_and_writes_through_fake_gh_on_path() {
    isolated(|| {
        use punch_core::github::RealGh;
        let all = RealGh.list("cli/cli", SyncScope::All).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].closes, [14895]);
        let assigned = RealGh.list("cli/cli", SyncScope::Assigned).unwrap();
        assert_eq!(assigned.len(), 0);
        let marker = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap())
            .join("viewer-is-author");
        std::fs::write(&marker, "").unwrap();
        assert_eq!(
            RealGh.list("cli/cli", SyncScope::Assigned).unwrap().len(),
            1
        );
        std::fs::remove_file(marker).unwrap();
        let linked = RealGh.linked("cli/cli", 14895).unwrap();
        assert_eq!(linked.len(), 2);
        assert_eq!(linked[0].number, 14846);
        let mut store = Store::open().unwrap();
        ops::project_add(
            &mut store,
            &Actor::Person,
            ops::ProjectAdd {
                slug: "cli".into(),
                name: None,
                github: Some("cli/cli".into()),
                scope: Some(SyncScope::All),
            },
        )
        .unwrap();
        ops::sync(&mut store, &Actor::Person, &RealGh, None).unwrap();
        assert!(ops::show(&mut store, &Actor::Person, "cli#14846").is_err());
        let claimed = ops::claim(
            &mut store,
            &Actor::Person,
            &RealGh,
            "cli#14895",
            Some("desk"),
        )
        .unwrap();
        assert_eq!(claimed.data.assignee.as_deref(), Some("keith"));
        assert_eq!(claimed.data.column, Column::Review);
        assert!(
            ops::unclaim(&mut store, &Actor::Person, &RealGh, "cli#14895")
                .unwrap()
                .data
                .assignee
                .is_none()
        );
        let err = RealGh.assign("bad/repo", 1, true).unwrap_err();
        assert_eq!(err.code, "gh-failed");
        assert!(err.message.contains("HTTP 403: fixture permission denied"));
        let calls = std::fs::read_to_string(
            std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap()).join("gh-calls"),
        )
        .unwrap();
        assert!(calls.contains("--assignee @me") && calls.contains("viewer { login }"));
        assert!(calls.contains("--add-assignee @me") && calls.contains("--remove-assignee @me"));
        assert!(calls.contains("cursor=next"));
    });
}

#[test]
fn sync_rolls_back_cards_inserted_before_a_later_refresh_failure() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        let before = ops::list(
            &mut store,
            &Actor::Person,
            ops::List {
                all: true,
                ..Default::default()
            },
        )
        .unwrap()
        .data;
        let mut new_pr = gh.items.borrow()[1].clone();
        new_pr.number = 4;
        gh.items.borrow_mut().push(new_pr);
        *gh.failure.borrow_mut() = Some("linked");
        let out = ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        assert_eq!(out.data[0].status, "failed");
        assert!(!out.changed);
        assert_eq!(
            ops::list(
                &mut store,
                &Actor::Person,
                ops::List {
                    all: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .data,
            before
        );
        assert!(ops::show(&mut store, &Actor::Person, "demo#4").is_err());
    });
}

// A sync that only refreshes fetched_at must not bump revisions: the TUI uses
// revision for stale checks and the Basecamp mirror keys off real changes.
#[test]
fn resync_without_github_changes_keeps_revision_and_reports_unchanged() {
    isolated(|| {
        let (_dir, mut store) = setup();
        let gh =
            Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
        ops::sync(&mut store, &Actor::Person, &gh, Some("demo")).unwrap();
        let before = ops::show(&mut store, &Actor::Person, "demo#1")
            .unwrap()
            .data;
        let again = ops::sync(&mut store, &Actor::Person, &gh, Some("demo")).unwrap();
        let after = ops::show(&mut store, &Actor::Person, "demo#1")
            .unwrap()
            .data;
        assert!(!again.changed);
        assert_eq!(after.revision, before.revision);
        assert_ne!(
            after.gh_cache.unwrap().fetched_at,
            before.gh_cache.unwrap().fetched_at
        );
    });
}

#[test]
fn slow_sync_keeps_reads_and_local_add_available() {
    isolated(|| {
        use punch_core::github::RealGh;
        use std::time::{Duration, Instant};
        let (dir, store) = setup();
        std::env::set_current_dir(dir.path()).unwrap();
        // The child gh signals after sync has reached the external boundary.
        let state = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap());
        std::fs::write(state.join("slow"), "").unwrap();
        let worker = std::thread::spawn(move || {
            let mut store = store;
            ops::sync(&mut store, &Actor::Person, &RealGh, None).unwrap()
        });
        let start = Instant::now();
        while !state.join("waiting").exists() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut second = Store::open_at(dir.path()).unwrap();
        let start = Instant::now();
        assert!(ops::list(&mut second, &Actor::Person, ops::List::default()).is_ok());
        let args =
            serde_json::from_value(serde_json::json!({"project":"demo","title":"during sync"}))
                .unwrap();
        let added = ops::add(&mut second, &Actor::Person, args);
        let elapsed = start.elapsed();
        std::fs::write(state.join("release"), "").unwrap();
        let result = worker.join().unwrap();
        assert!(added.is_ok(), "{added:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        assert_eq!(result.data[0].status, "synced");
    });
}

fn reject_refresh(value: serde_json::Value) {
    use punch_core::github::RealGh;
    let (_dir, mut store) = setup();
    let gh = Fake::new(serde_json::from_str(include_str!("fixtures/github/open.json")).unwrap());
    ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
    let before = ops::list(
        &mut store,
        &Actor::Person,
        ops::List {
            all: true,
            ..Default::default()
        },
    )
    .unwrap()
    .data;
    let state = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap());
    std::fs::write(state.join("view-response"), value.to_string()).unwrap();
    assert_eq!(
        RealGh.get("owner/repo", Kind::Issue, 1).unwrap_err().code,
        "gh-failed"
    );
    for unclaim in [false, true] {
        let error = if unclaim {
            ops::unclaim(&mut store, &Actor::Person, &RealGh, "demo#1")
        } else {
            ops::claim(
                &mut store,
                &Actor::Person,
                &RealGh,
                "demo#1",
                Some("worker"),
            )
        }
        .unwrap_err();
        assert_eq!(error.code, "gh-failed");
        assert_eq!(
            ops::list(
                &mut store,
                &Actor::Person,
                ops::List {
                    all: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .data,
            before
        );
    }
    assert_eq!(
        ops::sync(&mut store, &Actor::Person, &RealGh, None)
            .unwrap()
            .data[0]
            .status,
        "failed"
    );
    assert_eq!(
        ops::list(
            &mut store,
            &Actor::Person,
            ops::List {
                all: true,
                ..Default::default()
            }
        )
        .unwrap()
        .data,
        before
    );
}
fn refresh_value() -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(include_str!("fixtures/github/open.json")).unwrap()[0]
        .clone()
}
#[test]
fn refresh_rejects_wrong_number_without_local_changes() {
    isolated(|| {
        let mut value = refresh_value();
        value["number"] = 999.into();
        reject_refresh(value);
    });
}
#[test]
fn refresh_rejects_wrong_kind_without_local_changes() {
    isolated(|| {
        let mut value = refresh_value();
        value["kind"] = "pr".into();
        reject_refresh(value);
    });
}
#[test]
fn refresh_rejects_unknown_or_impossible_state_without_local_changes() {
    isolated(|| {
        for state in ["BOGUS", "MERGED", "open"] {
            let mut value = refresh_value();
            value["state"] = state.into();
            reject_refresh(value);
        }
    });
}

fn reject_cursors(mode: &str, message: &str) {
    use punch_core::github::RealGh;
    let state = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap());
    std::fs::write(state.join("cursor-mode"), mode).unwrap();
    let error = RealGh.linked("owner/repo", 1).unwrap_err();
    assert_eq!(error.code, "gh-failed");
    assert!(error.message.contains(message), "{error:?}");
}
#[test]
fn pagination_rejects_repeated_cursor() {
    isolated(|| reject_cursors("repeat", "repeated pagination cursor"));
}
#[test]
fn pagination_rejects_cyclic_cursor() {
    isolated(|| reject_cursors("cycle", "repeated pagination cursor"));
}
#[test]
fn pagination_caps_unique_pages() {
    isolated(|| reject_cursors("unique", "pagination page limit"));
}

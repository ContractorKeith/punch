#![allow(clippy::unwrap_used)]
use punch_core::{basecamp::Basecamp, model::*, ops, store::Store};
use std::{cell::RefCell, rc::Rc};

#[test]
fn link_is_person_only_and_status_shows_mapping() {
    isolated(|| {
        let mut store = Store::open().unwrap();
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
        .unwrap();
        let args = ops::BcLink {
            project: "demo".into(),
            bucket_id: "10".into(),
            card_table_id: "20".into(),
            column_map: [("todo".into(), "30".into())].into(),
        };
        let err = ops::bc_link(&mut store, &Actor::Agent("bot".into()), args.clone()).unwrap_err();
        assert_eq!(err.code, "agent-cannot-manage-projects");
        assert!(
            ops::bc_status(&mut store, &Actor::Person, None)
                .unwrap()
                .data
                .is_empty()
        );
        assert!(
            ops::bc_link(&mut store, &Actor::Person, args.clone())
                .unwrap()
                .changed
        );
        assert!(
            !ops::bc_link(&mut store, &Actor::Person, args)
                .unwrap()
                .changed
        );
        let status = ops::bc_status(&mut store, &Actor::Person, Some("demo")).unwrap();
        assert_eq!(status.data[0].link.column_map["todo"], "30");
        assert_eq!(status.data[0].link.bucket_id, "10");
    });
}

#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
    fail: RefCell<bool>,
    fail_title: RefCell<bool>,
}
impl Basecamp for Fake {
    fn create(&self, _link: &BasecampLink, column: &str, title: &str) -> Result<String, String> {
        self.calls
            .borrow_mut()
            .push(format!("create {column} {title}"));
        if *self.fail.borrow() {
            Err("fixture unavailable".into())
        } else {
            Ok("99".into())
        }
    }
    fn move_card(&self, _link: &BasecampLink, id: &str, column: &str) -> Result<(), String> {
        self.calls.borrow_mut().push(format!("move {id} {column}"));
        if *self.fail.borrow() {
            Err("fixture unavailable".into())
        } else {
            Ok(())
        }
    }
    fn update_title(&self, _link: &BasecampLink, id: &str, title: &str) -> Result<(), String> {
        self.calls.borrow_mut().push(format!("title {id} {title}"));
        if *self.fail.borrow() || *self.fail_title.borrow() {
            Err("fixture unavailable".into())
        } else {
            Ok(())
        }
    }
}
fn setup() -> (Store, Rc<Fake>) {
    let fake = Rc::new(Fake::default());
    let mut store = Store::open().unwrap().with_basecamp(fake.clone());
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
    .unwrap();
    ops::bc_link(
        &mut store,
        &Actor::Person,
        ops::BcLink {
            project: "demo".into(),
            bucket_id: "10".into(),
            card_table_id: "20".into(),
            column_map: [("todo".into(), "30".into()), ("doing".into(), "40".into())].into(),
        },
    )
    .unwrap();
    (store, fake)
}
fn add(store: &mut Store) -> Task {
    ops::add(
        store,
        &Actor::Person,
        ops::Add {
            project: "demo".into(),
            title: "Fix it".into(),
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
fn creates_card_and_moves_it_when_local_column_changes() {
    isolated(|| {
        let (mut store, fake) = setup();
        let t = add(&mut store);
        assert_eq!(fake.calls.borrow().as_slice(), ["create 30 t1 · Fix it"]);
        let out =
            ops::move_task(&mut store, &Actor::Person, &t.reference(), Column::Doing).unwrap();
        assert!(out.warnings.is_empty());
        assert!(fake.calls.borrow().contains(&"move 99 40".into()));
        assert_eq!(out.data.basecamp_card.unwrap().card_id, "99");
        let count = fake.calls.borrow().len();
        assert!(
            !ops::move_task(&mut store, &Actor::Person, "t1", Column::Doing)
                .unwrap()
                .changed
        );
        assert_eq!(fake.calls.borrow().len(), count);
    });
}

fn isolated(f: impl FnOnce()) {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("PUNCH_BC_TEST_CASE").as_deref() == Ok(&name) {
        f();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let script = bin.join("basecamp");
    std::fs::write(
        &script,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$PUNCH_STATE_DIR/calls"
printf 'END\n' >> "$PUNCH_STATE_DIR/calls"
if [ -f "$PUNCH_STATE_DIR/fail" ]; then
    echo 'fixture denied' >&2
    exit 1
fi
if [ -f "$PUNCH_STATE_DIR/bad-json" ]; then
    echo 'not JSON'
    exit 0
fi
if [ -f "$PUNCH_STATE_DIR/hang" ]; then
    echo $$ > "$PUNCH_STATE_DIR/waiting"
    while :; do sleep 1; done
fi
case "$2" in
create|update) echo '{"ok":true,"data":{"id":99,"title":"fixture"}}' ;;
move) echo '{"ok":true,"data":{"id":"99","status":"moved","column":"40"}}' ;;
*) exit 99 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("PUNCH_BC_TEST_CASE", &name)
        .env("PUNCH_STATE_DIR", dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

struct NoGh;
impl punch_core::github::Gh for NoGh {
    fn list(
        &self,
        _: &str,
        _: SyncScope,
    ) -> Result<Vec<punch_core::github::Item>, punch_core::error::PunchError> {
        panic!("local project must not call GitHub")
    }
    fn get(
        &self,
        _: &str,
        _: Kind,
        _: i64,
    ) -> Result<punch_core::github::Item, punch_core::error::PunchError> {
        panic!("unexpected GitHub read")
    }
    fn linked(
        &self,
        _: &str,
        _: i64,
    ) -> Result<Vec<punch_core::github::Item>, punch_core::error::PunchError> {
        panic!("unexpected GitHub read")
    }
    fn assign(&self, _: &str, _: i64, _: bool) -> Result<(), punch_core::error::PunchError> {
        panic!("unexpected GitHub write")
    }
}
#[test]
fn claim_unclaim_and_edit_keep_card_title_in_step() {
    isolated(|| {
        let (mut store, fake) = setup();
        add(&mut store);
        ops::claim(&mut store, &Actor::Agent("bot".into()), &NoGh, "t1", None).unwrap();
        assert!(
            fake.calls
                .borrow()
                .contains(&"title 99 t1 · bot · Fix it".into())
        );
        ops::unclaim(&mut store, &Actor::Agent("bot".into()), &NoGh, "t1").unwrap();
        assert_eq!(fake.calls.borrow().last().unwrap(), "title 99 t1 · Fix it");
        ops::edit(
            &mut store,
            &Actor::Person,
            "t1",
            ops::Edit {
                title: Some("New title".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            fake.calls.borrow().last().unwrap(),
            "title 99 t1 · New title"
        );
        let calls = fake.calls.borrow().len();
        ops::edit(
            &mut store,
            &Actor::Person,
            "t1",
            ops::Edit {
                priority: Some(Priority(0)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fake.calls.borrow().len(), calls);
    });
}
#[test]
fn failed_create_is_dirty_and_sync_retries_without_changing_revision() {
    isolated(|| {
        let (mut store, fake) = setup();
        *fake.fail.borrow_mut() = true;
        let t = add(&mut store);
        assert!(t.basecamp_card.as_ref().unwrap().dirty);
        assert_eq!(
            t.basecamp_card.as_ref().unwrap().last_error.as_deref(),
            Some("fixture unavailable")
        );
        assert_eq!(
            ops::bc_status(&mut store, &Actor::Person, None)
                .unwrap()
                .data[0]
                .dirty_cards
                .len(),
            1
        );
        *fake.fail.borrow_mut() = false;
        let out = ops::sync(&mut store, &Actor::Person, &NoGh, None).unwrap();
        assert!(out.changed);
        let fresh = ops::show(&mut store, &Actor::Person, "t1").unwrap().data;
        assert_eq!(fresh.revision, t.revision);
        assert_eq!(
            fresh.basecamp_card.unwrap(),
            BasecampCard {
                card_id: "99".into(),
                dirty: false,
                last_error: None
            }
        );
        assert!(
            !ops::sync(&mut store, &Actor::Person, &NoGh, None)
                .unwrap()
                .changed
        );
    });
}

#[test]
fn failures_warn_without_rolling_back_and_archived_tasks_are_skipped() {
    isolated(|| {
        let (mut store, fake) = setup();
        add(&mut store);
        *fake.fail.borrow_mut() = true;
        let moved = ops::move_task(&mut store, &Actor::Person, "t1", Column::Doing).unwrap();
        assert!(moved.changed);
        assert!(moved.warnings[0].starts_with("basecamp-failed: t1:"));
        assert_eq!(moved.data.column, Column::Doing);
        assert!(moved.data.basecamp_card.unwrap().dirty);
        let failed_sync = ops::sync(&mut store, &Actor::Person, &NoGh, None).unwrap();
        assert!(failed_sync.warnings[0].starts_with("basecamp-failed:"));
        ops::archive(&mut store, &Actor::Person, "t1").unwrap();
        let count = fake.calls.borrow().len();
        ops::sync(&mut store, &Actor::Person, &NoGh, None).unwrap();
        ops::edit(
            &mut store,
            &Actor::Person,
            "t1",
            ops::Edit {
                title: Some("Archived title".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fake.calls.borrow().len(), count);
    });
}
#[test]
fn unmapped_column_warns_and_makes_no_external_call() {
    isolated(|| {
        let (mut store, fake) = setup();
        add(&mut store);
        let calls = fake.calls.borrow().len();
        let moved = ops::move_task(&mut store, &Actor::Person, "t1", Column::Review).unwrap();
        assert_eq!(moved.warnings, ["basecamp-unmapped: t1: review"]);
        assert_eq!(fake.calls.borrow().len(), calls);
        let sync = ops::sync(&mut store, &Actor::Person, &NoGh, None).unwrap();
        assert_eq!(sync.warnings, ["basecamp-unmapped: t1: review"]);
    });
}
#[test]
fn real_adapter_passes_exact_argv_and_parses_card_json() {
    isolated(|| {
        use punch_core::basecamp::RealBasecamp;
        let link = BasecampLink {
            project_id: 1,
            bucket_id: "10".into(),
            card_table_id: "20".into(),
            column_map: Default::default(),
        };
        let bc = RealBasecamp;
        let id = bc.create(&link, "30", "t1 · bot · --Fix $it").unwrap();
        assert_eq!(id, "99");
        bc.move_card(&link, &id, "40").unwrap();
        bc.update_title(&link, &id, "t1 · New title").unwrap();
        let root = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap());
        let calls = std::fs::read_to_string(root.join("calls")).unwrap();
        assert_eq!(
            calls,
            concat!(
                "cards\ncreate\n--project\n10\n--card-table\n20\n--column\n30\n--json\n--\nt1 · bot · --Fix $it\nEND\n",
                "cards\nmove\n99\n--project\n10\n--card-table\n20\n--to\n40\n--json\nEND\n",
                "cards\nupdate\n99\n--project\n10\n--card-table\n20\n--title\nt1 · New title\n--json\nEND\n"
            )
        );
        std::fs::write(root.join("fail"), "").unwrap();
        assert!(
            bc.update_title(&link, &id, "x")
                .unwrap_err()
                .contains("fixture denied")
        );
        std::fs::remove_file(root.join("fail")).unwrap();
        std::fs::write(root.join("bad-json"), "").unwrap();
        assert!(bc.create(&link, "30", "x").is_err());
    });
}

#[test]
fn relinking_to_another_table_creates_new_cards_without_reusing_old_ids() {
    isolated(|| {
        let (mut store, fake) = setup();
        add(&mut store);
        ops::bc_link(
            &mut store,
            &Actor::Person,
            ops::BcLink {
                project: "demo".into(),
                bucket_id: "11".into(),
                card_table_id: "21".into(),
                column_map: [("todo".into(), "31".into())].into(),
            },
        )
        .unwrap();
        fake.calls.borrow_mut().clear();
        ops::sync(&mut store, &Actor::Person, &NoGh, None).unwrap();
        assert_eq!(fake.calls.borrow().as_slice(), ["create 31 t1 · Fix it"]);
    });
}

struct GhFixture(RefCell<punch_core::github::Item>);
impl punch_core::github::Gh for GhFixture {
    fn list(
        &self,
        _: &str,
        _: SyncScope,
    ) -> Result<Vec<punch_core::github::Item>, punch_core::error::PunchError> {
        Ok(vec![self.0.borrow().clone()])
    }
    fn get(
        &self,
        _: &str,
        _: Kind,
        _: i64,
    ) -> Result<punch_core::github::Item, punch_core::error::PunchError> {
        Ok(self.0.borrow().clone())
    }
    fn linked(
        &self,
        _: &str,
        _: i64,
    ) -> Result<Vec<punch_core::github::Item>, punch_core::error::PunchError> {
        Ok(vec![])
    }
    fn assign(&self, _: &str, _: i64, claim: bool) -> Result<(), punch_core::error::PunchError> {
        self.0.borrow_mut().assignees = if claim {
            vec![punch_core::github::Login {
                login: "keith".into(),
            }]
        } else {
            vec![]
        };
        Ok(())
    }
}
#[test]
fn github_refresh_and_claim_mirror_derived_columns_titles_and_slots() {
    isolated(|| {
        let fake = Rc::new(Fake::default());
        let mut store = Store::open().unwrap().with_basecamp(fake.clone());
        ops::project_add(
            &mut store,
            &Actor::Person,
            ops::ProjectAdd {
                slug: "demo".into(),
                name: None,
                github: Some("example/demo".into()),
                scope: None,
            },
        )
        .unwrap();
        ops::bc_link(
            &mut store,
            &Actor::Person,
            ops::BcLink {
                project: "demo".into(),
                bucket_id: "10".into(),
                card_table_id: "20".into(),
                column_map: [("todo".into(), "30".into()), ("doing".into(), "40".into())].into(),
            },
        )
        .unwrap();
        let gh = GhFixture(RefCell::new(
            serde_json::from_value(serde_json::json!({
                "kind":"issue","number":7,"title":"Remote title","state":"OPEN","assignees":[],
                "url":"https://example.invalid/7","updatedAt":"2026-11-01T00:00:00Z",
                "author":null,"isDraft":null,"reviewDecision":null
            }))
            .unwrap(),
        ));
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        assert_eq!(
            fake.calls.borrow().as_slice(),
            ["create 30 #7 · Remote title"]
        );
        let initial = ops::show(&mut store, &Actor::Person, "demo#7")
            .unwrap()
            .data;
        let unchanged = ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        let fresh = ops::show(&mut store, &Actor::Person, "demo#7")
            .unwrap()
            .data;
        assert!(!unchanged.changed);
        assert_eq!(fresh.revision, initial.revision);
        assert_ne!(
            fresh.gh_cache.unwrap().fetched_at,
            initial.gh_cache.unwrap().fetched_at
        );
        ops::claim(&mut store, &Actor::Agent("bot".into()), &gh, "demo#7", None).unwrap();
        assert!(fake.calls.borrow().contains(&"move 99 40".into()));
        assert_eq!(
            fake.calls.borrow().last().unwrap(),
            "title 99 #7 · bot · Remote title"
        );
        ops::unclaim(&mut store, &Actor::Agent("bot".into()), &gh, "demo#7").unwrap();
        assert_eq!(
            fake.calls.borrow().last().unwrap(),
            "title 99 #7 · Remote title"
        );
        gh.0.borrow_mut().title = "Renamed".into();
        gh.0.borrow_mut().assignees = vec![punch_core::github::Login {
            login: "keith".into(),
        }];
        let refreshed = ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        assert!(refreshed.changed);
        assert_eq!(fake.calls.borrow().last().unwrap(), "title 99 #7 · Renamed");
        ops::archive(&mut store, &Actor::Person, "demo#7").unwrap();
        fake.calls.borrow_mut().clear();
        gh.0.borrow_mut().title = "Archived remote".into();
        ops::sync(&mut store, &Actor::Person, &gh, None).unwrap();
        assert!(fake.calls.borrow().is_empty());
    });
}
#[test]
fn sync_filter_and_archived_projects_do_not_touch_other_mirrors() {
    isolated(|| {
        let (mut store, fake) = setup();
        add(&mut store);
        ops::project_add(
            &mut store,
            &Actor::Person,
            ops::ProjectAdd {
                slug: "other".into(),
                name: None,
                github: None,
                scope: None,
            },
        )
        .unwrap();
        fake.calls.borrow_mut().clear();
        ops::sync(&mut store, &Actor::Person, &NoGh, Some("other")).unwrap();
        assert!(fake.calls.borrow().is_empty());
        ops::project_archive(&mut store, &Actor::Person, "demo").unwrap();
        ops::sync(&mut store, &Actor::Person, &NoGh, Some("demo")).unwrap();
        assert!(fake.calls.borrow().is_empty());
    });
}

#[test]
fn failed_title_update_retains_card_id_and_retries_the_current_slot() {
    isolated(|| {
        let (mut store, fake) = setup();
        add(&mut store);
        *fake.fail_title.borrow_mut() = true;
        let out = ops::claim(&mut store, &Actor::Agent("bot".into()), &NoGh, "t1", None).unwrap();
        assert_eq!(out.data.slot.as_deref(), Some("bot"));
        let card = out.data.basecamp_card.unwrap();
        assert!(card.dirty);
        assert_eq!(card.card_id, "99");
        assert!(out.warnings[0].starts_with("basecamp-failed:"));
        *fake.fail_title.borrow_mut() = false;
        ops::sync(&mut store, &Actor::Person, &NoGh, None).unwrap();
        let fresh = ops::show(&mut store, &Actor::Person, "t1").unwrap().data;
        assert!(!fresh.basecamp_card.unwrap().dirty);
        assert_eq!(
            fake.calls.borrow().last().unwrap(),
            "title 99 t1 · bot · Fix it"
        );
    });
}
#[test]
fn plan_mirrors_valid_new_tasks_once_and_returns_card_state_for_existing_items() {
    isolated(|| {
        let (mut store, fake) = setup();
        let args: ops::Add =
            serde_json::from_value(serde_json::json!({"project":"demo","title":"Plan item"}))
                .unwrap();
        let out = ops::add_plan(
            &mut store,
            &Actor::Person,
            vec![
                Ok(args.clone()),
                Ok(args),
                Err(punch_core::error::PunchError::usage("bad item")),
            ],
        )
        .unwrap();
        assert!(out.changed);
        assert_eq!(fake.calls.borrow().as_slice(), ["create 30 t1 · Plan item"]);
        let json = serde_json::to_value(out).unwrap();
        assert_eq!(json["data"][0]["status"], "created");
        assert_eq!(json["data"][1]["status"], "existing");
        assert_eq!(json["data"][2]["status"], "failed");
        assert_eq!(json["data"][0]["task"]["basecamp_card"]["card_id"], "99");
        assert_eq!(json["data"][1]["task"]["basecamp_card"]["card_id"], "99");
    });
}

#[test]
fn hung_basecamp_commits_locally_keeps_store_available_and_times_out() {
    isolated(|| {
        use std::time::{Duration, Instant};
        let (store, _) = setup();
        drop(store);
        let root = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap());
        std::fs::write(root.join("hang"), "").unwrap();
        let worker = std::thread::spawn(|| {
            let mut store = Store::open().unwrap();
            let start = Instant::now();
            let task = add(&mut store);
            assert!(start.elapsed() >= Duration::from_secs(30));
            assert!(start.elapsed() < Duration::from_secs(35));
            task
        });
        let start = Instant::now();
        while !root.join("waiting").exists() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut other = Store::open().unwrap();
        let task = ops::show(&mut other, &Actor::Person, "t1").unwrap().data;
        assert_eq!(task.title, "Fix it");
        ops::note(&mut other, &Actor::Person, "t1", "concurrent note").unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        let task = worker.join().unwrap();
        let card = task.basecamp_card.unwrap();
        assert!(card.dirty);
        assert!(card.last_error.unwrap().contains("timeout"));
        let pid = std::fs::read_to_string(root.join("waiting")).unwrap();
        assert!(
            !std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .status()
                .unwrap()
                .success()
        );
    });
}

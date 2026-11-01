#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn run(dir: &TempDir, args: &[&str]) -> assert_cmd::assert::Assert {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("punch").unwrap();
    cmd.env("PUNCH_STATE_DIR", dir.path())
        .env_remove("PUNCH_AGENT")
        .args(args)
        .assert()
}

#[test]
fn project_add_is_idempotent_and_json_has_exact_envelope() {
    let dir = TempDir::new().unwrap();
    let first = run(&dir, &["--json", "project", "add", "demo"])
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 4);
    assert_eq!(value["ok"], true);
    assert_eq!(value["changed"], true);
    assert_eq!(value["data"]["slug"], "demo");
    assert_eq!(value["warnings"], serde_json::json!([]));
    let second = run(&dir, &["project", "add", "demo", "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&second).unwrap()["changed"],
        false
    );
}

fn json(dir: &TempDir, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    serde_json::from_slice(&run(dir, &all).success().get_output().stdout).unwrap()
}
fn setup() -> TempDir {
    let dir = TempDir::new().unwrap();
    json(&dir, &["project", "add", "demo"]);
    dir
}
#[test]
fn add_retries_return_existing_without_changing_fields_or_revision() {
    let dir = setup();
    let first = json(
        &dir,
        &["add", "Build it", "-p", "demo", "--notes", "original"],
    );
    assert_eq!(first["data"]["existing"], false);
    let retry = json(
        &dir,
        &["add", "Build it", "-p", "demo", "--notes", "ignored"],
    );
    assert_eq!(retry["changed"], false);
    assert_eq!(retry["data"]["existing"], true);
    assert_eq!(retry["data"]["task"], first["data"]["task"]);
    let show = json(&dir, &["show", "t1"]);
    assert_eq!(show["data"]["notes"], "original");
    assert_eq!(show["data"]["revision"], 1);
}

fn refusal(dir: &TempDir, args: &[&str], code: &str) {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = run(dir, &all).code(1).get_output().stdout.clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.as_object().unwrap().len(), 2);
    assert_eq!(v["ok"], false);
    assert_eq!(v["refusals"][0]["code"], code);
    assert!(v["refusals"][0]["ref"].is_string());
    assert!(v["refusals"][0]["message"].is_string());
}
#[test]
fn move_and_edit_observe_dependencies_cycles_and_noops() {
    let dir = setup();
    json(&dir, &["add", "first", "-p", "demo"]);
    json(&dir, &["add", "second", "-p", "demo", "--after", "t1"]);
    refusal(&dir, &["move", "t2", "doing"], "after-not-done");
    refusal(&dir, &["edit", "t1", "--after", "t2"], "after-cycle");
    refusal(&dir, &["edit", "t1", "--after", "t1"], "after-self");
    assert_eq!(json(&dir, &["show", "1"])["data"]["revision"], 1);
    assert_eq!(json(&dir, &["move", "t1", "done"])["data"]["revision"], 2);
    assert_eq!(json(&dir, &["move", "t2", "doing"])["changed"], true);
    assert_eq!(json(&dir, &["move", "t2", "doing"])["changed"], false);
    assert_eq!(
        json(
            &dir,
            &["edit", "t2", "--title", "second", "--after", "t1,t1"]
        )["changed"],
        false
    );
    assert_eq!(
        json(
            &dir,
            &["edit", "t2", "--clear-after", "--due", "2028-02-29"]
        )["data"]["revision"],
        3
    );
    assert_eq!(
        json(&dir, &["edit", "t2", "--clear-due"])["data"]["due"],
        Value::Null
    );
}
#[test]
fn agent_rules_are_enforced_and_notes_append_actor_and_utc() {
    let dir = setup();
    json(&dir, &["add", "first", "-p", "demo"]);
    for column in ["done", "shipped"] {
        refusal(
            &dir,
            &["--as", "bot", "move", "t1", column],
            "agent-cannot-finish",
        );
    }
    for command in ["archive", "unarchive"] {
        refusal(
            &dir,
            &["--as", "bot", command, "t1"],
            "agent-cannot-archive",
        );
    }
    refusal(
        &dir,
        &["--as", "bot", "project", "add", "new"],
        "agent-cannot-manage-projects",
    );
    refusal(
        &dir,
        &["--as", "bot", "project", "archive", "demo"],
        "agent-cannot-manage-projects",
    );
    for column in ["doing", "review", "blocked", "todo"] {
        json(&dir, &["--as", "bot", "move", "t1", column]);
    }
    let n = json(&dir, &["--as", "bot", "note", "t1", "hello"]);
    let text = n["data"]["notes"].as_str().unwrap();
    assert!(text.starts_with("\n["));
    assert!(text.ends_with("Z bot] hello"));
    assert_eq!(json(&dir, &["archive", "t1"])["changed"], true);
    assert_eq!(json(&dir, &["archive", "t1"])["changed"], false);
    assert_eq!(json(&dir, &["list"])["data"], serde_json::json!([]));
    assert_eq!(
        json(&dir, &["list", "--all"])["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(json(&dir, &["unarchive", "t1"])["changed"], true);
}
#[test]
fn ready_orders_priority_due_then_position_and_excludes_unfinished_dependencies() {
    let dir = setup();
    for args in [
        vec![
            "add",
            "later",
            "-p",
            "demo",
            "--priority",
            "1",
            "--due",
            "2027-02-01",
        ],
        vec![
            "add",
            "first",
            "-p",
            "demo",
            "--priority",
            "1",
            "--due",
            "2027-01-01",
        ],
        vec!["add", "undated", "-p", "demo", "--priority", "1"],
        vec!["add", "urgent", "-p", "demo", "--priority", "0"],
        vec![
            "add",
            "blocked",
            "-p",
            "demo",
            "--priority",
            "0",
            "--after",
            "t1",
        ],
        vec![
            "add",
            "tie",
            "-p",
            "demo",
            "--priority",
            "1",
            "--due",
            "2027-01-01",
        ],
    ] {
        json(&dir, &args);
    }
    let v = json(&dir, &["ready"]);
    let titles: Vec<_> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["urgent", "first", "tie", "later", "undated"]);
    assert_eq!(
        json(&dir, &["ready", "--limit", "2"])["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    json(&dir, &["move", "t1", "shipped"]);
    assert_eq!(
        json(&dir, &["ready", "--limit", "2"])["data"][1]["title"],
        "blocked"
    );
}
#[test]
fn plan_commits_valid_items_and_reports_failed_items_without_partial_rows() {
    let dir = setup();
    let path = dir.path().join("plan.json");
    std::fs::write(
        &path,
        r#"[
      {"project":"demo","title":"first"},
      {"project":"demo","title":"bad","after":["t999"]},
      {"project":"demo","title":"second","after":["t1"]},
      {"project":"demo","title":"first"},
      {"project":"demo","title":"invalid","priority":9}
    ]"#,
    )
    .unwrap();
    let out = run(&dir, &["--json", "add", "--file", path.to_str().unwrap()])
        .code(1)
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["changed"], true);
    let statuses: Vec<_> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        ["created", "failed", "created", "existing", "failed"]
    );
    assert_eq!(v["data"][1]["refusals"][0]["code"], "unknown-task");
    assert_eq!(json(&dir, &["list"])["data"].as_array().unwrap().len(), 2);
    let second = run(&dir, &["--json", "add", "--file", path.to_str().unwrap()])
        .code(1)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&second).unwrap()["changed"],
        false
    );
}
#[test]
fn usage_errors_are_json_and_never_change_tasks() {
    let dir = setup();
    for args in [
        vec!["--json", "add"],
        vec!["--json", "add", "x", "-p", "demo", "--priority", "4"],
        vec!["--json", "move", "t1", "invalid"],
        vec!["--json", "add", "x", "-p", "demo", "--due", "2027-02-29"],
        vec!["--json", "edit", "t1", "--due", "2027-01-01", "--clear-due"],
    ] {
        let out = run(&dir, &args).code(2).get_output().clone();
        assert!(out.stderr.is_empty());
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["refusals"][0]["code"], "invalid-input");
    }
    assert_eq!(json(&dir, &["list"])["data"], serde_json::json!([]));
}
#[test]
fn unknown_references_projects_and_cross_project_dependencies_refuse() {
    let dir = setup();
    refusal(&dir, &["show", "nonsense"], "unknown-task");
    refusal(
        &dir,
        &["--as", "bot", "archive", "nonsense"],
        "unknown-task",
    );
    refusal(&dir, &["add", "x", "-p", "missing"], "unknown-project");
    refusal(&dir, &["list", "-p", "missing"], "unknown-project");
    refusal(
        &dir,
        &["project", "add", "demo", "--name", "Different"],
        "project-exists",
    );
    json(&dir, &["add", "first", "-p", "demo"]);
    json(&dir, &["project", "add", "other"]);
    refusal(
        &dir,
        &["add", "other", "-p", "other", "--after", "t1"],
        "after-cross-project",
    );
    refusal(
        &dir,
        &["add", "self", "-p", "demo", "--after", "t2"],
        "after-self",
    );
    assert_eq!(json(&dir, &["list"])["data"].as_array().unwrap().len(), 1);
}
#[test]
fn environment_actor_is_enforced_and_explicit_as_overrides_it() {
    let dir = setup();
    json(&dir, &["add", "first", "-p", "demo"]);
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("punch").unwrap();
    cmd.env("PUNCH_STATE_DIR", dir.path())
        .env("PUNCH_AGENT", "from-env")
        .args(["--json", "move", "t1", "done"])
        .assert()
        .code(1);
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("punch").unwrap();
    let out = cmd
        .env("PUNCH_STATE_DIR", dir.path())
        .env("PUNCH_AGENT", "from-env")
        .args(["--as", "explicit", "--json", "note", "t1", "hello"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        serde_json::from_slice::<Value>(&out).unwrap()["data"]["notes"]
            .as_str()
            .unwrap()
            .contains(" explicit] hello")
    );
}
#[test]
fn json_help_and_read_help_do_not_open_store() {
    let dir = TempDir::new().unwrap();
    run(&dir, &["--json"]).success();
    for command in [
        vec!["list", "--help"],
        vec!["show", "--help"],
        vec!["ready", "--help"],
        vec!["project", "list", "--help"],
    ] {
        let out = run(&dir, &command).success().get_output().stdout.clone();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("agents should pass --json")
        );
    }
    assert!(!dir.path().join("punch.db").exists());
}
#[test]
fn agents_can_append_notes_but_cannot_replace_notes_or_titles() {
    let dir = setup();
    json(&dir, &["add", "first", "-p", "demo", "--notes", "original"]);
    refusal(
        &dir,
        &["--as", "bot", "edit", "t1", "--title", "changed"],
        "agent-cannot-edit",
    );
    refusal(
        &dir,
        &["--as", "bot", "edit", "t1", "--notes", "replacement"],
        "agent-cannot-edit",
    );
    let updated = json(
        &dir,
        &[
            "--as",
            "bot",
            "edit",
            "t1",
            "--priority",
            "0",
            "--due",
            "2027-01-01",
        ],
    );
    assert_eq!(updated["data"]["title"], "first");
    assert_eq!(updated["data"]["notes"], "original");
    assert_eq!(updated["data"]["revision"], 2);
}
#[test]
fn storage_failures_and_too_new_store_exit_three_with_json() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("punch.db"), "corrupt database").unwrap();
    let out = run(&dir, &["--json", "list"])
        .code(3)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&out).unwrap()["refusals"][0]["code"],
        "storage-failed"
    );
    let dir = TempDir::new().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("punch.db")).unwrap();
    db.execute_batch(include_str!(
        "../../punch-core/src/store/fixtures/newer.sql"
    ))
    .unwrap();
    drop(db);
    let out = run(&dir, &["--json", "list"])
        .code(3)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&out).unwrap()["refusals"][0]["code"],
        "store-too-new"
    );
}
#[test]
fn stdin_plan_reports_created_existing_and_is_json_only() {
    let dir = setup();
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("punch").unwrap();
    let out = cmd
        .env("PUNCH_STATE_DIR", dir.path())
        .env_remove("PUNCH_AGENT")
        .args(["add", "--file", "-", "--json"])
        .write_stdin(r#"[{"project":"demo","title":"one"},{"project":"demo","title":"one"}]"#)
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(out.stderr.is_empty());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"][0]["status"], "created");
    assert_eq!(v["data"][1]["status"], "existing");
}
#[test]
fn project_archive_hides_tasks_and_does_not_rewrite_them() {
    let dir = setup();
    json(&dir, &["add", "first", "-p", "demo"]);
    assert_eq!(json(&dir, &["project", "archive", "demo"])["changed"], true);
    assert_eq!(
        json(&dir, &["project", "archive", "demo"])["changed"],
        false
    );
    assert_eq!(json(&dir, &["list"])["data"], serde_json::json!([]));
    assert_eq!(json(&dir, &["ready"])["data"], serde_json::json!([]));
    assert_eq!(json(&dir, &["list", "--all"])["data"][0]["revision"], 1);
}

#[test]
fn simultaneous_add_retries_create_one_task() {
    let dir = setup();
    let children: Vec<_> = (0..6)
        .map(|_| {
            std::process::Command::new(assert_cmd::cargo::cargo_bin!("punch"))
                .env("PUNCH_STATE_DIR", dir.path())
                .env_remove("PUNCH_AGENT")
                .args(["--json", "add", "one", "-p", "demo"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut created = 0;
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        if v["changed"] == true {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    assert_eq!(json(&dir, &["list"])["data"].as_array().unwrap().len(), 1);
}
#[test]
fn malformed_plan_is_usage_error_and_preserves_tasks() {
    let dir = setup();
    for text in ["not json", r#"{"project":"demo","title":"not an array"}"#] {
        #[allow(deprecated)]
        let mut cmd = Command::cargo_bin("punch").unwrap();
        let out = cmd
            .env("PUNCH_STATE_DIR", dir.path())
            .env_remove("PUNCH_AGENT")
            .args(["--json", "add", "--file", "-"])
            .write_stdin(text)
            .assert()
            .code(2)
            .get_output()
            .stdout
            .clone();
        assert_eq!(
            serde_json::from_slice::<Value>(&out).unwrap()["refusals"][0]["code"],
            "invalid-input"
        );
    }
    assert_eq!(json(&dir, &["list"])["data"], serde_json::json!([]));
}
#[test]
fn local_assignee_identity_and_archive_control_add_idempotence() {
    let dir = setup();
    assert_eq!(
        json(
            &dir,
            &["add", "same", "-p", "demo", "--assignee", "bot", "--agent"]
        )["data"]["task"]["assignee_kind"],
        "agent"
    );
    let existing = json(&dir, &["add", "same", "-p", "demo", "--assignee", "bot"]);
    assert_eq!(existing["data"]["existing"], true);
    assert_eq!(existing["data"]["task"]["slot"], "bot");
    assert_eq!(
        json(&dir, &["add", "same", "-p", "demo", "--assignee", "keith"])["data"]["existing"],
        false
    );
    json(&dir, &["archive", "t1"]);
    assert_eq!(
        json(
            &dir,
            &["add", "same", "-p", "demo", "--assignee", "bot", "--agent"]
        )["data"]["existing"],
        false
    );
}

#[test]
fn claim_unclaim_and_sync_are_available_with_json_envelopes() {
    let dir = setup();
    json(&dir, &["add", "local", "-p", "demo"]);
    let claimed = json(&dir, &["--as", "worker", "claim", "t1", "--slot", "slot-a"]);
    assert_eq!(claimed["data"]["slot"], "slot-a");
    assert_eq!(claimed["data"]["assignee"], "worker");
    assert_eq!(
        json(&dir, &["unclaim", "t1"])["data"]["slot"],
        serde_json::Value::Null
    );
    assert_eq!(json(&dir, &["sync"])["data"], serde_json::json!([]));
    refusal(&dir, &["sync", "-p", "missing"], "unknown-project");
    refusal(&dir, &["claim", "t999"], "unknown-task");
}

#[test]
fn github_cli_reports_partial_sync_and_claim_failures_without_local_changes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = setup();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let script = bin.join("gh");
    std::fs::write(
        &script,
        include_str!("../../punch-core/tests/fixtures/github/gh"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let paths = std::env::join_paths(paths).unwrap();
    let github_run = |args: &[&str], failure: &str| {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("punch"));
        cmd.env("PUNCH_STATE_DIR", dir.path())
            .env_remove("PUNCH_AGENT")
            .env("PATH", &paths)
            .env("PUNCH_GH_FAILURE", failure)
            .env(
                "PUNCH_GH_FIXTURES",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../punch-core/tests/fixtures/github"
                ),
            )
            .arg("--json")
            .args(args)
            .assert()
    };
    json(
        &dir,
        &[
            "project", "add", "cli", "--github", "cli/cli", "--scope", "all",
        ],
    );
    json(&dir, &["project", "add", "bad", "--github", "bad/repo"]);
    let out = github_run(&["sync"], "").code(3).get_output().clone();
    assert!(out.stderr.is_empty());
    let out: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["changed"], true);
    assert_eq!(out["data"][0]["status"], "failed");
    assert_eq!(out["data"][0]["refusals"][0]["code"], "gh-failed");
    assert_eq!(out["data"][1]["status"], "synced");
    let before = json(&dir, &["show", "cli#14895"])["data"].clone();
    assert_eq!(before["column"], "review");
    assert_eq!(json(&dir, &["list"])["data"][0]["column"], "review");
    assert_eq!(json(&dir, &["ready"])["data"], serde_json::json!([]));
    for (failure, code) in [
        ("offline", "offline"),
        ("denied", "gh-failed"),
        ("malformed", "gh-failed"),
        ("shape", "gh-failed"),
    ] {
        for verb in ["claim", "unclaim"] {
            let out = github_run(&[verb, "cli#14895"], failure)
                .code(1)
                .get_output()
                .clone();
            assert!(out.stderr.is_empty());
            let out: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(out["ok"], false);
            assert_eq!(out["refusals"][0]["code"], code);
            assert_eq!(json(&dir, &["show", "cli#14895"])["data"], before);
        }
    }
}

#[test]
fn basecamp_link_status_and_agent_refusal_are_exposed() {
    let dir = setup();
    let args = [
        "bc",
        "link",
        "-p",
        "demo",
        "--bucket",
        "10",
        "--card-table",
        "20",
        "--map",
        "todo=30,doing=40",
    ];
    let out = json(&dir, &args);
    assert!(out["changed"].as_bool().unwrap());
    assert_eq!(json(&dir, &args)["changed"], false);
    assert_eq!(
        json(&dir, &["bc", "status", "-p", "demo"])["data"][0]["link"]["column_map"]["doing"],
        "40"
    );
    assert!(
        String::from_utf8_lossy(&run(&dir, &["bc", "status"]).success().get_output().stdout)
            .contains("todo=30")
    );
    let mut agent = vec!["--json", "--as", "bot"];
    agent.extend_from_slice(&args);
    assert!(
        String::from_utf8_lossy(&run(&dir, &agent).code(1).get_output().stdout)
            .contains("agent-cannot-manage-projects")
    );
    run(
        &dir,
        &[
            "bc",
            "link",
            "-p",
            "demo",
            "--bucket",
            "10",
            "--card-table",
            "20",
            "--map",
            "bogus=30",
        ],
    )
    .code(2);
    run(
        &dir,
        &[
            "bc",
            "link",
            "-p",
            "demo",
            "--bucket",
            "10",
            "--card-table",
            "20",
            "--map",
            "todo=30,todo=40",
        ],
    )
    .code(2);
    assert!(
        String::from_utf8_lossy(
            &run(&dir, &["bc", "status", "--help"])
                .success()
                .get_output()
                .stdout
        )
        .contains("agents should pass --json")
    );
}

#[test]
fn basecamp_failures_are_successful_json_warnings_and_visible_in_text_status() {
    use std::os::unix::fs::PermissionsExt;
    let dir = setup();
    json(
        &dir,
        &[
            "bc",
            "link",
            "-p",
            "demo",
            "--bucket",
            "10",
            "--card-table",
            "20",
            "--map",
            "todo=30",
        ],
    );
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let script = bin.join("basecamp");
    std::fs::write(&script, "#!/bin/sh\necho 'fixture denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("punch").unwrap();
    let output = cmd
        .env("PUNCH_STATE_DIR", dir.path())
        .env_remove("PUNCH_AGENT")
        .env("PATH", std::env::join_paths(paths).unwrap())
        .args(["--json", "add", "Fix it", "-p", "demo"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert!(
        value["warnings"][0]
            .as_str()
            .unwrap()
            .starts_with("basecamp-failed:")
    );
    assert_eq!(value["data"]["task"]["basecamp_card"]["dirty"], true);
    let text = run(&dir, &["bc", "status"])
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&text);
    assert!(text.contains("t1 dirty card="));
    assert!(text.contains("fixture denied"));
}

#[test]
fn bare_and_explicit_help_use_punch_name() {
    let dir = TempDir::new().unwrap();
    for args in [vec!["--json"], vec!["--help"]] {
        let output = run(&dir, &args).success().get_output().stdout.clone();
        let help = String::from_utf8(output).unwrap();
        assert!(help.contains("Usage: punch [OPTIONS] [COMMAND]"), "{help}");
    }
    assert!(!dir.path().join("punch.db").exists());
}

#[test]
fn board_json_stays_noninteractive_and_nonterminal_board_refuses_cleanly() {
    let dir = TempDir::new().unwrap();
    run(&dir, &["board", "--json"]).code(2);
    run(&dir, &[])
        .code(2)
        .stderr("invalid-input: the board requires a terminal; use list --json for agents\n");
    run(&dir, &["board"]).code(2);
}

#[test]
fn board_documentation_describes_current_entrypoints_once() {
    let readme = include_str!("../../../README.md");
    let contract = include_str!("../../../docs/refusal-codes.md");
    assert_eq!(
        readme
            .matches("Bare `punch` and `punch board` open")
            .count(),
        1
    );
    assert!(readme.contains("Ctrl-C"));
    assert!(!readme.contains("Bare `punch` prints help"));
    assert!(!readme.contains("synchronization, the board"));
    assert!(!readme.contains("phase 2"));
    assert_eq!(
        contract
            .matches("Without a subcommand, an interactive terminal opens the board.")
            .count(),
        1
    );
    assert!(!contract.contains("Without a subcommand, punch prints help"));
    assert!(contract.contains("bare `punch --json`"));
    assert!(contract.contains("`punch board --json` exits 2"));
}

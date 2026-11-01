#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

fn command(dir: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("punch"));
    cmd.env("PUNCH_STATE_DIR", dir.path())
        .env_remove("PUNCH_AGENT");
    cmd
}
fn session(dir: &TempDir, requests: &[Value]) -> Vec<Value> {
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let output = command(dir)
        .arg("mcp")
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
}
fn call(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}
fn envelope(reply: &Value) -> Value {
    let result = &reply["result"];
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    text
}
#[test]
fn piped_session_initializes_lists_calls_refuses_and_pings() {
    let dir = TempDir::new().unwrap();
    command(&dir)
        .args(["project", "add", "demo"])
        .assert()
        .success();
    let replies = session(
        &dir,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":"catalog","method":"tools/list"}),
            call(3, "add", json!({"title":"MCP task","project":"demo"})),
            call(4, "move", json!({"reference":"t1","column":"doing"})),
            call(5, "move", json!({"reference":"t1","column":"done"})),
            json!({"jsonrpc":"2.0","id":6,"method":"ping"}),
        ],
    );
    assert_eq!(replies.len(), 6);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        replies[0]["result"]["capabilities"],
        json!({"tools":{"listChanged":false}})
    );
    assert_eq!(replies[1]["id"], "catalog");
    assert!(
        replies[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "add")
    );
    assert_eq!(envelope(&replies[2])["data"]["task"]["title"], "MCP task");
    assert_eq!(envelope(&replies[3])["data"]["column"], "doing");
    assert_eq!(
        envelope(&replies[4])["refusals"][0]["code"],
        "agent-cannot-finish"
    );
    assert_eq!(replies[4]["result"]["isError"], true);
    assert_eq!(replies[5]["result"], json!({}));
    let cli = command(&dir)
        .args(["--json", "show", "t1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli: Value = serde_json::from_slice(&cli).unwrap();
    let read = session(
        &dir,
        &[initialize(), call(2, "show", json!({"reference":"t1"}))],
    );
    assert_eq!(envelope(&read[1]), cli);
}

#[test]
fn guide_prints_agent_rules_and_json_envelope_without_opening_store() {
    let dir = TempDir::new().unwrap();
    let output = command(&dir)
        .arg("guide")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    for rule in [
        "--json",
        "punch sync",
        "punch ready",
        "claim",
        "review",
        "never",
        "finish",
        "archive",
        "refusals",
        "code",
        "0",
        "1",
        "2",
        "3",
        "not idempotent",
        "note",
    ] {
        assert!(text.contains(rule), "missing guide rule: {rule}");
    }
    let output = command(&dir)
        .args(["guide", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        value,
        json!({"ok":true,"changed":false,"data":text.trim_end(),"warnings":[]})
    );
    let read = session(&dir, &[initialize(), call(2, "guide", json!({}))]);
    assert_eq!(envelope(&read[1]), value);
    assert!(!dir.path().join("punch.db").exists());
}

#[test]
fn setup_prints_exact_registration_commands_and_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    for target in ["claude", "codex"] {
        let expected = format!("{target} mcp add punch -- punch mcp");
        let output = command(&dir)
            .env("CLAUDE_CONFIG_DIR", config.path())
            .env("CODEX_HOME", config.path())
            .args(["setup", target])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_eq!(String::from_utf8(output).unwrap(), format!("{expected}\n"));
        let output = command(&dir)
            .env("CLAUDE_CONFIG_DIR", config.path())
            .env("CODEX_HOME", config.path())
            .args(["--json", "setup", target])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap(),
            json!({"ok":true,"changed":false,"data":expected,"warnings":[]})
        );
    }
    command(&dir).args(["setup", "other"]).assert().code(2);
    assert_eq!(std::fs::read_dir(config.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn catalog_matches_cli_operations_and_exposes_flag_schemas() {
    let dir = TempDir::new().unwrap();
    let replies = session(
        &dir,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ],
    );
    let tools = replies[1]["result"]["tools"].as_array().unwrap();
    let mut names = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "add",
            "add_plan",
            "archive",
            "claim",
            "edit",
            "guide",
            "list",
            "move",
            "note",
            "project_add",
            "project_archive",
            "project_list",
            "ready",
            "show",
            "sync",
            "unarchive",
            "unclaim"
        ]
    );
    // Read the public CLI's help, independently of the MCP catalog implementation.
    let help = command(&dir)
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(help).unwrap();
    let cli_names = help
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !name.starts_with('-') && *name != "help")
        .collect::<Vec<_>>();
    for name in cli_names {
        if matches!(name, "mcp" | "setup") {
            continue;
        }
        if name == "project" {
            for child in ["add", "list", "archive"] {
                assert!(names.contains(&format!("project_{child}").as_str()));
            }
        } else {
            assert!(names.contains(&name), "missing tool for {name}");
        }
    }
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let path = if name == "add_plan" {
            vec!["add"]
        } else {
            name.split('_').collect()
        };
        let help = command(&dir)
            .args(&path)
            .arg("--help")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let help = String::from_utf8(help).unwrap();
        let properties = tool["inputSchema"]["properties"].as_object().unwrap();
        assert!(!properties.contains_key("as"));
        assert!(!properties.contains_key("json"));
        for key in properties.keys() {
            assert!(
                help.contains(&format!("--{key}"))
                    || help.contains(&format!("<{}>", key.to_uppercase()))
                    || help.contains(&format!("[{}]", key.to_uppercase())),
                "{name}: {key} absent from CLI"
            );
        }
    }
    let find = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
    assert_eq!(
        find("edit")["inputSchema"]["properties"]["clear-due"]["type"],
        "boolean"
    );
    assert_eq!(
        find("add")["inputSchema"]["properties"]["after"]["type"],
        "array"
    );
    assert_eq!(
        find("add")["inputSchema"]["properties"]["priority"]["maximum"],
        3
    );
    assert_eq!(
        find("claim")["inputSchema"]["properties"]["slot"]["type"],
        "string"
    );
    assert!(!dir.path().join("punch.db").exists());
}

#[test]
fn invalid_rpc_and_tool_arguments_do_not_stop_the_server_or_change_state() {
    let dir = TempDir::new().unwrap();
    command(&dir)
        .args(["project", "add", "demo"])
        .assert()
        .success();
    let replies = session(
        &dir,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","id":2,"method":"unknown"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope"}}),
            call(4, "add", json!({"project":"demo"})),
            call(
                5,
                "add",
                json!({"project":"demo","title":"bad","priority":"0"}),
            ),
            call(
                6,
                "add",
                json!({"project":"demo","title":"bad","as":"person"}),
            ),
            call(7, "add_plan", json!({"file":"-"})),
            call(8, "move", json!({"reference":"t1","column":"other"})),
            call(
                9,
                "edit",
                json!({"reference":"t1","due":"2027-01-01","clear-due":true}),
            ),
            call(
                10,
                "add",
                json!({"project":"demo","title":"bad","priority":4}),
            ),
            call(11, "list", json!({})),
            json!({"jsonrpc":"2.0","id":12,"method":"ping"}),
        ],
    );
    assert_eq!(replies[1]["error"]["code"], -32601);
    assert_eq!(replies[2]["error"]["code"], -32602);
    for reply in &replies[3..10] {
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert_eq!(envelope(reply)["refusals"][0]["code"], "invalid-input");
    }
    assert_eq!(envelope(&replies[10])["data"], json!([]));
    assert_eq!(replies[11]["result"], json!({}));
    let input = "not json\n[]\n{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":1}\n{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\n";
    let output = command(&dir)
        .arg("mcp")
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let values: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(values.len(), 3);
    assert_eq!(values[0]["error"]["code"], -32700);
    assert_eq!(values[1]["error"]["code"], -32600);
    assert_eq!(values[2]["result"], json!({}));
}

#[test]
fn mcp_actor_slot_precedence_and_restrictions_cannot_be_overridden_by_tools() {
    let dir = TempDir::new().unwrap();
    command(&dir)
        .args(["project", "add", "demo"])
        .assert()
        .success();
    command(&dir)
        .args(["add", "task", "-p", "demo"])
        .assert()
        .success();
    for (args, env, expected) in [
        (vec!["mcp"], None, "mcp"),
        (vec!["mcp"], Some("environment"), "environment"),
        (
            vec!["mcp", "--as", "explicit"],
            Some("environment"),
            "explicit",
        ),
    ] {
        let input = [
            initialize(),
            call(2, "claim", json!({"reference":"t1"})),
            call(3, "unclaim", json!({"reference":"t1"})),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let mut cmd = command(&dir);
        if let Some(env) = env {
            cmd.env("PUNCH_AGENT", env);
        }
        let output = cmd
            .args(args)
            .write_stdin(input)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(envelope(&replies[1])["data"]["slot"], expected);
        assert_eq!(envelope(&replies[1])["data"]["assignee"], expected);
    }
    let replies = session(
        &dir,
        &[
            initialize(),
            call(2, "archive", json!({"reference":"t1"})),
            call(3, "unarchive", json!({"reference":"t1"})),
            call(4, "project_add", json!({"slug":"other"})),
            call(5, "project_archive", json!({"slug":"demo"})),
        ],
    );
    for (reply, code) in replies[1..].iter().zip([
        "agent-cannot-archive",
        "agent-cannot-archive",
        "agent-cannot-manage-projects",
        "agent-cannot-manage-projects",
    ]) {
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(envelope(reply)["refusals"][0]["code"], code);
    }
}

#[test]
fn initialize_rejects_invalid_params_and_negotiates_versions() {
    let dir = TempDir::new().unwrap();
    let mut unknown = initialize();
    unknown["params"]["protocolVersion"] = json!("unknown");
    let replies = session(
        &dir,
        &[
            json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":[]}),
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":123}}),
            unknown,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":[]}),
        ],
    );
    assert_eq!(replies[0]["error"]["code"], -32600);
    assert_eq!(replies[1]["error"]["code"], -32602);
    assert_eq!(replies[2]["error"]["code"], -32602);
    assert_eq!(replies[3]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[4]["error"]["code"], -32602);
    let mut old = initialize();
    old["params"]["protocolVersion"] = json!("2024-11-05");
    assert_eq!(
        session(&dir, &[old])[0]["result"]["protocolVersion"],
        "2025-06-18"
    );
    assert!(!dir.path().join("punch.db").exists());
}

#[test]
fn plan_files_partial_results_and_storage_errors_use_cli_envelopes() {
    let dir = TempDir::new().unwrap();
    command(&dir)
        .args(["project", "add", "demo"])
        .assert()
        .success();
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &plan,
        r#"[{"title":"valid","project":"demo"},{"title":"invalid","project":"missing"}]"#,
    )
    .unwrap();
    let replies = session(
        &dir,
        &[
            initialize(),
            call(2, "add_plan", json!({"file":plan})),
            call(3, "list", json!({})),
        ],
    );
    assert_eq!(replies[1]["result"]["isError"], true);
    assert_eq!(envelope(&replies[1])["data"][0]["status"], "created");
    assert_eq!(envelope(&replies[1])["data"][1]["status"], "failed");
    assert_eq!(envelope(&replies[2])["data"].as_array().unwrap().len(), 1);
    let retry = session(
        &dir,
        &[initialize(), call(2, "add_plan", json!({"file":plan}))],
    );
    let cli = command(&dir)
        .args([
            "--json",
            "--as",
            "mcp",
            "add",
            "--file",
            plan.to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        envelope(&retry[1]),
        serde_json::from_slice::<Value>(&cli).unwrap()
    );
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("punch.db"), "broken database").unwrap();
    let replies = session(
        &dir,
        &[
            initialize(),
            call(2, "list", json!({})),
            json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
        ],
    );
    assert_eq!(replies[1]["result"]["isError"], true);
    assert_eq!(
        envelope(&replies[1])["refusals"][0]["code"],
        "storage-failed"
    );
    assert_eq!(replies[2]["result"], json!({}));
}

#[test]
fn tool_string_values_are_literal_and_empty_dependencies_are_supported() {
    let dir = TempDir::new().unwrap();
    command(&dir)
        .args(["project", "add", "demo"])
        .assert()
        .success();
    let replies = session(
        &dir,
        &[
            initialize(),
            call(
                2,
                "add",
                json!({"project":"demo","title":"--as=person","notes":"--json","after":[]}),
            ),
            call(3, "edit", json!({"reference":"t1","after":[]})),
            call(4, "claim", json!({"reference":"t1","slot":"--as=person"})),
            call(5, "move", json!({"reference":"t1","column":"done"})),
        ],
    );
    assert_eq!(
        envelope(&replies[1])["data"]["task"]["title"],
        "--as=person"
    );
    assert_eq!(envelope(&replies[1])["data"]["task"]["notes"], "--json");
    assert_eq!(envelope(&replies[2])["data"]["after"], json!([]));
    assert_eq!(envelope(&replies[3])["data"]["slot"], "--as=person");
    assert_eq!(
        envelope(&replies[4])["refusals"][0]["code"],
        "agent-cannot-finish"
    );
}

// Keep stdin open: a plan must never wait for EOF or consume the next request.
fn live_plan_session(dir: &TempDir, path: &std::path::Path) -> Vec<Value> {
    use std::io::{BufRead, Write};
    use std::process::Stdio;
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin!("punch"))
        .arg("mcp")
        .env("PUNCH_STATE_DIR", dir.path())
        .env_remove("PUNCH_AGENT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    for request in [
        initialize(),
        call(2, "add_plan", json!({"file":path})),
        json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
    ] {
        writeln!(stdin, "{request}").unwrap();
    }
    let replies = (0..3)
        .map(|_| rx.recv_timeout(std::time::Duration::from_secs(3)))
        .collect::<Result<Vec<_>, _>>();
    child.kill().unwrap();
    child.wait().unwrap();
    drop(stdin);
    reader.join().unwrap();
    replies
        .expect("server must answer while stdin stays open")
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(unix)]
#[test]
fn plan_rejects_stdin_aliases_and_non_regular_files_before_opening() {
    let dir = TempDir::new().unwrap();
    let fifo = dir.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let alias = dir.path().join("stdin-link");
    std::os::unix::fs::symlink("/dev/stdin", &alias).unwrap();
    let mut paths = vec![
        std::path::PathBuf::from("/dev/stdin"),
        alias,
        fifo,
        "/dev/null".into(),
        dir.path().to_owned(),
    ];
    #[cfg(target_os = "linux")]
    paths.push("/proc/self/fd/0".into());
    for path in paths {
        let replies = live_plan_session(&dir, &path);
        assert_eq!(replies[1]["result"]["isError"], true, "{}", path.display());
        assert_eq!(
            envelope(&replies[1])["refusals"][0]["code"],
            "invalid-input"
        );
        assert_eq!(replies[2]["id"], 3);
        assert_eq!(replies[2]["result"], json!({}));
    }
    let plan = dir.path().join("plan.json");
    std::fs::write(&plan, "[]").unwrap();
    let link = dir.path().join("plan-link");
    std::os::unix::fs::symlink(plan, &link).unwrap();
    assert_eq!(envelope(&live_plan_session(&dir, &link)[1])["ok"], true);
}

fn raw_session(dir: &TempDir, input: String) -> Vec<Value> {
    let output = command(dir)
        .arg("mcp")
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn assert_invalid_id(id: &str) {
    let dir = TempDir::new().unwrap();
    let replies = raw_session(
        &dir,
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"ping\"}}\n{{\"jsonrpc\":\"2.0\",\"id\":\"after\",\"method\":\"ping\"}}\n"
        ),
    );
    assert_eq!(replies.len(), 2);
    assert_eq!(
        replies[0],
        json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}),
        "id {id}"
    );
    assert_eq!(
        replies[1],
        json!({"jsonrpc":"2.0","id":"after","result":{}})
    );
}
#[test]
fn request_id_null_is_invalid() {
    assert_invalid_id("null");
}
#[test]
fn request_id_fractional_is_invalid() {
    for id in ["1.5", "1.0", "1e0"] {
        assert_invalid_id(id);
    }
}
#[test]
fn request_id_huge_is_invalid_without_rounding() {
    for id in [
        "18446744073709551616",
        "18446744073709551617",
        "-9223372036854775809",
        "1e400",
    ] {
        assert_invalid_id(id);
    }
}
#[test]
fn request_ids_echo_strings_and_integer_boundaries_exactly() {
    let dir = TempDir::new().unwrap();
    for id in [
        "\"a-string\"",
        "\"\"",
        "0",
        "-1",
        "-9223372036854775808",
        "9223372036854775807",
        "18446744073709551615",
    ] {
        let replies = raw_session(
            &dir,
            format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"ping\"}}\n"),
        );
        assert_eq!(replies[0]["id"].to_string(), id);
        assert_eq!(replies[0]["result"], json!({}));
    }
}

#[test]
fn version_negotiation_only_advertises_supported_non_batch_versions() {
    let dir = TempDir::new().unwrap();
    for (requested, expected) in [
        ("2025-06-18", "2025-06-18"),
        ("2025-11-25", "2025-11-25"),
        ("2024-11-05", "2025-06-18"),
        ("2025-03-26", "2025-06-18"),
        ("unknown", "2025-06-18"),
        ("2099-01-01", "2025-06-18"),
    ] {
        let mut init = initialize();
        init["params"]["protocolVersion"] = json!(requested);
        let replies = session(
            &dir,
            &[
                init,
                json!([{"jsonrpc":"2.0","id":2,"method":"ping"}, {"jsonrpc":"2.0","id":3,"method":"ping"}]),
                json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
            ],
        );
        assert_eq!(
            replies[0]["result"]["protocolVersion"], expected,
            "requested {requested}"
        );
        assert_eq!(replies[1]["error"]["code"], -32600);
        assert_eq!(replies[1]["id"], Value::Null);
        assert_eq!(replies[2]["result"], json!({}));
    }
}

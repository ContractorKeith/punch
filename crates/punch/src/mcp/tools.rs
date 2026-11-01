//! Tool schemas and argument conversion share clap's command definitions.
use crate::cli::{self, Cli};
use clap::{ArgAction, CommandFactory, Parser};
use punch_core::error::PunchError;
use serde_json::{Value, json};
use std::io::Read;

const MAX_PLAN_BYTES: u64 = 1024 * 1024;

pub struct Tool {
    pub name: String,
    path: Vec<String>,
    command: clap::Command,
    pub schema: Value,
}

pub fn catalog() -> Vec<Tool> {
    let mut root = Cli::command();
    root.build();
    let mut tools = Vec::new();
    for command in root.get_subcommands() {
        if matches!(command.get_name(), "help" | "mcp" | "setup" | "board") {
            continue;
        }
        if command.has_subcommands() {
            for child in command.get_subcommands().filter(|c| c.get_name() != "help") {
                tools.push(Tool::new(
                    vec![command.get_name(), child.get_name()],
                    child.clone(),
                ));
            }
        } else {
            tools.push(Tool::new(vec![command.get_name()], command.clone()));
        }
    }
    // The plan op shares `add` in the CLI but has its own MCP tool.
    if let Some(add) = tools.iter().find(|t| t.name == "add") {
        let mut plan = Tool::new(vec!["add"], add.command.clone());
        plan.name = "add_plan".into();
        plan.schema["properties"] = json!({"file":{"type":"string","description":"JSON plan path resolving to a regular file, at most 1 MiB. Stdin (-) is reserved for the MCP transport."}});
        plan.schema["required"] = json!(["file"]);
        tools.push(plan);
    }
    tools
}
fn exposed(arg: &clap::Arg) -> bool {
    !matches!(
        arg.get_id().as_str(),
        "json" | "slot" | "help" | "version" | "file"
    ) || arg.get_long() == Some("slot") // claim --slot is an op argument; --as is not.
}
fn property(arg: &clap::Arg) -> Value {
    match arg.get_id().as_str() {
        "priority" => json!({"type":"integer","minimum":0,"maximum":3}),
        "limit" => json!({"type":"integer","minimum":0}),
        "column" => {
            json!({"type":"string","enum":["todo","doing","review","blocked","done","shipped"]})
        }
        "scope" => json!({"type":"string","enum":["assigned","all"]}),
        "after" => json!({"type":"array","items":{"type":"string"}}),
        _ if matches!(arg.get_action(), ArgAction::SetTrue) => json!({"type":"boolean"}),
        _ => json!({"type":"string"}),
    }
}
fn key(arg: &clap::Arg) -> &str {
    arg.get_long().unwrap_or(arg.get_id().as_str())
}
impl Tool {
    fn new(path: Vec<&str>, command: clap::Command) -> Self {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for arg in command.get_arguments().filter(|a| exposed(a)) {
            properties.insert(key(arg).into(), property(arg));
            if arg.is_required_set() || (path == ["add"] && matches!(key(arg), "title" | "project"))
            {
                required.push(key(arg).to_owned());
            }
        }
        Self {
            name: path.join("_"),
            path: path.into_iter().map(str::to_owned).collect(),
            command,
            schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        }
    }
    pub fn definition(&self) -> Value {
        let description = if self.name == "add_plan" {
            "Import a JSON add plan from a file".to_owned()
        } else {
            self.command
                .get_about()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Punch {} operation", self.name))
        };
        json!({"name":self.name,"description":description,"inputSchema":self.schema})
    }
    pub fn call(&self, arguments: &Value, slot: &str) -> Result<cli::Reply, PunchError> {
        let object = arguments
            .as_object()
            .ok_or_else(|| PunchError::usage("arguments must be an object"))?;
        let properties = &self.schema["properties"];
        for (name, value) in object {
            let schema = &properties[name];
            let valid = match schema["type"].as_str() {
                Some("string") => value.is_string(),
                Some("boolean") => value.is_boolean(),
                Some("integer") => value.as_u64().is_some(),
                Some("array") => value
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string)),
                _ => false,
            };
            if !valid {
                return Err(PunchError::usage(format!("invalid argument: {name}")));
            }
        }
        for required in self.schema["required"].as_array().into_iter().flatten() {
            if !object.contains_key(required.as_str().unwrap_or("")) {
                return Err(PunchError::usage(format!("missing argument: {required}")));
            }
        }
        // Follow symlinks before opening: devices/FIFOs may consume transport or block.
        let plan = if let Some(path) = object.get("file").and_then(Value::as_str) {
            if path == "-" {
                return Err(PunchError::usage("stdin is reserved for the MCP transport"));
            }
            let metadata = std::fs::metadata(path).map_err(PunchError::usage)?;
            if !metadata.is_file() {
                return Err(PunchError::usage("MCP plans require a regular file"));
            }
            if metadata.len() > MAX_PLAN_BYTES {
                return Err(PunchError::usage("MCP plan exceeds 1 MiB"));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(PunchError::usage)?
                .take(MAX_PLAN_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(PunchError::usage)?;
            // The second bound also covers a file growing after metadata was read.
            if bytes.len() as u64 > MAX_PLAN_BYTES {
                return Err(PunchError::usage("MCP plan exceeds 1 MiB"));
            }
            Some(String::from_utf8(bytes).map_err(PunchError::usage)?)
        } else {
            None
        };
        let mut raw = vec!["punch".to_owned(), format!("--as={slot}"), "--json".into()];
        raw.extend(self.path.clone());
        // Named options precede positionals; -- protects titles/refs starting with '-'.
        for arg in self
            .command
            .get_arguments()
            .filter(|a| a.get_long().is_some())
        {
            let name = key(arg);
            let Some(value) = object.get(name) else {
                continue;
            };
            if value == false {
                continue;
            }
            if value == true {
                raw.push(format!("--{name}"));
                continue;
            }
            if name == "after" && value.as_array().is_some_and(Vec::is_empty) {
                if self.name == "edit" {
                    raw.push("--clear-after".into());
                }
                continue;
            }
            let text = if let Some(array) = value.as_array() {
                array
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            };
            raw.push(format!("--{name}={text}"));
        }
        raw.push("--".into());
        for arg in self.command.get_positionals() {
            if let Some(value) = object.get(key(arg)) {
                raw.push(value.as_str().unwrap_or("").into());
            }
        }
        let cli = Cli::try_parse_from(raw).map_err(PunchError::usage)?;
        cli::execute_with_plan(&cli, plan.as_deref())
    }
}

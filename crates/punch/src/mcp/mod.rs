//! Synchronous newline-delimited JSON-RPC; stdout contains protocol responses only.
mod tools;
use punch_core::error::PunchError;
use serde_json::{Value, json, value::RawValue};
use std::io::{BufRead, Read, Write};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_TOOL_RESULT_BYTES: usize = 4 * 1024 * 1024;

// Parse containers as raw tokens so serde_json's private number tag cannot turn
// a wire object into a number. Genuine numeric tokens retain exact precision.
fn wire_value(raw: &RawValue, depth: usize) -> Result<Value, serde_json::Error> {
    if depth >= 128 {
        return Err(serde::de::Error::custom("JSON nesting limit exceeded"));
    }
    match raw.get().as_bytes().first() {
        Some(b'{') => {
            let fields: std::collections::BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(raw.get())?;
            fields
                .into_iter()
                .map(|(key, value)| Ok((key, wire_value(&value, depth + 1)?)))
                .collect::<Result<serde_json::Map<_, _>, _>>()
                .map(Value::Object)
        }
        Some(b'[') => {
            let items: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
            items
                .iter()
                .map(|value| wire_value(value, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        _ => serde_json::from_str(raw.get()),
    }
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
// Keep committed writes identifiable without echoing large task fields or diagnostics.
fn receipt(data: &Value) -> Value {
    if let Some(items) = data.as_array() {
        return Value::Array(items.iter().map(receipt).collect());
    }
    if let Some(task) = data.get("task") {
        let mut result = json!({"task":receipt(task)});
        for key in ["existing", "status"] {
            if let Some(value) = data.get(key) {
                result[key] = value.clone();
            }
        }
        return result;
    }
    if let (Some(id), Some(revision)) = (data["id"].as_i64(), data["revision"].as_i64()) {
        return json!({"ref":format!("t{id}"),"revision":revision});
    }
    if let Some(id) = data.get("id") {
        return json!({"id":id});
    }
    // Sync identifies its per-project outcomes by slug rather than database ID.
    if let Some(project) = data.get("project") {
        return json!({"project":project,"status":data["status"]});
    }
    Value::Null
}
fn tool_result(result: Result<crate::cli::Reply, PunchError>) -> Value {
    let (value, failed) = match result {
        Ok(reply) => (reply.value, reply.exit_code != 0),
        Err(e) => (json!({"ok":false,"refusals":[e.detail()]}), true),
    };
    let response = json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":failed});
    // Count both envelopes and JSON escaping, rather than just the domain data.
    if response.to_string().len() > MAX_TOOL_RESULT_BYTES {
        if value["changed"] == true {
            return tool_result(Ok(crate::cli::Reply {
                value: json!({
                    "ok":true,"changed":true,"data":receipt(&value["data"]),
                    "warnings":["result-truncated: result exceeded 4 MiB; use show to read it"]
                }),
                exit_code: 0,
            }));
        }
        return tool_result(Err(PunchError::usage(
            "Tool result exceeded 4 MiB; narrow the request with a project filter, a limit, or show a single task",
        )));
    }
    response
}
pub fn run(slot: &str) -> Result<(), PunchError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(stdin.lock(), stdout.lock(), slot)
}
fn serve(mut input: impl BufRead, mut output: impl Write, slot: &str) -> Result<(), PunchError> {
    let catalog = tools::catalog();
    let mut initialized = false;
    loop {
        // At most the payload cap plus CRLF; drain oversize lines without buffering.
        let mut line = Vec::new();
        let read = (&mut input)
            .take((MAX_REQUEST_BYTES + 2) as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        let newline = line.last() == Some(&b'\n');
        if newline {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        }
        let response = if line.len() > MAX_REQUEST_BYTES {
            if !newline {
                input.skip_until(b'\n')?;
            }
            Some(error(Value::Null, -32600, "Request exceeds 1 MiB"))
        } else {
            match serde_json::from_slice::<Box<RawValue>>(&line).and_then(|raw| wire_value(&raw, 0))
            {
                Err(_) => Some(error(Value::Null, -32700, "Parse error")),
                Ok(request) => {
                    let id = request.get("id").cloned().unwrap_or(Value::Null);
                    if !request.is_object()
                        || request["jsonrpc"] != "2.0"
                        || !request["method"].is_string()
                        || request.get("id").is_some_and(|v| {
                            !v.is_string() && v.as_i64().is_none() && v.as_u64().is_none()
                        })
                        || request
                            .get("params")
                            .is_some_and(|v| !v.is_object() && !v.is_array())
                    {
                        Some(error(Value::Null, -32600, "Invalid Request"))
                    } else if request.get("id").is_none() {
                        // Notifications, including initialized, never receive a response.
                        None
                    } else {
                        let method = request["method"].as_str().unwrap_or("");
                        let result = match method {
                            "initialize"
                                if !request["params"]["protocolVersion"].is_string()
                                    || !request["params"]["capabilities"].is_object()
                                    || !request["params"]["clientInfo"]["name"].is_string()
                                    || !request["params"]["clientInfo"]["version"].is_string() =>
                            {
                                Err((-32602, "Invalid initialize parameters"))
                            }
                            "initialize" => {
                                initialized = true;
                                let version =
                                    request["params"]["protocolVersion"].as_str().unwrap_or("");
                                let version = match version {
                                    "2025-06-18" | "2025-11-25" => version,
                                    _ => "2025-06-18",
                                };
                                Ok(
                                    json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"punch","version":env!("CARGO_PKG_VERSION")}}),
                                )
                            }
                            "ping" => Ok(json!({})),
                            "tools/list" if initialized => Ok(
                                json!({"tools":catalog.iter().map(|t| t.definition()).collect::<Vec<_>>()}),
                            ),
                            "tools/call" if initialized => {
                                let params = &request["params"];
                                if let Some(tool) = catalog
                                    .iter()
                                    .find(|t| Some(t.name.as_str()) == params["name"].as_str())
                                {
                                    Ok(tool_result(
                                        tool.call(
                                            params.get("arguments").unwrap_or(&json!({})),
                                            slot,
                                        ),
                                    ))
                                } else {
                                    Err((-32602, "Unknown tool"))
                                }
                            }
                            "tools/list" | "tools/call" => Err((-32600, "Initialize first")),
                            _ => Err((-32601, "Method not found")),
                        };
                        Some(match result {
                            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                            Err((code, message)) => error(id, code, message),
                        })
                    }
                }
            }
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response).map_err(PunchError::storage)?;
            writeln!(output)?;
            output.flush()?;
        }
    }
    Ok(())
}

//! Synchronous newline-delimited JSON-RPC; stdout contains protocol responses only.
mod tools;
use punch_core::error::PunchError;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn tool_result(result: Result<crate::cli::Reply, PunchError>) -> Value {
    let (value, failed) = match result {
        Ok(reply) => (reply.value, reply.exit_code != 0),
        Err(e) => (json!({"ok":false,"refusals":[e.detail()]}), true),
    };
    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":failed})
}
pub fn run(slot: &str) -> Result<(), PunchError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(stdin.lock(), stdout.lock(), slot)
}
fn serve(input: impl BufRead, mut output: impl Write, slot: &str) -> Result<(), PunchError> {
    let catalog = tools::catalog();
    let mut initialized = false;
    for line in input.lines() {
        let line = line?;
        let response = match serde_json::from_str::<Value>(&line) {
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
                                "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25" => {
                                    version
                                }
                                _ => "2025-11-25",
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
                                    tool.call(params.get("arguments").unwrap_or(&json!({})), slot),
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
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response).map_err(PunchError::storage)?;
            writeln!(output)?;
            output.flush()?;
        }
    }
    Ok(())
}

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use super::tool_definitions::get_tool_definitions;
use super::tool_handlers::handle_tool_call;

/// Build a JSON-RPC response for a given request.
///
/// Returns `None` for notification messages that should not generate a response.
fn build_response(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(json!({}));

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "cli-manager",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })),
        "notifications/initialized" => {
            // No response needed for notifications
            None
        }
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": get_tool_definitions()
            }
        })),
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            let (content, is_error) = handle_tool_call(tool_name, &arguments);

            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": content,
                    "isError": is_error
                }
            }))
        }
        "ping" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", method)
            }
        })),
    }
}

/// Build a JSON-RPC parse error response.
fn build_parse_error(error_message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": {
            "code": -32700,
            "message": format!("Parse error: {}", error_message)
        }
    })
}

/// Run the MCP server using stdio transport.
pub fn run() -> ! {
    eprintln!("cli-manager MCP server starting...");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let error_response = build_parse_error(&e.to_string());
                let _ = writeln!(stdout, "{}", error_response);
                let _ = stdout.flush();
                continue;
            }
        };

        if let Some(response) = build_response(&request) {
            let _ = writeln!(stdout, "{}", response);
            let _ = stdout.flush();
        }
    }

    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Tests: build_response — initialize
    // ========================================================================

    #[test]
    fn initialize_response_structure() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["serverInfo"]["name"], "cli-manager");
        assert!(result["serverInfo"]["version"].is_string());
    }

    #[test]
    fn initialize_response_preserves_id() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "initialize"
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["id"], 42);
    }

    #[test]
    fn initialize_response_preserves_string_id() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "abc-123",
            "method": "initialize"
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["id"], "abc-123");
    }

    // ========================================================================
    // Tests: build_response — notifications/initialized
    // ========================================================================

    #[test]
    fn initialized_notification_returns_none() {
        let request = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let response = build_response(&request);
        assert!(response.is_none());
    }

    // ========================================================================
    // Tests: build_response — tools/list
    // ========================================================================

    #[test]
    fn tools_list_response_structure() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 2);
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 11);
    }

    #[test]
    fn tools_list_contains_all_tool_names() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/list"
        });
        let response = build_response(&request).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"terminal_list"));
        assert!(names.contains(&"terminal_create"));
        assert!(names.contains(&"terminal_kill"));
        assert!(names.contains(&"buffer_get"));
    }

    // ========================================================================
    // Tests: build_response — tools/call
    // ========================================================================

    #[test]
    fn tools_call_unknown_tool_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "nonexistent",
                "arguments": {}
            }
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 4);
        assert_eq!(response["result"]["isError"], true);
        let content = response["result"]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert!(
            content[0]["text"]
                .as_str()
                .unwrap()
                .contains("Unknown tool"),
        );
    }

    #[test]
    fn tools_call_missing_params_name_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {}
        });
        let response = build_response(&request).unwrap();
        // When name is missing, it defaults to "" which is an unknown tool
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn tools_call_with_validation_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "terminal_kill",
                "arguments": {}
            }
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter"), "got: {text}");
    }

    #[test]
    fn tools_call_response_has_correct_structure() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "terminal_list",
                "arguments": {}
            }
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 7);
        // result has content and isError
        assert!(response["result"].get("content").is_some());
        assert!(response["result"].get("isError").is_some());
    }

    // ========================================================================
    // Tests: build_response — ping
    // ========================================================================

    #[test]
    fn ping_response() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "ping"
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 10);
        assert!(response["result"].is_object());
        // result should be empty object
        assert_eq!(response["result"], json!({}));
    }

    // ========================================================================
    // Tests: build_response — unknown method
    // ========================================================================

    #[test]
    fn unknown_method_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "some/unknown/method"
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 11);
        assert_eq!(response["error"]["code"], -32601);
        let msg = response["error"]["message"].as_str().unwrap();
        assert!(
            msg.contains("Method not found: some/unknown/method"),
            "got: {msg}"
        );
    }

    #[test]
    fn empty_method_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 12
        });
        let response = build_response(&request).unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }

    // ========================================================================
    // Tests: build_response — id handling
    // ========================================================================

    #[test]
    fn null_id_is_preserved() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": null,
            "method": "ping"
        });
        let response = build_response(&request).unwrap();
        assert!(response["id"].is_null());
    }

    #[test]
    fn missing_id_becomes_null() {
        let request = json!({
            "jsonrpc": "2.0",
            "method": "ping"
        });
        let response = build_response(&request).unwrap();
        assert!(response["id"].is_null());
    }

    // ========================================================================
    // Tests: build_parse_error
    // ========================================================================

    #[test]
    fn parse_error_structure() {
        let response = build_parse_error("unexpected token");
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response["id"].is_null());
        assert_eq!(response["error"]["code"], -32700);
        let msg = response["error"]["message"].as_str().unwrap();
        assert!(msg.contains("Parse error: unexpected token"), "got: {msg}");
    }

    #[test]
    fn parse_error_has_null_id() {
        let response = build_parse_error("bad json");
        assert!(response["id"].is_null());
    }
}

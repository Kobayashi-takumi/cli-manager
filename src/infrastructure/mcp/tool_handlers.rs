use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use crate::infrastructure::ipc::socket_discovery;

/// Send an IPC command to the running CLI Manager instance and return the response.
fn send_ipc_command(ipc_json: &str) -> Result<Value, String> {
    let socket_path = socket_discovery::read_socket_path()
        .map_err(|_| "No running cli-manager instance found. Is cli-manager running?".to_string())?;

    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|e| format!("Failed to connect to cli-manager: {}", e))?;

    // Send command (newline-delimited)
    let mut msg = ipc_json.to_string();
    msg.push('\n');
    stream
        .write_all(msg.as_bytes())
        .map_err(|e| format!("Failed to send command: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush: {}", e))?;

    // Read response
    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("Failed to read response: {}", e))?;

    serde_json::from_str(&response).map_err(|e| format!("Invalid response JSON: {}", e))
}

/// Format a successful IPC response into MCP content.
fn format_ipc_response(ipc_response: &Value) -> (Value, bool) {
    let ok = ipc_response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if ok {
        let text = if let Some(data) = ipc_response.get("data") {
            serde_json::to_string_pretty(data).unwrap_or_else(|_| "OK".to_string())
        } else {
            "OK".to_string()
        };
        (json!([{"type": "text", "text": text}]), false)
    } else {
        let error_msg = ipc_response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        (json!([{"type": "text", "text": error_msg}]), true)
    }
}

/// Build the IPC JSON command string for a given MCP tool call.
///
/// Returns Ok(ipc_json_string) on success, or Err((content, is_error)) for
/// parameter validation failures or unknown tools.
fn build_ipc_command(tool_name: &str, arguments: &Value) -> Result<String, (Value, bool)> {
    match tool_name {
        "terminal_list" => Ok(r#"{"cmd":"list-windows"}"#.to_string()),
        "terminal_create" => {
            let mut cmd = json!({"cmd": "create-window"});
            if let Some(name) = arguments.get("name").and_then(|v| v.as_str()) {
                cmd["name"] = json!(name);
            }
            if let Some(command) = arguments.get("command").and_then(|v| v.as_str()) {
                cmd["command"] = json!(command);
            }
            Ok(cmd.to_string())
        }
        "terminal_kill" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| missing_param_error("target"))?;
            Ok(json!({"cmd": "kill-window", "target": target}).to_string())
        }
        "terminal_select" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| missing_param_error("target"))?;
            Ok(json!({"cmd": "select-window", "target": target}).to_string())
        }
        "terminal_rename" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| missing_param_error("target"))?;
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| missing_param_error("name"))?;
            Ok(json!({"cmd": "rename-window", "target": target, "name": name}).to_string())
        }
        "terminal_send_keys" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| missing_param_error("target"))?;
            let keys = arguments
                .get("keys")
                .and_then(|v| v.as_array())
                .ok_or_else(|| missing_param_error("keys"))?
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>();
            Ok(json!({"cmd": "send-keys", "target": target, "keys": keys}).to_string())
        }
        "terminal_capture" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| missing_param_error("target"))?;
            let scrollback = arguments
                .get("include_scrollback")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(json!({"cmd": "capture-pane", "target": target, "scrollback": scrollback}).to_string())
        }
        "buffer_get" => Ok(r#"{"cmd":"show-buffer"}"#.to_string()),
        "buffer_set" => {
            let text = arguments
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| missing_param_error("text"))?;
            Ok(json!({"cmd": "set-buffer", "text": text}).to_string())
        }
        "buffer_paste" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| missing_param_error("target"))?;
            Ok(json!({"cmd": "paste-buffer", "target": target}).to_string())
        }
        "notify" => {
            let body = arguments
                .get("body")
                .and_then(|v| v.as_str())
                .ok_or_else(|| missing_param_error("body"))?;
            let mut cmd = json!({"cmd": "notify", "body": body});
            if let Some(title) = arguments.get("title").and_then(|v| v.as_str()) {
                cmd["title"] = json!(title);
            }
            Ok(cmd.to_string())
        }
        _ => Err((
            json!([{"type": "text", "text": format!("Unknown tool: {}", tool_name)}]),
            true,
        )),
    }
}

/// Create the error tuple for a missing required parameter.
fn missing_param_error(param_name: &str) -> (Value, bool) {
    (
        json!([{"type": "text", "text": format!("Missing required parameter: {}", param_name)}]),
        true,
    )
}

/// Handle an MCP tool call by converting to IPC and returning the result.
///
/// Returns (content_array, is_error).
pub fn handle_tool_call(tool_name: &str, arguments: &Value) -> (Value, bool) {
    let ipc_json = match build_ipc_command(tool_name, arguments) {
        Ok(json) => json,
        Err(err_tuple) => return err_tuple,
    };

    match send_ipc_command(&ipc_json) {
        Ok(ipc_response) => format_ipc_response(&ipc_response),
        Err(e) => (json!([{"type": "text", "text": e}]), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Tests: missing_param_error helper
    // ========================================================================

    #[test]
    fn missing_param_error_format() {
        let (content, is_error) = missing_param_error("target");
        assert!(is_error);
        let arr = content.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "Missing required parameter: target");
    }

    // ========================================================================
    // Tests: build_ipc_command — parameter validation for tools requiring target
    // ========================================================================

    #[test]
    fn build_terminal_kill_missing_target() {
        let result = build_ipc_command("terminal_kill", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    #[test]
    fn build_terminal_select_missing_target() {
        let result = build_ipc_command("terminal_select", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    #[test]
    fn build_terminal_rename_missing_target() {
        let result = build_ipc_command("terminal_rename", &json!({"name": "foo"}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    #[test]
    fn build_terminal_rename_missing_name() {
        let result = build_ipc_command("terminal_rename", &json!({"target": 1}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: name"), "got: {text}");
    }

    #[test]
    fn build_terminal_send_keys_missing_target() {
        let result = build_ipc_command("terminal_send_keys", &json!({"keys": ["a"]}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    #[test]
    fn build_terminal_send_keys_missing_keys() {
        let result = build_ipc_command("terminal_send_keys", &json!({"target": 1}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: keys"), "got: {text}");
    }

    #[test]
    fn build_terminal_capture_missing_target() {
        let result = build_ipc_command("terminal_capture", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    #[test]
    fn build_buffer_set_missing_text() {
        let result = build_ipc_command("buffer_set", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: text"), "got: {text}");
    }

    #[test]
    fn build_buffer_paste_missing_target() {
        let result = build_ipc_command("buffer_paste", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    // ========================================================================
    // Tests: build_ipc_command — unknown tool
    // ========================================================================

    #[test]
    fn build_unknown_tool() {
        let result = build_ipc_command("nonexistent_tool", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Unknown tool: nonexistent_tool"), "got: {text}");
    }

    // ========================================================================
    // Tests: build_ipc_command — successful command building
    // ========================================================================

    #[test]
    fn build_terminal_list_command() {
        let result = build_ipc_command("terminal_list", &json!({})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "list-windows");
    }

    #[test]
    fn build_terminal_create_no_args() {
        let result = build_ipc_command("terminal_create", &json!({})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "create-window");
    }

    #[test]
    fn build_terminal_create_with_name_and_command() {
        let result = build_ipc_command(
            "terminal_create",
            &json!({"name": "dev", "command": "/bin/zsh"}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "create-window");
        assert_eq!(v["name"], "dev");
        assert_eq!(v["command"], "/bin/zsh");
    }

    #[test]
    fn build_terminal_kill_command() {
        let result = build_ipc_command("terminal_kill", &json!({"target": 3})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "kill-window");
        assert_eq!(v["target"], 3);
    }

    #[test]
    fn build_terminal_select_command() {
        let result = build_ipc_command("terminal_select", &json!({"target": 2})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "select-window");
        assert_eq!(v["target"], 2);
    }

    #[test]
    fn build_terminal_rename_command() {
        let result = build_ipc_command(
            "terminal_rename",
            &json!({"target": 1, "name": "my-server"}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "rename-window");
        assert_eq!(v["target"], 1);
        assert_eq!(v["name"], "my-server");
    }

    #[test]
    fn build_terminal_send_keys_command() {
        let result = build_ipc_command(
            "terminal_send_keys",
            &json!({"target": 1, "keys": ["ls", "Enter"]}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "send-keys");
        assert_eq!(v["target"], 1);
        let keys = v["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], "ls");
        assert_eq!(keys[1], "Enter");
    }

    #[test]
    fn build_terminal_capture_without_scrollback() {
        let result =
            build_ipc_command("terminal_capture", &json!({"target": 1})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "capture-pane");
        assert_eq!(v["target"], 1);
        assert_eq!(v["scrollback"], false);
    }

    #[test]
    fn build_terminal_capture_with_scrollback() {
        let result = build_ipc_command(
            "terminal_capture",
            &json!({"target": 1, "include_scrollback": true}),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "capture-pane");
        assert_eq!(v["target"], 1);
        assert_eq!(v["scrollback"], true);
    }

    #[test]
    fn build_buffer_get_command() {
        let result = build_ipc_command("buffer_get", &json!({})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "show-buffer");
    }

    #[test]
    fn build_buffer_set_command() {
        let result =
            build_ipc_command("buffer_set", &json!({"text": "hello world"})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "set-buffer");
        assert_eq!(v["text"], "hello world");
    }

    #[test]
    fn build_buffer_paste_command() {
        let result = build_ipc_command("buffer_paste", &json!({"target": 5})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "paste-buffer");
        assert_eq!(v["target"], 5);
    }

    // ========================================================================
    // Tests: format_ipc_response
    // ========================================================================

    #[test]
    fn format_ok_response_without_data() {
        let ipc_resp = json!({"ok": true});
        let (content, is_error) = format_ipc_response(&ipc_resp);
        assert!(!is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert_eq!(text, "OK");
    }

    #[test]
    fn format_ok_response_with_data() {
        let ipc_resp = json!({"ok": true, "data": {"id": 3}});
        let (content, is_error) = format_ipc_response(&ipc_resp);
        assert!(!is_error);
        let text = content[0]["text"].as_str().unwrap();
        // Should be pretty-printed JSON of the data field
        assert!(text.contains("\"id\": 3"), "got: {text}");
    }

    #[test]
    fn format_ok_response_with_array_data() {
        let ipc_resp = json!({"ok": true, "data": [{"id": 1, "name": "term1"}]});
        let (content, is_error) = format_ipc_response(&ipc_resp);
        assert!(!is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("term1"), "got: {text}");
    }

    #[test]
    fn format_error_response() {
        let ipc_resp = json!({"ok": false, "error": "terminal not found"});
        let (content, is_error) = format_ipc_response(&ipc_resp);
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert_eq!(text, "terminal not found");
    }

    #[test]
    fn format_error_response_without_error_field() {
        let ipc_resp = json!({"ok": false});
        let (content, is_error) = format_ipc_response(&ipc_resp);
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert_eq!(text, "Unknown error");
    }

    #[test]
    fn format_response_missing_ok_field() {
        // If "ok" is missing, defaults to false
        let ipc_resp = json!({"data": "something"});
        let (_, is_error) = format_ipc_response(&ipc_resp);
        assert!(is_error);
    }

    // ========================================================================
    // Tests: handle_tool_call — parameter validation (no socket needed)
    // ========================================================================

    #[test]
    fn handle_unknown_tool() {
        let (content, is_error) = handle_tool_call("does_not_exist", &json!({}));
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Unknown tool: does_not_exist"), "got: {text}");
    }

    #[test]
    fn handle_terminal_kill_no_target() {
        let (content, is_error) = handle_tool_call("terminal_kill", &json!({}));
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: target"), "got: {text}");
    }

    #[test]
    fn handle_terminal_rename_no_name() {
        let (content, is_error) =
            handle_tool_call("terminal_rename", &json!({"target": 1}));
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: name"), "got: {text}");
    }

    #[test]
    fn handle_buffer_set_no_text() {
        let (content, is_error) = handle_tool_call("buffer_set", &json!({}));
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: text"), "got: {text}");
    }

    // ========================================================================
    // Tests: handle_tool_call — connection failure (no running instance)
    // ========================================================================

    // ========================================================================
    // Tests: build_ipc_command — notify
    // ========================================================================

    #[test]
    fn build_notify_with_title_and_body() {
        let result = build_ipc_command("notify", &json!({"title": "Build", "body": "Done"})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "notify");
        assert_eq!(v["body"], "Done");
        assert_eq!(v["title"], "Build");
    }

    #[test]
    fn build_notify_body_only() {
        let result = build_ipc_command("notify", &json!({"body": "Task complete"})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cmd"], "notify");
        assert_eq!(v["body"], "Task complete");
        assert!(v.get("title").is_none());
    }

    #[test]
    fn build_notify_missing_body() {
        let result = build_ipc_command("notify", &json!({"title": "Test"}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: body"), "got: {text}");
    }

    #[test]
    fn build_notify_empty_args() {
        let result = build_ipc_command("notify", &json!({}));
        let (content, is_error) = result.unwrap_err();
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: body"), "got: {text}");
    }

    // ========================================================================
    // Tests: handle_tool_call — notify parameter validation
    // ========================================================================

    #[test]
    fn handle_notify_no_body() {
        let (content, is_error) = handle_tool_call("notify", &json!({}));
        assert!(is_error);
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required parameter: body"), "got: {text}");
    }

    // ========================================================================
    // Tests: handle_tool_call — connection failure (no running instance)
    // ========================================================================

    #[test]
    fn handle_terminal_list_no_running_instance() {
        // This test verifies that when no cli-manager is running,
        // handle_tool_call returns a connection error (not a crash).
        // The exact error depends on whether a discovery file exists.
        let (content, is_error) = handle_tool_call("terminal_list", &json!({}));
        // Should be an error since no cli-manager instance is running
        // (unless one happens to be running, in which case it would succeed)
        // We just verify the response structure is valid
        assert!(content.is_array());
        assert_eq!(content.as_array().unwrap().len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert!(content[0]["text"].is_string());
        // is_error is true when no instance running, but could be false if one is
        let _ = is_error;
    }
}

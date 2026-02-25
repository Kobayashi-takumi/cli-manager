use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process;

/// Entry point for `cm ctl <subcommand>`.
///
/// Parses CLI arguments, connects to the IPC socket, sends a JSON command,
/// reads the response, and outputs to stdout/stderr.
pub fn run(args: &[String]) -> ! {
    if args.len() < 3 {
        print_usage();
        process::exit(1);
    }

    let subcommand = &args[2];
    let sub_args = &args[3..];

    let json = match build_request(subcommand, sub_args) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };

    let response = match send_request(&json) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };

    // Parse response JSON
    let parsed: serde_json::Value = match serde_json::from_str(&response) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error parsing response: {}", e);
            process::exit(1);
        }
    };

    if parsed.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        // Check if --raw flag was passed
        let raw = sub_args.contains(&"--raw".to_string());
        if raw {
            println!("{}", response);
        } else {
            // Pretty-print the response
            match serde_json::to_string_pretty(&parsed) {
                Ok(pretty) => println!("{}", pretty),
                Err(_) => println!("{}", response),
            }
        }
        process::exit(0);
    } else {
        let error_msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        eprintln!("error: {}", error_msg);
        process::exit(1);
    }
}

fn build_request(subcommand: &str, args: &[String]) -> Result<String, String> {
    match subcommand {
        "send-keys" => {
            let (target, keys) = parse_target_and_rest(args, "send-keys")?;
            if keys.is_empty() {
                return Err("send-keys requires at least one key argument".to_string());
            }
            let keys_json: Vec<serde_json::Value> = keys
                .iter()
                .map(|k| serde_json::Value::String(k.clone()))
                .collect();
            Ok(serde_json::json!({
                "cmd": "send-keys",
                "target": target,
                "keys": keys_json,
            })
            .to_string())
        }
        "capture-pane" => {
            let (target, rest) = parse_target_and_rest(args, "capture-pane")?;
            let scrollback = rest.contains(&"-S".to_string());
            Ok(serde_json::json!({
                "cmd": "capture-pane",
                "target": target,
                "scrollback": scrollback,
            })
            .to_string())
        }
        "list-windows" => Ok(serde_json::json!({"cmd": "list-windows"}).to_string()),
        "paste-buffer" => {
            let (target, _) = parse_target_and_rest(args, "paste-buffer")?;
            Ok(serde_json::json!({
                "cmd": "paste-buffer",
                "target": target,
            })
            .to_string())
        }
        "set-buffer" => {
            if args.is_empty() {
                return Err("set-buffer requires a text argument".to_string());
            }
            // Join all remaining args as the text
            let text = args.join(" ");
            Ok(serde_json::json!({
                "cmd": "set-buffer",
                "text": text,
            })
            .to_string())
        }
        "show-buffer" => Ok(serde_json::json!({"cmd": "show-buffer"}).to_string()),
        "create-window" => {
            let mut obj = serde_json::json!({"cmd": "create-window"});
            let mut i = 0;
            while i < args.len() {
                match args[i].as_str() {
                    "--name" => {
                        if i + 1 < args.len() {
                            obj["name"] = serde_json::json!(&args[i + 1]);
                            i += 2;
                        } else {
                            return Err("--name requires a value".to_string());
                        }
                    }
                    "--cmd" => {
                        if i + 1 < args.len() {
                            obj["command"] = serde_json::json!(&args[i + 1]);
                            i += 2;
                        } else {
                            return Err("--cmd requires a value".to_string());
                        }
                    }
                    other => {
                        return Err(format!("unknown option: {}", other));
                    }
                }
            }
            Ok(obj.to_string())
        }
        "kill-window" => {
            let (target, _) = parse_target_and_rest(args, "kill-window")?;
            Ok(serde_json::json!({"cmd": "kill-window", "target": target}).to_string())
        }
        "select-window" => {
            let (target, _) = parse_target_and_rest(args, "select-window")?;
            Ok(serde_json::json!({"cmd": "select-window", "target": target}).to_string())
        }
        "rename-window" => {
            let (target, rest2) = parse_target_and_rest(args, "rename-window")?;
            let mut name: Option<String> = None;
            let mut i = 0;
            while i < rest2.len() {
                if rest2[i] == "--name" {
                    if i + 1 < rest2.len() {
                        name = Some(rest2[i + 1].clone());
                        i += 2;
                    } else {
                        return Err("--name requires a value".to_string());
                    }
                } else {
                    return Err(format!("unknown option: {}", rest2[i]));
                }
            }
            let name = match name {
                Some(n) => n,
                None => {
                    return Err("--name is required for rename-window".to_string());
                }
            };
            Ok(
                serde_json::json!({"cmd": "rename-window", "target": target, "name": name})
                    .to_string(),
            )
        }
        _ => Err(format!("unknown subcommand: {}", subcommand)),
    }
}

/// Parse `-t <id>` from args and return (target_id, remaining_args).
fn parse_target_and_rest(args: &[String], cmd_name: &str) -> Result<(u32, Vec<String>), String> {
    let mut target: Option<u32> = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            i += 1;
            if i >= args.len() {
                return Err(format!("{} -t requires a terminal ID", cmd_name));
            }
            target = Some(
                args[i]
                    .parse::<u32>()
                    .map_err(|_| format!("invalid terminal ID: {}", args[i]))?,
            );
        } else if args[i] == "--raw" {
            // Skip --raw, it's handled at the response level
        } else {
            rest.push(args[i].clone());
        }
        i += 1;
    }
    match target {
        Some(t) => Ok((t, rest)),
        None => Err(format!("{} requires -t <id>", cmd_name)),
    }
}

fn send_request(json: &str) -> Result<String, String> {
    // Try CLI_MANAGER_SOCK env var first, then fall back to discovery file
    let socket_path = std::env::var("CLI_MANAGER_SOCK").or_else(|_| {
        crate::infrastructure::ipc::socket_discovery::read_socket_path()
            .map_err(|_| ())
    }).map_err(|_| {
        "No running cli-manager instance found. Is cli-manager running?".to_string()
    })?;

    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|e| format!("cannot connect to {}: {}", socket_path, e))?;

    // Send request + newline
    let mut request = json.to_string();
    request.push('\n');
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write error: {}", e))?;

    // Read response (one line)
    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("read error: {}", e))?;

    if response.is_empty() {
        return Err("empty response from server".to_string());
    }

    Ok(response.trim().to_string())
}

fn print_usage() {
    eprintln!("Usage: cm ctl <subcommand> [options]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  send-keys -t <id> <keys...>      Send keys to terminal");
    eprintln!("  capture-pane -t <id> [-S]         Capture terminal content");
    eprintln!("  list-windows                      List all terminals");
    eprintln!("  paste-buffer -t <id>              Paste yank buffer to terminal");
    eprintln!("  set-buffer <text>                 Set yank buffer text");
    eprintln!("  show-buffer                       Show yank buffer content");
    eprintln!("  create-window [--name <n>] [--cmd <c>]  Create a new terminal");
    eprintln!("  kill-window -t <id>               Kill a terminal");
    eprintln!("  select-window -t <id>             Select (focus) a terminal");
    eprintln!("  rename-window -t <id> --name <n>  Rename a terminal");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --raw    Output raw JSON response");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // ========================================================================
    // Helper to convert &str slices to Vec<String> for test convenience
    // ========================================================================

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    // ========================================================================
    // Tests: build_request — send-keys
    // ========================================================================

    #[test]
    fn build_request_send_keys() {
        let args = s(&["-t", "2", "cargo test", "Enter"]);
        let json_str = build_request("send-keys", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "send-keys");
        assert_eq!(v["target"], 2);
        let keys = v["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], "cargo test");
        assert_eq!(keys[1], "Enter");
    }

    #[test]
    fn build_request_send_keys_single_key() {
        let args = s(&["-t", "1", "q"]);
        let json_str = build_request("send-keys", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "send-keys");
        assert_eq!(v["target"], 1);
        let keys = v["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "q");
    }

    #[test]
    fn build_request_send_keys_missing_target() {
        let args = s(&["cargo test", "Enter"]);
        let err = build_request("send-keys", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    #[test]
    fn build_request_send_keys_no_keys() {
        let args = s(&["-t", "2"]);
        let err = build_request("send-keys", &args).unwrap_err();
        assert!(
            err.contains("requires at least one key argument"),
            "got: {err}"
        );
    }

    #[test]
    fn build_request_send_keys_empty_args() {
        let args = s(&[]);
        let err = build_request("send-keys", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — capture-pane
    // ========================================================================

    #[test]
    fn build_request_capture_pane_without_scrollback() {
        let args = s(&["-t", "1"]);
        let json_str = build_request("capture-pane", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "capture-pane");
        assert_eq!(v["target"], 1);
        assert_eq!(v["scrollback"], false);
    }

    #[test]
    fn build_request_capture_pane_with_scrollback() {
        let args = s(&["-t", "1", "-S"]);
        let json_str = build_request("capture-pane", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "capture-pane");
        assert_eq!(v["target"], 1);
        assert_eq!(v["scrollback"], true);
    }

    #[test]
    fn build_request_capture_pane_missing_target() {
        let args = s(&["-S"]);
        let err = build_request("capture-pane", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — list-windows
    // ========================================================================

    #[test]
    fn build_request_list_windows() {
        let args = s(&[]);
        let json_str = build_request("list-windows", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "list-windows");
    }

    // ========================================================================
    // Tests: build_request — paste-buffer
    // ========================================================================

    #[test]
    fn build_request_paste_buffer() {
        let args = s(&["-t", "3"]);
        let json_str = build_request("paste-buffer", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "paste-buffer");
        assert_eq!(v["target"], 3);
    }

    #[test]
    fn build_request_paste_buffer_missing_target() {
        let args = s(&[]);
        let err = build_request("paste-buffer", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — set-buffer
    // ========================================================================

    #[test]
    fn build_request_set_buffer() {
        let args = s(&["hello world"]);
        let json_str = build_request("set-buffer", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "set-buffer");
        assert_eq!(v["text"], "hello world");
    }

    #[test]
    fn build_request_set_buffer_multiple_words() {
        let args = s(&["hello", "world", "foo"]);
        let json_str = build_request("set-buffer", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "set-buffer");
        assert_eq!(v["text"], "hello world foo");
    }

    #[test]
    fn build_request_set_buffer_empty() {
        let args: Vec<String> = vec![];
        let err = build_request("set-buffer", &args).unwrap_err();
        assert!(err.contains("requires a text argument"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — show-buffer
    // ========================================================================

    #[test]
    fn build_request_show_buffer() {
        let args = s(&[]);
        let json_str = build_request("show-buffer", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "show-buffer");
    }

    // ========================================================================
    // Tests: build_request — unknown subcommand
    // ========================================================================

    #[test]
    fn build_request_unknown_subcommand() {
        let args = s(&[]);
        let err = build_request("unknown", &args).unwrap_err();
        assert!(err.contains("unknown subcommand: unknown"), "got: {err}");
    }

    #[test]
    fn build_request_unknown_subcommand_destroy() {
        let args = s(&[]);
        let err = build_request("destroy-everything", &args).unwrap_err();
        assert!(
            err.contains("unknown subcommand: destroy-everything"),
            "got: {err}"
        );
    }

    // ========================================================================
    // Tests: parse_target_and_rest — edge cases
    // ========================================================================

    #[test]
    fn parse_target_and_rest_basic() {
        let args = s(&["-t", "5", "foo", "bar"]);
        let (target, rest) = parse_target_and_rest(&args, "test-cmd").unwrap();
        assert_eq!(target, 5);
        assert_eq!(rest, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn parse_target_and_rest_target_at_end() {
        let args = s(&["foo", "-t", "3"]);
        let (target, rest) = parse_target_and_rest(&args, "test-cmd").unwrap();
        assert_eq!(target, 3);
        assert_eq!(rest, vec!["foo".to_string()]);
    }

    #[test]
    fn parse_target_and_rest_missing_id() {
        let args = s(&["-t"]);
        let err = parse_target_and_rest(&args, "test-cmd").unwrap_err();
        assert!(err.contains("-t requires a terminal ID"), "got: {err}");
    }

    #[test]
    fn parse_target_and_rest_invalid_id() {
        let args = s(&["-t", "abc"]);
        let err = parse_target_and_rest(&args, "test-cmd").unwrap_err();
        assert!(err.contains("invalid terminal ID: abc"), "got: {err}");
    }

    #[test]
    fn parse_target_and_rest_negative_id() {
        let args = s(&["-t", "-1"]);
        let err = parse_target_and_rest(&args, "test-cmd").unwrap_err();
        assert!(err.contains("invalid terminal ID: -1"), "got: {err}");
    }

    #[test]
    fn parse_target_and_rest_no_target_flag() {
        let args = s(&["foo", "bar"]);
        let err = parse_target_and_rest(&args, "test-cmd").unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    #[test]
    fn parse_target_and_rest_raw_flag_skipped() {
        let args = s(&["-t", "2", "--raw", "key1"]);
        let (target, rest) = parse_target_and_rest(&args, "test-cmd").unwrap();
        assert_eq!(target, 2);
        assert_eq!(rest, vec!["key1".to_string()]);
    }

    #[test]
    fn parse_target_and_rest_zero_id() {
        let args = s(&["-t", "0"]);
        let (target, rest) = parse_target_and_rest(&args, "test-cmd").unwrap();
        assert_eq!(target, 0);
        assert!(rest.is_empty());
    }

    // ========================================================================
    // Tests: build_request produces valid JSON parseable by protocol
    // ========================================================================

    #[test]
    fn build_request_send_keys_roundtrip_with_protocol() {
        let args = s(&["-t", "2", "cargo test", "Enter"]);
        let json_str = build_request("send-keys", &args).unwrap();
        // Verify the JSON can be parsed by the protocol parser
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "send-keys");
        assert_eq!(v["target"], 2);
        assert_eq!(v["keys"][0], "cargo test");
        assert_eq!(v["keys"][1], "Enter");
    }

    #[test]
    fn build_request_capture_pane_roundtrip_with_protocol() {
        let args = s(&["-t", "5", "-S"]);
        let json_str = build_request("capture-pane", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "capture-pane");
        assert_eq!(v["target"], 5);
        assert_eq!(v["scrollback"], true);
    }

    #[test]
    fn build_request_list_windows_roundtrip_with_protocol() {
        let json_str = build_request("list-windows", &s(&[])).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "list-windows");
    }

    // ========================================================================
    // Tests: send_request — env var not set
    // ========================================================================

    #[test]
    fn send_request_without_env_var_or_discovery() {
        // Temporarily remove the env var if set
        let saved = std::env::var("CLI_MANAGER_SOCK").ok();
        unsafe {
            std::env::remove_var("CLI_MANAGER_SOCK");
        }
        // Also ensure discovery file doesn't exist
        let discovery_path = crate::infrastructure::ipc::socket_discovery::discovery_file_path();
        let saved_discovery = std::fs::read_to_string(&discovery_path).ok();
        let _ = std::fs::remove_file(&discovery_path);

        let result = send_request("{}");

        // Restore env var
        if let Some(val) = saved {
            unsafe {
                std::env::set_var("CLI_MANAGER_SOCK", val);
            }
        }
        // Restore discovery file
        if let Some(content) = saved_discovery {
            let _ = std::fs::write(&discovery_path, content);
        }

        let err = result.unwrap_err();
        assert!(
            err.contains("No running cli-manager instance found"),
            "got: {err}"
        );
    }

    #[test]
    fn send_request_with_invalid_socket_path() {
        let saved = std::env::var("CLI_MANAGER_SOCK").ok();
        unsafe {
            std::env::set_var("CLI_MANAGER_SOCK", "/tmp/nonexistent-cli-manager-test.sock");
        }
        let result = send_request("{}");
        // Restore
        if let Some(val) = saved {
            unsafe {
                std::env::set_var("CLI_MANAGER_SOCK", val);
            }
        } else {
            unsafe {
                std::env::remove_var("CLI_MANAGER_SOCK");
            }
        }
        let err = result.unwrap_err();
        assert!(
            err.contains("cannot connect to"),
            "got: {err}"
        );
    }

    // ========================================================================
    // Tests: build_request — create-window
    // ========================================================================

    #[test]
    fn build_request_create_window_no_options() {
        let args = s(&[]);
        let json_str = build_request("create-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "create-window");
        // No name or command fields when not specified
        assert!(v.get("name").is_none());
        assert!(v.get("command").is_none());
    }

    #[test]
    fn build_request_create_window_with_name() {
        let args = s(&["--name", "my-terminal"]);
        let json_str = build_request("create-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "create-window");
        assert_eq!(v["name"], "my-terminal");
        assert!(v.get("command").is_none());
    }

    #[test]
    fn build_request_create_window_with_cmd() {
        let args = s(&["--cmd", "bash"]);
        let json_str = build_request("create-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "create-window");
        assert_eq!(v["command"], "bash");
        assert!(v.get("name").is_none());
    }

    #[test]
    fn build_request_create_window_with_name_and_cmd() {
        let args = s(&["--name", "dev", "--cmd", "zsh"]);
        let json_str = build_request("create-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "create-window");
        assert_eq!(v["name"], "dev");
        assert_eq!(v["command"], "zsh");
    }

    #[test]
    fn build_request_create_window_name_missing_value() {
        let args = s(&["--name"]);
        let err = build_request("create-window", &args).unwrap_err();
        assert!(err.contains("--name requires a value"), "got: {err}");
    }

    #[test]
    fn build_request_create_window_cmd_missing_value() {
        let args = s(&["--cmd"]);
        let err = build_request("create-window", &args).unwrap_err();
        assert!(err.contains("--cmd requires a value"), "got: {err}");
    }

    #[test]
    fn build_request_create_window_unknown_option() {
        let args = s(&["--foo"]);
        let err = build_request("create-window", &args).unwrap_err();
        assert!(err.contains("unknown option: --foo"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — kill-window
    // ========================================================================

    #[test]
    fn build_request_kill_window() {
        let args = s(&["-t", "3"]);
        let json_str = build_request("kill-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "kill-window");
        assert_eq!(v["target"], 3);
    }

    #[test]
    fn build_request_kill_window_missing_target() {
        let args = s(&[]);
        let err = build_request("kill-window", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — select-window
    // ========================================================================

    #[test]
    fn build_request_select_window() {
        let args = s(&["-t", "7"]);
        let json_str = build_request("select-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "select-window");
        assert_eq!(v["target"], 7);
    }

    #[test]
    fn build_request_select_window_missing_target() {
        let args = s(&[]);
        let err = build_request("select-window", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — rename-window
    // ========================================================================

    #[test]
    fn build_request_rename_window() {
        let args = s(&["-t", "2", "--name", "new-name"]);
        let json_str = build_request("rename-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "rename-window");
        assert_eq!(v["target"], 2);
        assert_eq!(v["name"], "new-name");
    }

    #[test]
    fn build_request_rename_window_missing_target() {
        let args = s(&["--name", "foo"]);
        let err = build_request("rename-window", &args).unwrap_err();
        assert!(err.contains("requires -t <id>"), "got: {err}");
    }

    #[test]
    fn build_request_rename_window_missing_name() {
        let args = s(&["-t", "2"]);
        let err = build_request("rename-window", &args).unwrap_err();
        assert!(err.contains("--name is required"), "got: {err}");
    }

    #[test]
    fn build_request_rename_window_name_missing_value() {
        let args = s(&["-t", "2", "--name"]);
        let err = build_request("rename-window", &args).unwrap_err();
        assert!(err.contains("--name requires a value"), "got: {err}");
    }

    #[test]
    fn build_request_rename_window_unknown_option() {
        let args = s(&["-t", "2", "--foo"]);
        let err = build_request("rename-window", &args).unwrap_err();
        assert!(err.contains("unknown option: --foo"), "got: {err}");
    }

    // ========================================================================
    // Tests: build_request — new subcommands roundtrip
    // ========================================================================

    #[test]
    fn build_request_create_window_roundtrip_with_protocol() {
        let args = s(&["--name", "test", "--cmd", "bash -l"]);
        let json_str = build_request("create-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "create-window");
        assert_eq!(v["name"], "test");
        assert_eq!(v["command"], "bash -l");
    }

    #[test]
    fn build_request_kill_window_roundtrip_with_protocol() {
        let args = s(&["-t", "10"]);
        let json_str = build_request("kill-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "kill-window");
        assert_eq!(v["target"], 10);
    }

    #[test]
    fn build_request_select_window_roundtrip_with_protocol() {
        let args = s(&["-t", "4"]);
        let json_str = build_request("select-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "select-window");
        assert_eq!(v["target"], 4);
    }

    #[test]
    fn build_request_rename_window_roundtrip_with_protocol() {
        let args = s(&["-t", "1", "--name", "renamed"]);
        let json_str = build_request("rename-window", &args).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["cmd"], "rename-window");
        assert_eq!(v["target"], 1);
        assert_eq!(v["name"], "renamed");
    }

    #[test]
    fn send_request_falls_back_to_discovery_file() {
        // When CLI_MANAGER_SOCK is not set, send_request should try the discovery file.
        // We write an invalid socket path to the discovery file and verify it
        // attempts to connect (and fails with "cannot connect to").
        let saved_env = std::env::var("CLI_MANAGER_SOCK").ok();
        unsafe {
            std::env::remove_var("CLI_MANAGER_SOCK");
        }

        let discovery_path = crate::infrastructure::ipc::socket_discovery::discovery_file_path();
        let saved_discovery = std::fs::read_to_string(&discovery_path).ok();

        // Write a non-existent socket path to the discovery file
        let _ = crate::infrastructure::ipc::socket_discovery::write_socket_path(
            "/tmp/nonexistent-cm-discovery-test.sock",
        );

        let result = send_request("{}");

        // Restore env var
        if let Some(val) = saved_env {
            unsafe {
                std::env::set_var("CLI_MANAGER_SOCK", val);
            }
        }
        // Restore discovery file
        if let Some(content) = saved_discovery {
            let _ = std::fs::write(&discovery_path, content);
        } else {
            let _ = std::fs::remove_file(&discovery_path);
        }

        let err = result.unwrap_err();
        assert!(
            err.contains("cannot connect to"),
            "discovery fallback should attempt connection; got: {err}"
        );
    }
}

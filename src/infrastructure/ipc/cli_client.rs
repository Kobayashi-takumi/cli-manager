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
    let socket_path = std::env::var("CLI_MANAGER_SOCK")
        .map_err(|_| "CLI_MANAGER_SOCK not set. Are you running inside cli-manager?".to_string())?;

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
    eprintln!("  send-keys -t <id> <keys...>     Send keys to terminal");
    eprintln!("  capture-pane -t <id> [-S]        Capture terminal content");
    eprintln!("  list-windows                     List all terminals");
    eprintln!("  paste-buffer -t <id>             Paste yank buffer to terminal");
    eprintln!("  set-buffer <text>                Set yank buffer text");
    eprintln!("  show-buffer                      Show yank buffer content");
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
    fn send_request_without_env_var() {
        // Temporarily remove the env var if set
        let saved = std::env::var("CLI_MANAGER_SOCK").ok();
        unsafe {
            std::env::remove_var("CLI_MANAGER_SOCK");
        }
        let result = send_request("{}");
        // Restore
        if let Some(val) = saved {
            unsafe {
                std::env::set_var("CLI_MANAGER_SOCK", val);
            }
        }
        let err = result.unwrap_err();
        assert!(
            err.contains("CLI_MANAGER_SOCK not set"),
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
}

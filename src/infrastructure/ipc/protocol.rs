//! JSON wire protocol for IPC commands and responses.
//!
//! Converts between JSON strings and domain IPC types using serde intermediate
//! structs. The domain types themselves do not derive serde traits (domain purity).

use serde::{Deserialize, Serialize};

use crate::domain::primitive::{IpcCommand, IpcResponse, IpcResponseData, WindowInfo};

// ============================================================================
// Request (inbound) intermediate types
// ============================================================================

#[derive(Deserialize)]
struct RawRequest {
    cmd: String,
    target: Option<u32>,
    keys: Option<Vec<String>>,
    scrollback: Option<bool>,
    text: Option<String>,
    name: Option<String>,
    command: Option<String>,
}

// ============================================================================
// Response (outbound) intermediate types
// ============================================================================

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    error: String,
}

#[derive(Serialize)]
struct DataResponse<T: Serialize> {
    ok: bool,
    data: T,
}

#[derive(Serialize)]
struct CapturePaneData {
    text: String,
    cursor: CursorData,
    size: SizeData,
    name: String,
    cwd: Option<String>,
    scrollback_total: usize,
}

#[derive(Serialize)]
struct CursorData {
    row: usize,
    col: usize,
}

#[derive(Serialize)]
struct SizeData {
    rows: usize,
    cols: usize,
}

#[derive(Serialize)]
struct WindowInfoData {
    id: u32,
    name: String,
    cwd: Option<String>,
    is_active: bool,
    is_running: bool,
}

#[derive(Serialize)]
struct BufferData {
    text: Option<String>,
}

// ============================================================================
// Public API
// ============================================================================

/// Parse a JSON string into an `IpcCommand`.
///
/// Returns `Err(String)` with a human-readable message on parse failure.
pub fn parse_command(json: &str) -> Result<IpcCommand, String> {
    let raw: RawRequest =
        serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;

    match raw.cmd.as_str() {
        "send-keys" => {
            let target = raw
                .target
                .ok_or_else(|| "missing field: target".to_string())?;
            let keys = raw
                .keys
                .ok_or_else(|| "missing field: keys".to_string())?;
            Ok(IpcCommand::SendKeys { target, keys })
        }
        "capture-pane" => {
            let target = raw
                .target
                .ok_or_else(|| "missing field: target".to_string())?;
            let include_scrollback = raw.scrollback.unwrap_or(false);
            Ok(IpcCommand::CapturePane {
                target,
                include_scrollback,
            })
        }
        "list-windows" => Ok(IpcCommand::ListWindows),
        "paste-buffer" => {
            let target = raw
                .target
                .ok_or_else(|| "missing field: target".to_string())?;
            Ok(IpcCommand::PasteBuffer { target })
        }
        "set-buffer" => {
            let text = raw
                .text
                .ok_or_else(|| "missing field: text".to_string())?;
            Ok(IpcCommand::SetBuffer { text })
        }
        "show-buffer" => Ok(IpcCommand::ShowBuffer),
        "create-window" => Ok(IpcCommand::CreateWindow {
            name: raw.name,
            command: raw.command,
        }),
        "kill-window" => {
            let target = raw
                .target
                .ok_or_else(|| "missing field: target".to_string())?;
            Ok(IpcCommand::KillWindow { target })
        }
        "select-window" => {
            let target = raw
                .target
                .ok_or_else(|| "missing field: target".to_string())?;
            Ok(IpcCommand::SelectWindow { target })
        }
        "rename-window" => {
            let target = raw
                .target
                .ok_or_else(|| "missing field: target".to_string())?;
            let name = raw
                .name
                .ok_or_else(|| "missing field: name".to_string())?;
            Ok(IpcCommand::RenameWindow { target, name })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

/// Serialize an `IpcResponse` into a JSON string.
pub fn serialize_response(response: &IpcResponse) -> String {
    match response {
        IpcResponse::Ok => {
            serde_json::to_string(&OkResponse { ok: true }).expect("serialize OkResponse")
        }
        IpcResponse::Error(msg) => serde_json::to_string(&ErrorResponse {
            ok: false,
            error: msg.clone(),
        })
        .expect("serialize ErrorResponse"),
        IpcResponse::OkWithData(data) => match data {
            IpcResponseData::CapturePane {
                text,
                cursor_row,
                cursor_col,
                size_rows,
                size_cols,
                name,
                cwd,
                scrollback_total,
            } => {
                let payload = DataResponse {
                    ok: true,
                    data: CapturePaneData {
                        text: text.clone(),
                        cursor: CursorData {
                            row: *cursor_row,
                            col: *cursor_col,
                        },
                        size: SizeData {
                            rows: *size_rows,
                            cols: *size_cols,
                        },
                        name: name.clone(),
                        cwd: cwd.clone(),
                        scrollback_total: *scrollback_total,
                    },
                };
                serde_json::to_string(&payload).expect("serialize CapturePane")
            }
            IpcResponseData::ListWindows { windows } => {
                let win_data: Vec<WindowInfoData> = windows
                    .iter()
                    .map(|w| WindowInfoData {
                        id: w.id,
                        name: w.name.clone(),
                        cwd: w.cwd.clone(),
                        is_active: w.is_active,
                        is_running: w.is_running,
                    })
                    .collect();
                let payload = DataResponse {
                    ok: true,
                    data: win_data,
                };
                serde_json::to_string(&payload).expect("serialize ListWindows")
            }
            IpcResponseData::Buffer { text } => {
                let payload = DataResponse {
                    ok: true,
                    data: BufferData {
                        text: text.clone(),
                    },
                };
                serde_json::to_string(&payload).expect("serialize Buffer")
            }
            IpcResponseData::CreateWindow { id } => {
                #[derive(Serialize)]
                struct CreateWindowData {
                    id: u32,
                }
                let payload = DataResponse {
                    ok: true,
                    data: CreateWindowData { id: *id },
                };
                serde_json::to_string(&payload).expect("serialize CreateWindow")
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // ========================================================================
    // Tests: parse_command — all 6 command variants
    // ========================================================================

    #[test]
    fn parse_send_keys() {
        let json = r#"{"cmd": "send-keys", "target": 2, "keys": ["cargo test", "Enter"]}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::SendKeys {
                target: 2,
                keys: vec!["cargo test".to_string(), "Enter".to_string()],
            }
        );
    }

    #[test]
    fn parse_send_keys_empty_keys() {
        let json = r#"{"cmd": "send-keys", "target": 1, "keys": []}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::SendKeys {
                target: 1,
                keys: vec![],
            }
        );
    }

    #[test]
    fn parse_capture_pane_with_scrollback() {
        let json = r#"{"cmd": "capture-pane", "target": 3, "scrollback": true}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::CapturePane {
                target: 3,
                include_scrollback: true,
            }
        );
    }

    #[test]
    fn parse_capture_pane_without_scrollback() {
        let json = r#"{"cmd": "capture-pane", "target": 2, "scrollback": false}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::CapturePane {
                target: 2,
                include_scrollback: false,
            }
        );
    }

    #[test]
    fn parse_capture_pane_scrollback_defaults_to_false() {
        let json = r#"{"cmd": "capture-pane", "target": 2}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::CapturePane {
                target: 2,
                include_scrollback: false,
            }
        );
    }

    #[test]
    fn parse_list_windows() {
        let json = r#"{"cmd": "list-windows"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(cmd, IpcCommand::ListWindows);
    }

    #[test]
    fn parse_paste_buffer() {
        let json = r#"{"cmd": "paste-buffer", "target": 3}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(cmd, IpcCommand::PasteBuffer { target: 3 });
    }

    #[test]
    fn parse_set_buffer() {
        let json = r#"{"cmd": "set-buffer", "text": "hello world"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::SetBuffer {
                text: "hello world".to_string(),
            }
        );
    }

    #[test]
    fn parse_set_buffer_empty_text() {
        let json = r#"{"cmd": "set-buffer", "text": ""}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::SetBuffer {
                text: String::new(),
            }
        );
    }

    #[test]
    fn parse_show_buffer() {
        let json = r#"{"cmd": "show-buffer"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(cmd, IpcCommand::ShowBuffer);
    }

    // ========================================================================
    // Tests: parse_command — error cases
    // ========================================================================

    #[test]
    fn parse_missing_target_for_send_keys() {
        let json = r#"{"cmd": "send-keys", "keys": ["a"]}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: target"), "got: {err}");
    }

    #[test]
    fn parse_missing_keys_for_send_keys() {
        let json = r#"{"cmd": "send-keys", "target": 1}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: keys"), "got: {err}");
    }

    #[test]
    fn parse_missing_target_for_capture_pane() {
        let json = r#"{"cmd": "capture-pane"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: target"), "got: {err}");
    }

    #[test]
    fn parse_missing_target_for_paste_buffer() {
        let json = r#"{"cmd": "paste-buffer"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: target"), "got: {err}");
    }

    #[test]
    fn parse_missing_text_for_set_buffer() {
        let json = r#"{"cmd": "set-buffer"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: text"), "got: {err}");
    }

    #[test]
    fn parse_unknown_command() {
        let json = r#"{"cmd": "destroy-everything"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("unknown command: destroy-everything"), "got: {err}");
    }

    #[test]
    fn parse_invalid_json() {
        let err = parse_command("not json at all").unwrap_err();
        assert!(err.contains("invalid JSON"), "got: {err}");
    }

    #[test]
    fn parse_missing_cmd_field() {
        let json = r#"{"target": 1}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("invalid JSON"), "got: {err}");
    }

    // ========================================================================
    // Tests: serialize_response — all variants
    // ========================================================================

    #[test]
    fn serialize_ok() {
        let json = serialize_response(&IpcResponse::Ok);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v.get("error").is_none());
        assert!(v.get("data").is_none());
    }

    #[test]
    fn serialize_error() {
        let json = serialize_response(&IpcResponse::Error("something failed".to_string()));
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "something failed");
    }

    #[test]
    fn serialize_error_empty_message() {
        let json = serialize_response(&IpcResponse::Error(String::new()));
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "");
    }

    #[test]
    fn serialize_capture_pane() {
        let resp = IpcResponse::OkWithData(IpcResponseData::CapturePane {
            text: "$ ls\nfoo bar\n".to_string(),
            cursor_row: 24,
            cursor_col: 0,
            size_rows: 30,
            size_cols: 120,
            name: "dev server".to_string(),
            cwd: Some("/home/user/project".to_string()),
            scrollback_total: 1500,
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["text"], "$ ls\nfoo bar\n");
        assert_eq!(v["data"]["cursor"]["row"], 24);
        assert_eq!(v["data"]["cursor"]["col"], 0);
        assert_eq!(v["data"]["size"]["rows"], 30);
        assert_eq!(v["data"]["size"]["cols"], 120);
        assert_eq!(v["data"]["name"], "dev server");
        assert_eq!(v["data"]["cwd"], "/home/user/project");
        assert_eq!(v["data"]["scrollback_total"], 1500);
    }

    #[test]
    fn serialize_capture_pane_without_cwd() {
        let resp = IpcResponse::OkWithData(IpcResponseData::CapturePane {
            text: String::new(),
            cursor_row: 0,
            cursor_col: 0,
            size_rows: 24,
            size_cols: 80,
            name: "term".to_string(),
            cwd: None,
            scrollback_total: 0,
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["data"]["cwd"].is_null());
    }

    #[test]
    fn serialize_list_windows() {
        let resp = IpcResponse::OkWithData(IpcResponseData::ListWindows {
            windows: vec![
                WindowInfo {
                    id: 1,
                    name: "Terminal 1".to_string(),
                    cwd: Some("/home/user/project".to_string()),
                    is_active: true,
                    is_running: true,
                },
                WindowInfo {
                    id: 2,
                    name: "Terminal 2".to_string(),
                    cwd: None,
                    is_active: false,
                    is_running: false,
                },
            ],
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        let data = v["data"].as_array().unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0]["id"], 1);
        assert_eq!(data[0]["name"], "Terminal 1");
        assert_eq!(data[0]["cwd"], "/home/user/project");
        assert_eq!(data[0]["is_active"], true);
        assert_eq!(data[0]["is_running"], true);
        assert_eq!(data[1]["id"], 2);
        assert_eq!(data[1]["name"], "Terminal 2");
        assert!(data[1]["cwd"].is_null());
        assert_eq!(data[1]["is_active"], false);
        assert_eq!(data[1]["is_running"], false);
    }

    #[test]
    fn serialize_list_windows_empty() {
        let resp = IpcResponse::OkWithData(IpcResponseData::ListWindows {
            windows: vec![],
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        let data = v["data"].as_array().unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn serialize_buffer_with_text() {
        let resp = IpcResponse::OkWithData(IpcResponseData::Buffer {
            text: Some("hello".to_string()),
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["text"], "hello");
    }

    #[test]
    fn serialize_buffer_without_text() {
        let resp = IpcResponse::OkWithData(IpcResponseData::Buffer { text: None });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["data"]["text"].is_null());
    }

    // ========================================================================
    // Tests: Japanese text roundtrip
    // ========================================================================

    #[test]
    fn parse_japanese_text_in_set_buffer() {
        let json = r#"{"cmd": "set-buffer", "text": "こんにちは世界"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::SetBuffer {
                text: "こんにちは世界".to_string(),
            }
        );
    }

    #[test]
    fn serialize_japanese_text_in_buffer() {
        let resp = IpcResponse::OkWithData(IpcResponseData::Buffer {
            text: Some("こんにちは世界".to_string()),
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["data"]["text"], "こんにちは世界");
    }

    #[test]
    fn japanese_text_roundtrip_capture_pane() {
        let resp = IpcResponse::OkWithData(IpcResponseData::CapturePane {
            text: "$ echo テスト\nテスト\n".to_string(),
            cursor_row: 2,
            cursor_col: 0,
            size_rows: 24,
            size_cols: 80,
            name: "ターミナル1".to_string(),
            cwd: Some("/home/ユーザー/プロジェクト".to_string()),
            scrollback_total: 100,
        });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["data"]["text"], "$ echo テスト\nテスト\n");
        assert_eq!(v["data"]["name"], "ターミナル1");
        assert_eq!(v["data"]["cwd"], "/home/ユーザー/プロジェクト");
    }

    #[test]
    fn japanese_keys_in_send_keys() {
        let json = r#"{"cmd": "send-keys", "target": 1, "keys": ["日本語入力", "Enter"]}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::SendKeys {
                target: 1,
                keys: vec!["日本語入力".to_string(), "Enter".to_string()],
            }
        );
    }

    // ========================================================================
    // Tests: parse_command — new window management commands
    // ========================================================================

    #[test]
    fn parse_create_window_all_fields() {
        let json = r#"{"cmd": "create-window", "name": "my-term", "command": "/bin/bash"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::CreateWindow {
                name: Some("my-term".to_string()),
                command: Some("/bin/bash".to_string()),
            }
        );
    }

    #[test]
    fn parse_create_window_no_optional_fields() {
        let json = r#"{"cmd": "create-window"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::CreateWindow {
                name: None,
                command: None,
            }
        );
    }

    #[test]
    fn parse_kill_window() {
        let json = r#"{"cmd": "kill-window", "target": 5}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(cmd, IpcCommand::KillWindow { target: 5 });
    }

    #[test]
    fn parse_kill_window_missing_target() {
        let json = r#"{"cmd": "kill-window"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: target"), "got: {err}");
    }

    #[test]
    fn parse_select_window() {
        let json = r#"{"cmd": "select-window", "target": 2}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(cmd, IpcCommand::SelectWindow { target: 2 });
    }

    #[test]
    fn parse_select_window_missing_target() {
        let json = r#"{"cmd": "select-window"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: target"), "got: {err}");
    }

    #[test]
    fn parse_rename_window() {
        let json = r#"{"cmd": "rename-window", "target": 3, "name": "new-name"}"#;
        let cmd = parse_command(json).unwrap();
        assert_eq!(
            cmd,
            IpcCommand::RenameWindow {
                target: 3,
                name: "new-name".to_string(),
            }
        );
    }

    #[test]
    fn parse_rename_window_missing_name() {
        let json = r#"{"cmd": "rename-window", "target": 3}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: name"), "got: {err}");
    }

    #[test]
    fn parse_rename_window_missing_target() {
        let json = r#"{"cmd": "rename-window", "name": "new-name"}"#;
        let err = parse_command(json).unwrap_err();
        assert!(err.contains("missing field: target"), "got: {err}");
    }

    // ========================================================================
    // Tests: serialize_response — CreateWindow
    // ========================================================================

    #[test]
    fn serialize_create_window_response() {
        let resp = IpcResponse::OkWithData(IpcResponseData::CreateWindow { id: 3 });
        let json = serialize_response(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["id"], 3);
    }
}

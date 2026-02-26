/// IPC command types for external control of CLI Manager.
///
/// These commands represent the protocol for inter-process communication,
/// allowing external tools (e.g., a CLI client) to control terminal sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcCommand {
    /// Send keystrokes to a specific terminal.
    SendKeys { target: u32, keys: Vec<String> },
    /// Capture the visible pane content of a terminal.
    CapturePane { target: u32, include_scrollback: bool },
    /// List all terminal windows.
    ListWindows,
    /// Paste the yank buffer content into a terminal.
    PasteBuffer { target: u32 },
    /// Set the yank buffer content.
    SetBuffer { text: String },
    /// Show the current yank buffer content.
    ShowBuffer,
    /// Create a new terminal window.
    CreateWindow {
        name: Option<String>,
        command: Option<String>,
    },
    /// Kill (close) a terminal window.
    KillWindow { target: u32 },
    /// Select (activate) a terminal window.
    SelectWindow { target: u32 },
    /// Rename a terminal window.
    RenameWindow { target: u32, name: String },
    /// Send a desktop notification via CLI Manager.
    Notify { title: Option<String>, body: String },
}

/// IPC response types returned to external clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcResponse {
    /// Operation succeeded with no data.
    Ok,
    /// Operation succeeded with data payload.
    OkWithData(IpcResponseData),
    /// Operation failed with an error message.
    Error(String),
}

/// Data payloads for successful IPC responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcResponseData {
    /// Captured pane content with metadata.
    CapturePane {
        text: String,
        cursor_row: usize,
        cursor_col: usize,
        size_rows: usize,
        size_cols: usize,
        name: String,
        cwd: Option<String>,
        scrollback_total: usize,
    },
    /// List of terminal windows.
    ListWindows { windows: Vec<WindowInfo> },
    /// Yank buffer content.
    Buffer { text: Option<String> },
    /// Created terminal window ID.
    CreateWindow { id: u32 },
}

/// Information about a single terminal window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub cwd: Option<String>,
    pub is_active: bool,
    pub is_running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Tests: IpcCommand variants
    // =========================================================================

    #[test]
    fn send_keys_construction() {
        let cmd = IpcCommand::SendKeys {
            target: 1,
            keys: vec!["Enter".to_string(), "q".to_string()],
        };
        if let IpcCommand::SendKeys { target, keys } = &cmd {
            assert_eq!(*target, 1);
            assert_eq!(keys.len(), 2);
            assert_eq!(keys[0], "Enter");
            assert_eq!(keys[1], "q");
        } else {
            panic!("Expected SendKeys variant");
        }
    }

    #[test]
    fn send_keys_empty_keys_vec() {
        let cmd = IpcCommand::SendKeys {
            target: 0,
            keys: vec![],
        };
        if let IpcCommand::SendKeys { target, keys } = &cmd {
            assert_eq!(*target, 0);
            assert!(keys.is_empty());
        } else {
            panic!("Expected SendKeys variant");
        }
    }

    #[test]
    fn capture_pane_with_scrollback() {
        let cmd = IpcCommand::CapturePane {
            target: 5,
            include_scrollback: true,
        };
        if let IpcCommand::CapturePane {
            target,
            include_scrollback,
        } = &cmd
        {
            assert_eq!(*target, 5);
            assert!(*include_scrollback);
        } else {
            panic!("Expected CapturePane variant");
        }
    }

    #[test]
    fn capture_pane_without_scrollback() {
        let cmd = IpcCommand::CapturePane {
            target: 2,
            include_scrollback: false,
        };
        if let IpcCommand::CapturePane {
            target,
            include_scrollback,
        } = &cmd
        {
            assert_eq!(*target, 2);
            assert!(!*include_scrollback);
        } else {
            panic!("Expected CapturePane variant");
        }
    }

    #[test]
    fn list_windows_construction() {
        let cmd = IpcCommand::ListWindows;
        assert!(matches!(cmd, IpcCommand::ListWindows));
    }

    #[test]
    fn paste_buffer_construction() {
        let cmd = IpcCommand::PasteBuffer { target: 42 };
        if let IpcCommand::PasteBuffer { target } = &cmd {
            assert_eq!(*target, 42);
        } else {
            panic!("Expected PasteBuffer variant");
        }
    }

    #[test]
    fn set_buffer_construction() {
        let cmd = IpcCommand::SetBuffer {
            text: "hello world".to_string(),
        };
        if let IpcCommand::SetBuffer { text } = &cmd {
            assert_eq!(text, "hello world");
        } else {
            panic!("Expected SetBuffer variant");
        }
    }

    #[test]
    fn set_buffer_empty_text() {
        let cmd = IpcCommand::SetBuffer {
            text: String::new(),
        };
        if let IpcCommand::SetBuffer { text } = &cmd {
            assert!(text.is_empty());
        } else {
            panic!("Expected SetBuffer variant");
        }
    }

    #[test]
    fn show_buffer_construction() {
        let cmd = IpcCommand::ShowBuffer;
        assert!(matches!(cmd, IpcCommand::ShowBuffer));
    }

    // =========================================================================
    // Tests: IpcResponse variants
    // =========================================================================

    #[test]
    fn response_ok_construction() {
        let resp = IpcResponse::Ok;
        assert!(matches!(resp, IpcResponse::Ok));
    }

    #[test]
    fn response_ok_with_data_construction() {
        let data = IpcResponseData::Buffer { text: None };
        let resp = IpcResponse::OkWithData(data);
        assert!(matches!(resp, IpcResponse::OkWithData(_)));
    }

    #[test]
    fn response_error_construction() {
        let resp = IpcResponse::Error("something went wrong".to_string());
        if let IpcResponse::Error(msg) = &resp {
            assert_eq!(msg, "something went wrong");
        } else {
            panic!("Expected Error variant");
        }
    }

    #[test]
    fn response_error_empty_message() {
        let resp = IpcResponse::Error(String::new());
        if let IpcResponse::Error(msg) = &resp {
            assert!(msg.is_empty());
        } else {
            panic!("Expected Error variant");
        }
    }

    // =========================================================================
    // Tests: IpcResponseData variants
    // =========================================================================

    #[test]
    fn capture_pane_data_construction() {
        let data = IpcResponseData::CapturePane {
            text: "$ ls\nfoo bar\n".to_string(),
            cursor_row: 2,
            cursor_col: 0,
            size_rows: 24,
            size_cols: 80,
            name: "terminal-1".to_string(),
            cwd: Some("/home/user".to_string()),
            scrollback_total: 500,
        };
        if let IpcResponseData::CapturePane {
            text,
            cursor_row,
            cursor_col,
            size_rows,
            size_cols,
            name,
            cwd,
            scrollback_total,
        } = &data
        {
            assert_eq!(text, "$ ls\nfoo bar\n");
            assert_eq!(*cursor_row, 2);
            assert_eq!(*cursor_col, 0);
            assert_eq!(*size_rows, 24);
            assert_eq!(*size_cols, 80);
            assert_eq!(name, "terminal-1");
            assert_eq!(cwd.as_deref(), Some("/home/user"));
            assert_eq!(*scrollback_total, 500);
        } else {
            panic!("Expected CapturePane variant");
        }
    }

    #[test]
    fn capture_pane_data_without_cwd() {
        let data = IpcResponseData::CapturePane {
            text: String::new(),
            cursor_row: 0,
            cursor_col: 0,
            size_rows: 24,
            size_cols: 80,
            name: "term".to_string(),
            cwd: None,
            scrollback_total: 0,
        };
        if let IpcResponseData::CapturePane { cwd, .. } = &data {
            assert!(cwd.is_none());
        } else {
            panic!("Expected CapturePane variant");
        }
    }

    #[test]
    fn list_windows_data_construction() {
        let windows = vec![
            WindowInfo {
                id: 1,
                name: "main".to_string(),
                cwd: Some("/home".to_string()),
                is_active: true,
                is_running: true,
            },
            WindowInfo {
                id: 2,
                name: "build".to_string(),
                cwd: None,
                is_active: false,
                is_running: false,
            },
        ];
        let data = IpcResponseData::ListWindows {
            windows: windows.clone(),
        };
        if let IpcResponseData::ListWindows { windows: w } = &data {
            assert_eq!(w.len(), 2);
            assert_eq!(w[0].id, 1);
            assert_eq!(w[1].id, 2);
        } else {
            panic!("Expected ListWindows variant");
        }
    }

    #[test]
    fn list_windows_data_empty() {
        let data = IpcResponseData::ListWindows { windows: vec![] };
        if let IpcResponseData::ListWindows { windows } = &data {
            assert!(windows.is_empty());
        } else {
            panic!("Expected ListWindows variant");
        }
    }

    #[test]
    fn buffer_data_with_text() {
        let data = IpcResponseData::Buffer {
            text: Some("yanked text".to_string()),
        };
        if let IpcResponseData::Buffer { text } = &data {
            assert_eq!(text.as_deref(), Some("yanked text"));
        } else {
            panic!("Expected Buffer variant");
        }
    }

    #[test]
    fn buffer_data_without_text() {
        let data = IpcResponseData::Buffer { text: None };
        if let IpcResponseData::Buffer { text } = &data {
            assert!(text.is_none());
        } else {
            panic!("Expected Buffer variant");
        }
    }

    // =========================================================================
    // Tests: WindowInfo
    // =========================================================================

    #[test]
    fn window_info_construction() {
        let info = WindowInfo {
            id: 3,
            name: "editor".to_string(),
            cwd: Some("/tmp".to_string()),
            is_active: false,
            is_running: true,
        };
        assert_eq!(info.id, 3);
        assert_eq!(info.name, "editor");
        assert_eq!(info.cwd.as_deref(), Some("/tmp"));
        assert!(!info.is_active);
        assert!(info.is_running);
    }

    #[test]
    fn window_info_active_and_running() {
        let info = WindowInfo {
            id: 1,
            name: "main".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        assert!(info.is_active);
        assert!(info.is_running);
    }

    #[test]
    fn window_info_exited() {
        let info = WindowInfo {
            id: 2,
            name: "done".to_string(),
            cwd: None,
            is_active: false,
            is_running: false,
        };
        assert!(!info.is_active);
        assert!(!info.is_running);
    }

    #[test]
    fn window_info_equality() {
        let a = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: Some("/home".to_string()),
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: Some("/home".to_string()),
            is_active: true,
            is_running: true,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn window_info_inequality_different_id() {
        let a = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 2,
            name: "test".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn window_info_inequality_different_name() {
        let a = WindowInfo {
            id: 1,
            name: "alpha".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 1,
            name: "beta".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn window_info_inequality_different_cwd() {
        let a = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: Some("/a".to_string()),
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: Some("/b".to_string()),
            is_active: true,
            is_running: true,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn window_info_inequality_cwd_some_vs_none() {
        let a = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: Some("/a".to_string()),
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn window_info_inequality_different_is_active() {
        let a = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: None,
            is_active: false,
            is_running: true,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn window_info_inequality_different_is_running() {
        let a = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: None,
            is_active: true,
            is_running: true,
        };
        let b = WindowInfo {
            id: 1,
            name: "test".to_string(),
            cwd: None,
            is_active: true,
            is_running: false,
        };
        assert_ne!(a, b);
    }

    // =========================================================================
    // Tests: Clone
    // =========================================================================

    #[test]
    fn ipc_command_clone_equals_original() {
        let original = IpcCommand::SendKeys {
            target: 1,
            keys: vec!["a".to_string(), "b".to_string()],
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn ipc_response_clone_equals_original() {
        let original = IpcResponse::OkWithData(IpcResponseData::Buffer {
            text: Some("data".to_string()),
        });
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn window_info_clone_equals_original() {
        let original = WindowInfo {
            id: 7,
            name: "cloned".to_string(),
            cwd: Some("/usr".to_string()),
            is_active: false,
            is_running: true,
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // =========================================================================
    // Tests: Debug
    // =========================================================================

    #[test]
    fn ipc_command_debug_includes_variant_name() {
        let cmd = IpcCommand::ListWindows;
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("ListWindows"));
    }

    #[test]
    fn ipc_response_debug_includes_variant_name() {
        let resp = IpcResponse::Ok;
        let debug = format!("{:?}", resp);
        assert!(debug.contains("Ok"));
    }

    #[test]
    fn window_info_debug_includes_fields() {
        let info = WindowInfo {
            id: 1,
            name: "dbg".to_string(),
            cwd: None,
            is_active: true,
            is_running: false,
        };
        let debug = format!("{:?}", info);
        assert!(debug.contains("WindowInfo"));
        assert!(debug.contains("dbg"));
        assert!(debug.contains("is_active: true"));
        assert!(debug.contains("is_running: false"));
    }

    // =========================================================================
    // Tests: IpcCommand variant discrimination
    // =========================================================================

    #[test]
    fn different_command_variants_are_not_equal() {
        let a = IpcCommand::ListWindows;
        let b = IpcCommand::ShowBuffer;
        assert_ne!(a, b);
    }

    #[test]
    fn different_response_variants_are_not_equal() {
        let a = IpcResponse::Ok;
        let b = IpcResponse::Error("err".to_string());
        assert_ne!(a, b);
    }

    #[test]
    fn send_keys_different_targets_are_not_equal() {
        let a = IpcCommand::SendKeys {
            target: 1,
            keys: vec![],
        };
        let b = IpcCommand::SendKeys {
            target: 2,
            keys: vec![],
        };
        assert_ne!(a, b);
    }

    #[test]
    fn send_keys_different_keys_are_not_equal() {
        let a = IpcCommand::SendKeys {
            target: 1,
            keys: vec!["a".to_string()],
        };
        let b = IpcCommand::SendKeys {
            target: 1,
            keys: vec!["b".to_string()],
        };
        assert_ne!(a, b);
    }

    // =========================================================================
    // Tests: CreateWindow command
    // =========================================================================

    #[test]
    fn create_window_with_name_and_command() {
        let cmd = IpcCommand::CreateWindow {
            name: Some("my-term".to_string()),
            command: Some("/bin/bash".to_string()),
        };
        if let IpcCommand::CreateWindow { name, command } = &cmd {
            assert_eq!(name.as_deref(), Some("my-term"));
            assert_eq!(command.as_deref(), Some("/bin/bash"));
        } else {
            panic!("Expected CreateWindow variant");
        }
    }

    #[test]
    fn create_window_with_no_args() {
        let cmd = IpcCommand::CreateWindow {
            name: None,
            command: None,
        };
        if let IpcCommand::CreateWindow { name, command } = &cmd {
            assert!(name.is_none());
            assert!(command.is_none());
        } else {
            panic!("Expected CreateWindow variant");
        }
    }

    #[test]
    fn create_window_with_name_only() {
        let cmd = IpcCommand::CreateWindow {
            name: Some("editor".to_string()),
            command: None,
        };
        if let IpcCommand::CreateWindow { name, command } = &cmd {
            assert_eq!(name.as_deref(), Some("editor"));
            assert!(command.is_none());
        } else {
            panic!("Expected CreateWindow variant");
        }
    }

    #[test]
    fn create_window_with_command_only() {
        let cmd = IpcCommand::CreateWindow {
            name: None,
            command: Some("vim".to_string()),
        };
        if let IpcCommand::CreateWindow { name, command } = &cmd {
            assert!(name.is_none());
            assert_eq!(command.as_deref(), Some("vim"));
        } else {
            panic!("Expected CreateWindow variant");
        }
    }

    // =========================================================================
    // Tests: KillWindow command
    // =========================================================================

    #[test]
    fn kill_window_construction() {
        let cmd = IpcCommand::KillWindow { target: 3 };
        if let IpcCommand::KillWindow { target } = &cmd {
            assert_eq!(*target, 3);
        } else {
            panic!("Expected KillWindow variant");
        }
    }

    #[test]
    fn kill_window_target_zero() {
        let cmd = IpcCommand::KillWindow { target: 0 };
        if let IpcCommand::KillWindow { target } = &cmd {
            assert_eq!(*target, 0);
        } else {
            panic!("Expected KillWindow variant");
        }
    }

    // =========================================================================
    // Tests: SelectWindow command
    // =========================================================================

    #[test]
    fn select_window_construction() {
        let cmd = IpcCommand::SelectWindow { target: 7 };
        if let IpcCommand::SelectWindow { target } = &cmd {
            assert_eq!(*target, 7);
        } else {
            panic!("Expected SelectWindow variant");
        }
    }

    // =========================================================================
    // Tests: RenameWindow command
    // =========================================================================

    #[test]
    fn rename_window_construction() {
        let cmd = IpcCommand::RenameWindow {
            target: 2,
            name: "new-name".to_string(),
        };
        if let IpcCommand::RenameWindow { target, name } = &cmd {
            assert_eq!(*target, 2);
            assert_eq!(name, "new-name");
        } else {
            panic!("Expected RenameWindow variant");
        }
    }

    #[test]
    fn rename_window_with_empty_name() {
        let cmd = IpcCommand::RenameWindow {
            target: 1,
            name: String::new(),
        };
        if let IpcCommand::RenameWindow { target, name } = &cmd {
            assert_eq!(*target, 1);
            assert!(name.is_empty());
        } else {
            panic!("Expected RenameWindow variant");
        }
    }

    // =========================================================================
    // Tests: CreateWindow response data
    // =========================================================================

    #[test]
    fn create_window_response_data_construction() {
        let data = IpcResponseData::CreateWindow { id: 42 };
        if let IpcResponseData::CreateWindow { id } = &data {
            assert_eq!(*id, 42);
        } else {
            panic!("Expected CreateWindow response data variant");
        }
    }

    #[test]
    fn create_window_response_data_zero_id() {
        let data = IpcResponseData::CreateWindow { id: 0 };
        if let IpcResponseData::CreateWindow { id } = &data {
            assert_eq!(*id, 0);
        } else {
            panic!("Expected CreateWindow response data variant");
        }
    }

    #[test]
    fn create_window_response_wrapped_in_ok_with_data() {
        let resp = IpcResponse::OkWithData(IpcResponseData::CreateWindow { id: 5 });
        if let IpcResponse::OkWithData(IpcResponseData::CreateWindow { id }) = &resp {
            assert_eq!(*id, 5);
        } else {
            panic!("Expected OkWithData(CreateWindow) variant");
        }
    }

    // =========================================================================
    // Tests: New variant discrimination
    // =========================================================================

    #[test]
    fn create_window_not_equal_to_list_windows() {
        let a = IpcCommand::CreateWindow {
            name: None,
            command: None,
        };
        let b = IpcCommand::ListWindows;
        assert_ne!(a, b);
    }

    #[test]
    fn kill_window_not_equal_to_select_window() {
        let a = IpcCommand::KillWindow { target: 1 };
        let b = IpcCommand::SelectWindow { target: 1 };
        assert_ne!(a, b);
    }

    #[test]
    fn rename_window_different_targets_not_equal() {
        let a = IpcCommand::RenameWindow {
            target: 1,
            name: "same".to_string(),
        };
        let b = IpcCommand::RenameWindow {
            target: 2,
            name: "same".to_string(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn rename_window_different_names_not_equal() {
        let a = IpcCommand::RenameWindow {
            target: 1,
            name: "alpha".to_string(),
        };
        let b = IpcCommand::RenameWindow {
            target: 1,
            name: "beta".to_string(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn create_window_clone_equals_original() {
        let original = IpcCommand::CreateWindow {
            name: Some("test".to_string()),
            command: Some("bash".to_string()),
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn create_window_response_clone_equals_original() {
        let original = IpcResponseData::CreateWindow { id: 99 };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn create_window_debug_includes_variant_name() {
        let cmd = IpcCommand::CreateWindow {
            name: Some("dbg-test".to_string()),
            command: None,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("CreateWindow"));
        assert!(debug.contains("dbg-test"));
    }

    #[test]
    fn kill_window_debug_includes_variant_name() {
        let cmd = IpcCommand::KillWindow { target: 10 };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("KillWindow"));
        assert!(debug.contains("10"));
    }

    #[test]
    fn select_window_debug_includes_variant_name() {
        let cmd = IpcCommand::SelectWindow { target: 4 };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("SelectWindow"));
        assert!(debug.contains("4"));
    }

    #[test]
    fn rename_window_debug_includes_variant_name() {
        let cmd = IpcCommand::RenameWindow {
            target: 5,
            name: "renamed".to_string(),
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("RenameWindow"));
        assert!(debug.contains("renamed"));
    }

    #[test]
    fn create_window_response_data_not_equal_to_buffer_data() {
        let a = IpcResponseData::CreateWindow { id: 1 };
        let b = IpcResponseData::Buffer { text: None };
        assert_ne!(a, b);
    }

    // =========================================================================
    // Tests: Notify command
    // =========================================================================

    #[test]
    fn notify_with_title_and_body() {
        let cmd = IpcCommand::Notify {
            title: Some("Claude Code".to_string()),
            body: "Response complete".to_string(),
        };
        if let IpcCommand::Notify { title, body } = &cmd {
            assert_eq!(title.as_deref(), Some("Claude Code"));
            assert_eq!(body, "Response complete");
        } else {
            panic!("Expected Notify variant");
        }
    }

    #[test]
    fn notify_without_title() {
        let cmd = IpcCommand::Notify {
            title: None,
            body: "Task done".to_string(),
        };
        if let IpcCommand::Notify { title, body } = &cmd {
            assert!(title.is_none());
            assert_eq!(body, "Task done");
        } else {
            panic!("Expected Notify variant");
        }
    }

    #[test]
    fn notify_clone_equals_original() {
        let original = IpcCommand::Notify {
            title: Some("Test".to_string()),
            body: "Hello".to_string(),
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn notify_debug_includes_variant_name() {
        let cmd = IpcCommand::Notify {
            title: Some("Debug".to_string()),
            body: "test".to_string(),
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Notify"));
        assert!(debug.contains("Debug"));
        assert!(debug.contains("test"));
    }

    #[test]
    fn notify_not_equal_to_other_variants() {
        let notify = IpcCommand::Notify {
            title: None,
            body: "msg".to_string(),
        };
        let list = IpcCommand::ListWindows;
        assert_ne!(notify, list);
    }

    #[test]
    fn notify_different_bodies_not_equal() {
        let a = IpcCommand::Notify {
            title: None,
            body: "a".to_string(),
        };
        let b = IpcCommand::Notify {
            title: None,
            body: "b".to_string(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn notify_different_titles_not_equal() {
        let a = IpcCommand::Notify {
            title: Some("X".to_string()),
            body: "same".to_string(),
        };
        let b = IpcCommand::Notify {
            title: Some("Y".to_string()),
            body: "same".to_string(),
        };
        assert_ne!(a, b);
    }
}

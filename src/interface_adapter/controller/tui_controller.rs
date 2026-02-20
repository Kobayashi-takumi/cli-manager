use crate::domain::primitive::TerminalSize;
use crate::interface_adapter::port::{PtyPort, ScreenPort};
use crate::shared::error::AppError;
use crate::usecase::terminal_usecase::TerminalUsecase;

/// TUI actions (infrastructure-independent).
///
/// Each variant maps to a usecase method. The TUI layer converts key events
/// into `AppAction`s, and `TuiController::dispatch` forwards them.
pub enum AppAction {
    CreateTerminal { name: Option<String> },
    CloseTerminal,
    SelectNext,
    SelectPrev,
    SelectByIndex(usize),
    WriteToActive(Vec<u8>),
    ResizeAll(TerminalSize),
    PollAll,
    Quit,
    ToggleFocus,
    EnterScrollback,
    ExitScrollback,
    ScrollbackUp(usize),
    ScrollbackDown(usize),
    ScrollbackPageUp,
    ScrollbackPageDown,
    ScrollbackTop,
    ScrollbackBottom,
    RenameTerminal { name: String },
    OpenMemo,
    SaveMemo { text: String },
    ShowHelp,
    ToggleMiniTerminal,
    WriteToMiniTerminal(Vec<u8>),
    OpenQuickSwitcher,
    EnterScrollbackSearch,
    ScrollbackSearchNext,
    ScrollbackSearchPrev,
    ExitScrollbackSearch,
    ConfirmScrollbackSearch,
    YankLine,
    YankAllVisible,
    PasteYankBuffer,
    EnterVisualChar,
    EnterVisualLine,
}

/// Thin controller that translates `AppAction`s into usecase calls.
///
/// Lives in `interface_adapter` so it has no dependency on infrastructure
/// (ratatui, crossterm, etc.). The TUI runner owns a `TuiController` and
/// calls `dispatch` on every iteration.
pub struct TuiController<P: PtyPort, S: ScreenPort> {
    usecase: TerminalUsecase<P, S>,
}

impl<P: PtyPort, S: ScreenPort> TuiController<P, S> {
    pub fn new(usecase: TerminalUsecase<P, S>) -> Self {
        Self { usecase }
    }

    /// Dispatch an action to the underlying usecase.
    ///
    /// `size` is used when creating a terminal (passed through to pty spawn
    /// and screen buffer initialisation).
    ///
    /// `AppAction::Quit` is intentionally a no-op here; the caller (app
    /// runner) inspects the action *before* dispatching and sets its own
    /// `should_quit` flag.
    pub fn dispatch(&mut self, action: AppAction, size: TerminalSize) -> Result<(), AppError> {
        match action {
            AppAction::CreateTerminal { name } => {
                self.usecase.create_terminal(name, size)?;
            }
            AppAction::CloseTerminal => {
                self.usecase.close_active_terminal()?;
            }
            AppAction::SelectNext => self.usecase.select_next(),
            AppAction::SelectPrev => self.usecase.select_prev(),
            AppAction::SelectByIndex(idx) => self.usecase.select_by_index(idx),
            AppAction::WriteToActive(data) => {
                self.usecase.write_to_active(&data)?;
            }
            AppAction::ResizeAll(new_size) => {
                self.usecase.resize_all(new_size)?;
            }
            AppAction::PollAll => {
                self.usecase.poll_all()?;
            }
            AppAction::Quit => {}          // Handled by caller (should_quit flag)
            AppAction::ToggleFocus => {}   // Handled by caller (focus state)
            AppAction::EnterScrollback
            | AppAction::ExitScrollback
            | AppAction::ScrollbackUp(_)
            | AppAction::ScrollbackDown(_)
            | AppAction::ScrollbackPageUp
            | AppAction::ScrollbackPageDown
            | AppAction::ScrollbackTop
            | AppAction::ScrollbackBottom => {} // Handled by caller (app_runner)
            AppAction::RenameTerminal { name } => {
                if !name.is_empty() {
                    self.usecase.rename_active_terminal(name)?;
                }
            }
            AppAction::SaveMemo { text } => {
                self.usecase.set_active_memo(text)?;
            }
            AppAction::OpenMemo => {}              // Handled by caller (app_runner)
            AppAction::ShowHelp => {}              // Handled by caller (app_runner)
            AppAction::ToggleMiniTerminal => {}    // Handled by caller (app_runner)
            AppAction::WriteToMiniTerminal(_) => {} // Handled by caller (app_runner)
            AppAction::OpenQuickSwitcher => {}     // Handled by caller (app_runner)
            AppAction::EnterScrollbackSearch
            | AppAction::ScrollbackSearchNext
            | AppAction::ScrollbackSearchPrev
            | AppAction::ExitScrollbackSearch
            | AppAction::ConfirmScrollbackSearch => {} // Handled by caller (app_runner)
            AppAction::YankLine
            | AppAction::YankAllVisible
            | AppAction::PasteYankBuffer
            | AppAction::EnterVisualChar
            | AppAction::EnterVisualLine => {} // Handled by caller (app_runner)
        }
        Ok(())
    }

    /// Read accessor for UI rendering.
    ///
    /// The TUI widget layer uses this to read terminal state without owning
    /// the usecase directly.
    pub fn usecase(&self) -> &TerminalUsecase<P, S> {
        &self.usecase
    }

    /// Mutable accessor for operations that need to mutate usecase state
    /// outside of `dispatch()`, such as draining pending notifications.
    pub fn usecase_mut(&mut self) -> &mut TerminalUsecase<P, S> {
        &mut self.usecase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::primitive::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    // =========================================================================
    // Mock implementations
    // =========================================================================

    /// Records all calls made to the PtyPort methods for assertion.
    /// Uses Arc<Mutex<...>> for shared call tracking (Send + Sync safe).
    #[derive(Clone)]
    struct MockPtyPort {
        spawn_calls: Arc<Mutex<Vec<(TerminalId, String, PathBuf, TerminalSize)>>>,
        kill_calls: Arc<Mutex<Vec<TerminalId>>>,
        write_calls: Arc<Mutex<Vec<(TerminalId, Vec<u8>)>>>,
        resize_calls: Arc<Mutex<Vec<(TerminalId, TerminalSize)>>>,
        read_results: Arc<Mutex<HashMap<u32, Result<Vec<u8>, AppError>>>>,
        try_wait_results: Arc<Mutex<HashMap<u32, Result<Option<i32>, AppError>>>>,
    }

    impl MockPtyPort {
        fn new() -> Self {
            Self {
                spawn_calls: Arc::new(Mutex::new(Vec::new())),
                kill_calls: Arc::new(Mutex::new(Vec::new())),
                write_calls: Arc::new(Mutex::new(Vec::new())),
                resize_calls: Arc::new(Mutex::new(Vec::new())),
                read_results: Arc::new(Mutex::new(HashMap::new())),
                try_wait_results: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    impl PtyPort for MockPtyPort {
        fn spawn(
            &mut self,
            id: TerminalId,
            shell: &str,
            cwd: &Path,
            size: TerminalSize,
        ) -> Result<(), AppError> {
            self.spawn_calls.lock().unwrap().push((
                id,
                shell.to_string(),
                cwd.to_path_buf(),
                size,
            ));
            Ok(())
        }

        fn read(&mut self, id: TerminalId) -> Result<Vec<u8>, AppError> {
            self.read_results
                .lock()
                .unwrap()
                .remove(&id.value())
                .unwrap_or(Ok(Vec::new()))
        }

        fn write(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError> {
            self.write_calls
                .lock()
                .unwrap()
                .push((id, data.to_vec()));
            Ok(())
        }

        fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
            self.resize_calls.lock().unwrap().push((id, size));
            Ok(())
        }

        fn try_wait(&mut self, id: TerminalId) -> Result<Option<i32>, AppError> {
            self.try_wait_results
                .lock()
                .unwrap()
                .remove(&id.value())
                .unwrap_or(Ok(None))
        }

        fn kill(&mut self, id: TerminalId) -> Result<(), AppError> {
            self.kill_calls.lock().unwrap().push(id);
            Ok(())
        }
    }

    /// Records all calls made to the ScreenPort methods for assertion.
    /// All mutating trait methods take &mut self, so no interior mutability needed.
    struct MockScreenPort {
        create_calls: Vec<(TerminalId, TerminalSize)>,
        process_calls: Vec<(TerminalId, Vec<u8>)>,
        remove_calls: Vec<TerminalId>,
        resize_calls: Vec<(TerminalId, TerminalSize)>,
        cells: HashMap<u32, Vec<Vec<Cell>>>,
    }

    impl MockScreenPort {
        fn new() -> Self {
            Self {
                create_calls: Vec::new(),
                process_calls: Vec::new(),
                remove_calls: Vec::new(),
                resize_calls: Vec::new(),
                cells: HashMap::new(),
            }
        }
    }

    impl ScreenPort for MockScreenPort {
        fn create(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
            self.create_calls.push((id, size));
            let rows = size.rows as usize;
            let cols = size.cols as usize;
            let grid = vec![vec![Cell::default(); cols]; rows];
            self.cells.insert(id.value(), grid);
            Ok(())
        }

        fn process(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError> {
            self.process_calls.push((id, data.to_vec()));
            Ok(())
        }

        fn get_cells(&self, id: TerminalId) -> Result<&Vec<Vec<Cell>>, AppError> {
            self.cells
                .get(&id.value())
                .ok_or(AppError::ScreenNotFound(id))
        }

        fn get_cursor(&self, _id: TerminalId) -> Result<CursorPos, AppError> {
            Ok(CursorPos::default())
        }

        fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
            self.resize_calls.push((id, size));
            Ok(())
        }

        fn remove(&mut self, id: TerminalId) -> Result<(), AppError> {
            self.remove_calls.push(id);
            self.cells.remove(&id.value());
            Ok(())
        }

        fn get_cursor_visible(&self, _id: TerminalId) -> Result<bool, AppError> {
            Ok(true)
        }

        fn get_application_cursor_keys(&self, _id: TerminalId) -> Result<bool, AppError> {
            Ok(false)
        }

        fn get_bracketed_paste(&self, _id: TerminalId) -> Result<bool, AppError> {
            Ok(false)
        }

        fn get_cwd(&self, _id: TerminalId) -> Result<Option<String>, AppError> {
            Ok(None)
        }

        fn drain_notifications(&mut self, _id: TerminalId) -> Result<Vec<NotificationEvent>, AppError> {
            Ok(vec![])
        }

        fn set_scrollback_offset(&mut self, _id: TerminalId, _offset: usize) -> Result<(), AppError> {
            Ok(())
        }

        fn get_scrollback_offset(&self, _id: TerminalId) -> Result<usize, AppError> {
            Ok(0)
        }

        fn get_max_scrollback(&self, _id: TerminalId) -> Result<usize, AppError> {
            Ok(0)
        }

        fn is_alternate_screen(&self, _id: TerminalId) -> Result<bool, AppError> {
            Ok(false)
        }

        fn get_cursor_style(&self, _id: TerminalId) -> Result<CursorStyle, AppError> {
            Ok(CursorStyle::DefaultUserShape)
        }

        fn drain_pending_responses(&mut self, _id: TerminalId) -> Result<Vec<Vec<u8>>, AppError> {
            Ok(vec![])
        }

        fn search_scrollback(&mut self, _id: TerminalId, _query: &str) -> Result<Vec<SearchMatch>, AppError> {
            Ok(vec![])
        }

        fn get_row_cells(&mut self, _id: TerminalId, _abs_row: usize) -> Result<Vec<Cell>, AppError> {
            Ok(vec![])
        }
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    fn default_size() -> TerminalSize {
        TerminalSize::new(80, 24)
    }

    fn make_controller() -> TuiController<MockPtyPort, MockScreenPort> {
        let cwd = PathBuf::from("/tmp");
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let usecase = TerminalUsecase::new(cwd, pty, screen);
        TuiController::new(usecase)
    }

    fn make_controller_with_ports(
        pty: MockPtyPort,
        screen: MockScreenPort,
    ) -> TuiController<MockPtyPort, MockScreenPort> {
        let cwd = PathBuf::from("/tmp");
        let usecase = TerminalUsecase::new(cwd, pty, screen);
        TuiController::new(usecase)
    }

    // =========================================================================
    // Tests: new()
    // =========================================================================

    #[test]
    fn new_controller_holds_empty_usecase() {
        let ctrl = make_controller();
        assert!(ctrl.usecase().get_terminals().is_empty());
        assert_eq!(ctrl.usecase().get_active_index(), None);
    }

    // =========================================================================
    // Tests: dispatch(CreateTerminal)
    // =========================================================================

    #[test]
    fn dispatch_create_terminal_calls_usecase_create() {
        let pty = MockPtyPort::new();
        let spawn_calls = pty.spawn_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        let result = ctrl.dispatch(AppAction::CreateTerminal { name: None }, size);

        assert!(result.is_ok());
        let calls = spawn_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, TerminalId::new(1));
    }

    #[test]
    fn dispatch_create_terminal_with_name() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("my-term".to_string()),
            },
            size,
        )
        .unwrap();

        let active = ctrl.usecase().get_active_terminal().unwrap();
        assert_eq!(active.name(), "my-term");
    }

    #[test]
    fn dispatch_create_terminal_adds_terminal_to_list() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        assert_eq!(ctrl.usecase().get_terminals().len(), 1);
        assert_eq!(ctrl.usecase().get_active_index(), Some(0));
    }

    // =========================================================================
    // Tests: dispatch(CloseTerminal)
    // =========================================================================

    #[test]
    fn dispatch_close_terminal_calls_usecase_close() {
        let pty = MockPtyPort::new();
        let kill_calls = pty.kill_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();
        let result = ctrl.dispatch(AppAction::CloseTerminal, size);

        assert!(result.is_ok());
        let calls = kill_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], TerminalId::new(1));
        assert!(ctrl.usecase().get_terminals().is_empty());
    }

    #[test]
    fn dispatch_close_terminal_with_no_active_returns_error() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::CloseTerminal, size);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }

    // =========================================================================
    // Tests: dispatch(SelectNext / SelectPrev / SelectByIndex)
    // =========================================================================

    #[test]
    fn dispatch_select_next_advances_active_index() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t1".to_string()),
            },
            size,
        )
        .unwrap();
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t2".to_string()),
            },
            size,
        )
        .unwrap();
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t3".to_string()),
            },
            size,
        )
        .unwrap();

        // Active is index 2 (t3)
        assert_eq!(ctrl.usecase().get_active_index(), Some(2));

        ctrl.dispatch(AppAction::SelectNext, size).unwrap();
        assert_eq!(ctrl.usecase().get_active_index(), Some(0)); // wraps
    }

    #[test]
    fn dispatch_select_prev_retreats_active_index() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t1".to_string()),
            },
            size,
        )
        .unwrap();
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t2".to_string()),
            },
            size,
        )
        .unwrap();

        // Active is index 1 (t2); select first, then prev wraps
        ctrl.dispatch(AppAction::SelectByIndex(0), size).unwrap();
        assert_eq!(ctrl.usecase().get_active_index(), Some(0));

        ctrl.dispatch(AppAction::SelectPrev, size).unwrap();
        assert_eq!(ctrl.usecase().get_active_index(), Some(1)); // wraps
    }

    #[test]
    fn dispatch_select_by_index_sets_active() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t1".to_string()),
            },
            size,
        )
        .unwrap();
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t2".to_string()),
            },
            size,
        )
        .unwrap();
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("t3".to_string()),
            },
            size,
        )
        .unwrap();

        ctrl.dispatch(AppAction::SelectByIndex(1), size).unwrap();
        assert_eq!(ctrl.usecase().get_active_index(), Some(1));
        assert_eq!(ctrl.usecase().get_active_terminal().unwrap().name(), "t2");
    }

    #[test]
    fn dispatch_select_by_index_out_of_bounds_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();
        assert_eq!(ctrl.usecase().get_active_index(), Some(0));

        ctrl.dispatch(AppAction::SelectByIndex(99), size).unwrap();
        assert_eq!(ctrl.usecase().get_active_index(), Some(0)); // unchanged
    }

    #[test]
    fn dispatch_select_next_noop_when_no_terminals() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::SelectNext, size);
        assert!(result.is_ok());
        assert_eq!(ctrl.usecase().get_active_index(), None);
    }

    #[test]
    fn dispatch_select_prev_noop_when_no_terminals() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::SelectPrev, size);
        assert!(result.is_ok());
        assert_eq!(ctrl.usecase().get_active_index(), None);
    }

    // =========================================================================
    // Tests: dispatch(WriteToActive)
    // =========================================================================

    #[test]
    fn dispatch_write_to_active_forwards_data() {
        let pty = MockPtyPort::new();
        let write_calls = pty.write_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let data = b"hello world".to_vec();
        let result = ctrl.dispatch(AppAction::WriteToActive(data), size);

        assert!(result.is_ok());
        let calls = write_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, TerminalId::new(1));
        assert_eq!(calls[0].1, b"hello world");
    }

    #[test]
    fn dispatch_write_to_active_with_no_active_returns_error() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::WriteToActive(b"data".to_vec()), size);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }

    #[test]
    fn dispatch_write_to_active_empty_data() {
        let pty = MockPtyPort::new();
        let write_calls = pty.write_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let result = ctrl.dispatch(AppAction::WriteToActive(Vec::new()), size);

        assert!(result.is_ok());
        let calls = write_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, b"");
    }

    // =========================================================================
    // Tests: dispatch(ResizeAll)
    // =========================================================================

    #[test]
    fn dispatch_resize_all_calls_usecase_resize() {
        let pty = MockPtyPort::new();
        let pty_resize_calls = pty.resize_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();
        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let new_size = TerminalSize::new(120, 40);
        let result = ctrl.dispatch(AppAction::ResizeAll(new_size), size);

        assert!(result.is_ok());
        let calls = pty_resize_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, new_size);
        assert_eq!(calls[1].1, new_size);
    }

    #[test]
    fn dispatch_resize_all_uses_action_size_not_param() {
        let pty = MockPtyPort::new();
        let pty_resize_calls = pty.resize_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let action_size = TerminalSize::new(200, 50);
        let param_size = TerminalSize::new(80, 24);
        ctrl.dispatch(AppAction::ResizeAll(action_size), param_size)
            .unwrap();

        let calls = pty_resize_calls.lock().unwrap();
        // ResizeAll should use the size from the action, not the `size` parameter
        assert_eq!(calls[0].1, action_size);
    }

    #[test]
    fn dispatch_resize_all_on_empty_is_ok() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::ResizeAll(TerminalSize::new(120, 40)), size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: dispatch(PollAll)
    // =========================================================================

    #[test]
    fn dispatch_poll_all_calls_usecase_poll() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let result = ctrl.dispatch(AppAction::PollAll, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_poll_all_on_empty_is_ok() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::PollAll, size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: dispatch(Quit)
    // =========================================================================

    #[test]
    fn dispatch_quit_returns_ok_without_side_effects() {
        let mut ctrl = make_controller();
        let size = default_size();

        // Create a terminal first, then Quit should not affect state
        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let result = ctrl.dispatch(AppAction::Quit, size);

        assert!(result.is_ok());
        // Terminal list is unchanged -- Quit is a no-op in the controller
        assert_eq!(ctrl.usecase().get_terminals().len(), 1);
        assert_eq!(ctrl.usecase().get_active_index(), Some(0));
    }

    // =========================================================================
    // Tests: dispatch(ToggleFocus)
    // =========================================================================

    #[test]
    fn dispatch_toggle_focus_returns_ok() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::ToggleFocus, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_quit_on_empty_state_returns_ok() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::Quit, size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: usecase() accessor
    // =========================================================================

    #[test]
    fn usecase_accessor_returns_reference() {
        let ctrl = make_controller();

        let uc = ctrl.usecase();
        assert!(uc.get_terminals().is_empty());
        assert_eq!(uc.get_active_index(), None);
    }

    #[test]
    fn usecase_accessor_reflects_state_changes() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("check".to_string()),
            },
            size,
        )
        .unwrap();

        let uc = ctrl.usecase();
        assert_eq!(uc.get_terminals().len(), 1);
        assert_eq!(uc.get_active_terminal().unwrap().name(), "check");
    }

    #[test]
    fn usecase_accessor_provides_screen_port() {
        let mut ctrl = make_controller();
        let size = default_size();

        let id_val = ctrl
            .dispatch(AppAction::CreateTerminal { name: None }, size)
            .map(|_| TerminalId::new(1));
        assert!(id_val.is_ok());

        let cells = ctrl.usecase().screen_port().get_cells(TerminalId::new(1));
        assert!(cells.is_ok());
        let grid = cells.unwrap();
        assert_eq!(grid.len(), 24);
        assert_eq!(grid[0].len(), 80);
    }

    // =========================================================================
    // Tests: multiple dispatches in sequence
    // =========================================================================

    #[test]
    fn multiple_actions_in_sequence() {
        let pty = MockPtyPort::new();
        let spawn_calls = pty.spawn_calls.clone();
        let write_calls = pty.write_calls.clone();
        let screen = MockScreenPort::new();
        let mut ctrl = make_controller_with_ports(pty, screen);
        let size = default_size();

        // Create two terminals
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("a".to_string()),
            },
            size,
        )
        .unwrap();
        ctrl.dispatch(
            AppAction::CreateTerminal {
                name: Some("b".to_string()),
            },
            size,
        )
        .unwrap();

        assert_eq!(spawn_calls.lock().unwrap().len(), 2);
        assert_eq!(ctrl.usecase().get_active_index(), Some(1));

        // Select first, write to it
        ctrl.dispatch(AppAction::SelectByIndex(0), size).unwrap();
        ctrl.dispatch(AppAction::WriteToActive(b"input".to_vec()), size)
            .unwrap();

        let writes = write_calls.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, TerminalId::new(1)); // first terminal

        // Poll, then close
        ctrl.dispatch(AppAction::PollAll, size).unwrap();
        ctrl.dispatch(AppAction::CloseTerminal, size).unwrap();

        assert_eq!(ctrl.usecase().get_terminals().len(), 1);
        assert_eq!(ctrl.usecase().get_active_terminal().unwrap().name(), "b");
    }

    // =========================================================================
    // Tests: usecase_mut accessor
    // =========================================================================

    #[test]
    fn usecase_mut_returns_mutable_reference() {
        let mut ctrl = make_controller();
        let size = TerminalSize::new(80, 24);

        ctrl.dispatch(
            AppAction::CreateTerminal { name: None },
            size,
        )
        .unwrap();

        // Use usecase_mut to call take_pending_notifications
        let pending = ctrl.usecase_mut().take_pending_notifications();
        assert!(pending.is_empty());
    }

    // =========================================================================
    // Tests: dispatch(RenameTerminal)
    // =========================================================================

    #[test]
    fn dispatch_rename_terminal_changes_name() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal { name: Some("old".to_string()) },
            size,
        ).unwrap();

        ctrl.dispatch(
            AppAction::RenameTerminal { name: "new".to_string() },
            size,
        ).unwrap();

        assert_eq!(ctrl.usecase().get_active_terminal().unwrap().name(), "new");
    }

    #[test]
    fn dispatch_rename_terminal_empty_name_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal { name: Some("keep".to_string()) },
            size,
        ).unwrap();

        ctrl.dispatch(
            AppAction::RenameTerminal { name: String::new() },
            size,
        ).unwrap();

        assert_eq!(ctrl.usecase().get_active_terminal().unwrap().name(), "keep");
    }

    // =========================================================================
    // Tests: dispatch(SaveMemo)
    // =========================================================================

    #[test]
    fn dispatch_save_memo_sets_memo() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(
            AppAction::CreateTerminal { name: None },
            size,
        ).unwrap();

        ctrl.dispatch(
            AppAction::SaveMemo { text: "my note".to_string() },
            size,
        ).unwrap();

        assert_eq!(ctrl.usecase().get_active_memo().unwrap(), "my note");
    }

    #[test]
    fn dispatch_open_memo_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::OpenMemo, size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: dispatch(ShowHelp)
    // =========================================================================

    #[test]
    fn dispatch_show_help_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::ShowHelp, size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: dispatch(ToggleMiniTerminal)
    // =========================================================================

    #[test]
    fn dispatch_toggle_mini_terminal_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(AppAction::ToggleMiniTerminal, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_toggle_mini_terminal_does_not_affect_state() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let result = ctrl.dispatch(AppAction::ToggleMiniTerminal, size);
        assert!(result.is_ok());
        assert_eq!(ctrl.usecase().get_terminals().len(), 1);
        assert_eq!(ctrl.usecase().get_active_index(), Some(0));
    }

    // =========================================================================
    // Tests: dispatch(WriteToMiniTerminal)
    // =========================================================================

    #[test]
    fn dispatch_write_to_mini_terminal_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();

        let result = ctrl.dispatch(
            AppAction::WriteToMiniTerminal(b"hello".to_vec()),
            size,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_write_to_mini_terminal_does_not_affect_state() {
        let mut ctrl = make_controller();
        let size = default_size();

        ctrl.dispatch(AppAction::CreateTerminal { name: None }, size)
            .unwrap();

        let result = ctrl.dispatch(
            AppAction::WriteToMiniTerminal(b"data".to_vec()),
            size,
        );
        assert!(result.is_ok());
        assert_eq!(ctrl.usecase().get_terminals().len(), 1);
    }

    // =========================================================================
    // Tests: dispatch(OpenQuickSwitcher)
    // =========================================================================

    #[test]
    fn dispatch_open_quick_switcher_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::OpenQuickSwitcher, size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: dispatch(scrollback search actions) - Task #84
    // =========================================================================

    #[test]
    fn dispatch_enter_scrollback_search_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::EnterScrollbackSearch, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_scrollback_search_next_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::ScrollbackSearchNext, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_scrollback_search_prev_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::ScrollbackSearchPrev, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_exit_scrollback_search_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::ExitScrollbackSearch, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_confirm_scrollback_search_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::ConfirmScrollbackSearch, size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: dispatch(yank buffer actions) - Task #90
    // =========================================================================

    #[test]
    fn dispatch_yank_line_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::YankLine, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_yank_all_visible_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::YankAllVisible, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_paste_yank_buffer_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::PasteYankBuffer, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_enter_visual_char_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::EnterVisualChar, size);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_enter_visual_line_is_noop() {
        let mut ctrl = make_controller();
        let size = default_size();
        let result = ctrl.dispatch(AppAction::EnterVisualLine, size);
        assert!(result.is_ok());
    }

}

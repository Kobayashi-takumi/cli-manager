use std::path::PathBuf;

use crate::domain::model::ManagedTerminal;
use crate::domain::primitive::*;
use crate::interface_adapter::port::{PtyPort, ScreenPort};
use crate::shared::error::AppError;

pub struct TerminalUsecase<P: PtyPort, S: ScreenPort> {
    terminals: Vec<ManagedTerminal>,
    active_index: Option<usize>,
    next_id: u32,
    cwd: PathBuf,
    pty_port: P,
    screen_port: S,
    pending_notifications: Vec<(String, NotificationEvent)>,
}

impl<P: PtyPort, S: ScreenPort> TerminalUsecase<P, S> {
    pub fn new(cwd: PathBuf, pty_port: P, screen_port: S) -> Self {
        Self {
            terminals: Vec::new(),
            active_index: None,
            next_id: 1,
            cwd,
            pty_port,
            screen_port,
            pending_notifications: Vec::new(),
        }
    }

    pub fn create_terminal(
        &mut self,
        name: Option<String>,
        size: TerminalSize,
    ) -> Result<TerminalId, AppError> {
        let id = TerminalId::new(self.next_id);
        self.next_id += 1;

        let name = name.unwrap_or_else(|| format!("term-{}", id.value()));
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

        self.pty_port.spawn(id, &shell, &self.cwd, size)?;
        self.screen_port.create(id, size)?;

        let terminal = ManagedTerminal::new(id, name, self.cwd.clone());
        self.terminals.push(terminal);
        self.active_index = Some(self.terminals.len() - 1);

        Ok(id)
    }

    pub fn close_active_terminal(&mut self) -> Result<(), AppError> {
        let index = self.active_index.ok_or(AppError::NoActiveTerminal)?;
        let terminal = &self.terminals[index];
        let id = terminal.id();

        if terminal.status().is_running() {
            self.pty_port.kill(id)?;
        }
        self.screen_port.remove(id)?;
        self.terminals.remove(index);

        if self.terminals.is_empty() {
            self.active_index = None;
        } else if index >= self.terminals.len() {
            self.active_index = Some(self.terminals.len() - 1);
        }

        Ok(())
    }

    /// Poll all terminals for pty output and process exit.
    ///
    /// Uses index-based loop to avoid simultaneous mutable borrows
    /// of self.terminals and self.pty_port/screen_port.
    pub fn poll_all(&mut self) -> Result<(), AppError> {
        for i in 0..self.terminals.len() {
            // Skip already-exited terminals to avoid overwriting exit code
            if !self.terminals[i].status().is_running() {
                continue;
            }

            let id = self.terminals[i].id();

            // Read pty output
            match self.pty_port.read(id) {
                Ok(data) if !data.is_empty() => {
                    self.screen_port.process(id, &data)?;
                }
                Ok(_) => {}
                Err(_) => {
                    self.terminals[i].mark_exited(-1);
                    continue;
                }
            }

            // Check process exit
            if self.terminals[i].status().is_running()
                && let Ok(Some(code)) = self.pty_port.try_wait(id)
            {
                self.terminals[i].mark_exited(code);
            }

            // Collect notifications for all terminals (including active)
            // Desktop notifications are always forwarded; sidebar visual mark
            // is set only for non-active terminals so the user isn't distracted
            // by a "*" on the terminal they are already looking at.
            if let Ok(notifications) = self.screen_port.drain_notifications(id)
                && let Some(last) = notifications.into_iter().last()
            {
                let is_active = Some(i) == self.active_index;
                let name = self.terminals[i].name().to_string();
                // Sidebar unread mark only for non-active terminals
                if !is_active {
                    self.terminals[i].set_notification(last.clone());
                }
                // Desktop notification always forwarded
                self.pending_notifications.push((name, last));
            }
        }
        Ok(())
    }

    pub fn write_to_active(&mut self, data: &[u8]) -> Result<(), AppError> {
        let id = self
            .get_active_terminal()
            .ok_or(AppError::NoActiveTerminal)?
            .id();
        self.pty_port.write(id, data)
    }

    pub fn resize_all(&mut self, size: TerminalSize) -> Result<(), AppError> {
        for terminal in &self.terminals {
            let id = terminal.id();
            let _ = self.pty_port.resize(id, size);
            let _ = self.screen_port.resize(id, size);
        }
        Ok(())
    }

    pub fn select_next(&mut self) {
        if let Some(ref mut idx) = self.active_index
            && !self.terminals.is_empty()
        {
            *idx = (*idx + 1) % self.terminals.len();
        }
        if let Some(idx) = self.active_index {
            self.terminals[idx].clear_notification();
        }
    }

    pub fn select_prev(&mut self) {
        if let Some(ref mut idx) = self.active_index
            && !self.terminals.is_empty()
        {
            *idx = idx.checked_sub(1).unwrap_or(self.terminals.len() - 1);
        }
        if let Some(idx) = self.active_index {
            self.terminals[idx].clear_notification();
        }
    }

    pub fn select_by_index(&mut self, index: usize) {
        if index < self.terminals.len() {
            self.active_index = Some(index);
            self.terminals[index].clear_notification();
        }
    }

    pub fn get_terminals(&self) -> &[ManagedTerminal] {
        &self.terminals
    }

    pub fn get_active_index(&self) -> Option<usize> {
        self.active_index
    }

    pub fn get_active_terminal(&self) -> Option<&ManagedTerminal> {
        self.active_index.map(|i| &self.terminals[i])
    }

    /// Drain and return all pending notification events collected during `poll_all()`.
    /// Each entry is a `(terminal_name, notification_event)` pair.
    /// After calling this method, the internal pending list is cleared.
    pub fn take_pending_notifications(&mut self) -> Vec<(String, NotificationEvent)> {
        std::mem::take(&mut self.pending_notifications)
    }

    pub fn rename_active_terminal(&mut self, name: String) -> Result<(), AppError> {
        let index = self.active_index.ok_or(AppError::NoActiveTerminal)?;
        self.terminals[index].set_name(name);
        Ok(())
    }

    pub fn get_active_memo(&self) -> Result<&str, AppError> {
        let index = self.active_index.ok_or(AppError::NoActiveTerminal)?;
        Ok(self.terminals[index].memo())
    }

    pub fn set_active_memo(&mut self, memo: String) -> Result<(), AppError> {
        let index = self.active_index.ok_or(AppError::NoActiveTerminal)?;
        self.terminals[index].set_memo(memo);
        Ok(())
    }

    pub fn screen_port(&self) -> &S {
        &self.screen_port
    }

    pub fn screen_port_mut(&mut self) -> &mut S {
        &mut self.screen_port
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
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
        spawn_should_fail: bool,
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
                spawn_should_fail: false,
            }
        }

        fn with_spawn_failure(mut self) -> Self {
            self.spawn_should_fail = true;
            self
        }

        fn set_read_result(&self, id: TerminalId, result: Result<Vec<u8>, AppError>) {
            self.read_results
                .lock()
                .unwrap()
                .insert(id.value(), result);
        }

        fn set_try_wait_result(&self, id: TerminalId, result: Result<Option<i32>, AppError>) {
            self.try_wait_results
                .lock()
                .unwrap()
                .insert(id.value(), result);
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
            if self.spawn_should_fail {
                return Err(AppError::PtySpawn(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "mock spawn failure",
                )));
            }
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
    /// Only get_cells(&self) returns a reference, so cells is stored directly.
    struct MockScreenPort {
        create_calls: Vec<(TerminalId, TerminalSize)>,
        process_calls: Vec<(TerminalId, Vec<u8>)>,
        remove_calls: Vec<TerminalId>,
        resize_calls: Vec<(TerminalId, TerminalSize)>,
        cells: HashMap<u32, Vec<Vec<Cell>>>,
        create_should_fail: bool,
        pending_notifications: HashMap<u32, Vec<NotificationEvent>>,
    }

    impl MockScreenPort {
        fn new() -> Self {
            Self {
                create_calls: Vec::new(),
                process_calls: Vec::new(),
                remove_calls: Vec::new(),
                resize_calls: Vec::new(),
                cells: HashMap::new(),
                create_should_fail: false,
                pending_notifications: HashMap::new(),
            }
        }

        fn set_notifications(&mut self, id: TerminalId, events: Vec<NotificationEvent>) {
            self.pending_notifications.insert(id.value(), events);
        }
    }

    impl ScreenPort for MockScreenPort {
        fn create(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
            if self.create_should_fail {
                return Err(AppError::ScreenNotFound(id));
            }
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

        fn drain_notifications(&mut self, id: TerminalId) -> Result<Vec<NotificationEvent>, AppError> {
            Ok(self.pending_notifications.remove(&id.value()).unwrap_or_default())
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
    }

    // =========================================================================
    // Helper
    // =========================================================================

    fn default_size() -> TerminalSize {
        TerminalSize::new(80, 24)
    }

    fn make_usecase() -> TerminalUsecase<MockPtyPort, MockScreenPort> {
        let cwd = PathBuf::from("/tmp");
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        TerminalUsecase::new(cwd, pty, screen)
    }

    fn make_usecase_with_ports(
        pty: MockPtyPort,
        screen: MockScreenPort,
    ) -> TerminalUsecase<MockPtyPort, MockScreenPort> {
        let cwd = PathBuf::from("/tmp");
        TerminalUsecase::new(cwd, pty, screen)
    }

    // =========================================================================
    // Tests: new()
    // =========================================================================

    #[test]
    fn new_usecase_has_no_terminals() {
        let uc = make_usecase();
        assert!(uc.get_terminals().is_empty());
        assert_eq!(uc.get_active_index(), None);
        assert!(uc.get_active_terminal().is_none());
    }

    // =========================================================================
    // Tests: create_terminal
    // =========================================================================

    #[test]
    fn create_terminal_returns_incrementing_ids() {
        let mut uc = make_usecase();
        let size = default_size();

        let id1 = uc.create_terminal(None, size).unwrap();
        let id2 = uc.create_terminal(None, size).unwrap();
        let id3 = uc.create_terminal(None, size).unwrap();

        assert_eq!(id1.value(), 1);
        assert_eq!(id2.value(), 2);
        assert_eq!(id3.value(), 3);
    }

    #[test]
    fn create_terminal_sets_active_index_to_latest() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        assert_eq!(uc.get_active_index(), Some(0));

        uc.create_terminal(None, size).unwrap();
        assert_eq!(uc.get_active_index(), Some(1));

        uc.create_terminal(None, size).unwrap();
        assert_eq!(uc.get_active_index(), Some(2));
    }

    #[test]
    fn create_terminal_uses_provided_name() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("my-shell".to_string()), size)
            .unwrap();

        let terminal = uc.get_active_terminal().unwrap();
        assert_eq!(terminal.name(), "my-shell");
    }

    #[test]
    fn create_terminal_generates_default_name() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(None, size).unwrap();

        let terminal = uc.get_active_terminal().unwrap();
        assert_eq!(terminal.name(), "term-1");
    }

    #[test]
    fn create_terminal_calls_pty_spawn_and_screen_create() {
        let pty = MockPtyPort::new();
        let spawn_calls = pty.spawn_calls.clone();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        uc.create_terminal(None, size).unwrap();

        let calls = spawn_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, TerminalId::new(1));
        assert_eq!(calls[0].2, PathBuf::from("/tmp"));
        assert_eq!(calls[0].3, size);
    }

    #[test]
    fn create_terminal_propagates_pty_spawn_error() {
        let pty = MockPtyPort::new().with_spawn_failure();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let result = uc.create_terminal(None, size);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::PtySpawn(_)));
    }

    #[test]
    fn create_terminal_stores_terminal_in_list() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();

        let terminals = uc.get_terminals();
        assert_eq!(terminals.len(), 2);
        assert_eq!(terminals[0].name(), "t1");
        assert_eq!(terminals[1].name(), "t2");
    }

    // =========================================================================
    // Tests: close_active_terminal
    // =========================================================================

    #[test]
    fn close_active_terminal_with_no_active_returns_error() {
        let mut uc = make_usecase();

        let result = uc.close_active_terminal();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }

    #[test]
    fn close_active_terminal_removes_from_list() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // active is index 1 (t2)
        uc.close_active_terminal().unwrap();

        assert_eq!(uc.get_terminals().len(), 1);
        assert_eq!(uc.get_terminals()[0].name(), "t1");
    }

    #[test]
    fn close_active_terminal_calls_kill_for_running_terminal() {
        let pty = MockPtyPort::new();
        let kill_calls = pty.kill_calls.clone();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        uc.close_active_terminal().unwrap();

        let calls = kill_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], TerminalId::new(1));
    }

    #[test]
    fn close_active_terminal_sets_active_none_when_list_empty() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        uc.close_active_terminal().unwrap();

        assert_eq!(uc.get_active_index(), None);
        assert!(uc.get_terminals().is_empty());
    }

    #[test]
    fn close_active_terminal_adjusts_index_when_at_end() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();
        uc.create_terminal(Some("t3".to_string()), size).unwrap();

        // active_index is 2 (t3, last element)
        assert_eq!(uc.get_active_index(), Some(2));
        uc.close_active_terminal().unwrap();

        // After removing last element, index should adjust to new last
        assert_eq!(uc.get_active_index(), Some(1));
        assert_eq!(uc.get_active_terminal().unwrap().name(), "t2");
    }

    #[test]
    fn close_active_terminal_keeps_index_when_not_at_end() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();
        uc.create_terminal(Some("t3".to_string()), size).unwrap();

        // Select first terminal
        uc.select_by_index(0);
        assert_eq!(uc.get_active_index(), Some(0));

        uc.close_active_terminal().unwrap();

        // index 0 still valid, now points to what was t2
        assert_eq!(uc.get_active_index(), Some(0));
        assert_eq!(uc.get_active_terminal().unwrap().name(), "t2");
    }

    #[test]
    fn close_active_terminal_for_middle_element() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();
        uc.create_terminal(Some("t3".to_string()), size).unwrap();

        // Select middle terminal
        uc.select_by_index(1);
        uc.close_active_terminal().unwrap();

        // index 1 still valid, now points to what was t3
        assert_eq!(uc.get_terminals().len(), 2);
        assert_eq!(uc.get_active_index(), Some(1));
        assert_eq!(uc.get_active_terminal().unwrap().name(), "t3");
    }

    // =========================================================================
    // Tests: select_next / select_prev
    // =========================================================================

    #[test]
    fn select_next_wraps_around() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();
        uc.create_terminal(Some("t3".to_string()), size).unwrap();

        // Currently at index 2
        assert_eq!(uc.get_active_index(), Some(2));

        uc.select_next();
        assert_eq!(uc.get_active_index(), Some(0)); // wraps to start

        uc.select_next();
        assert_eq!(uc.get_active_index(), Some(1));

        uc.select_next();
        assert_eq!(uc.get_active_index(), Some(2));
    }

    #[test]
    fn select_prev_wraps_around() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();
        uc.create_terminal(Some("t3".to_string()), size).unwrap();

        // Select first terminal
        uc.select_by_index(0);
        assert_eq!(uc.get_active_index(), Some(0));

        uc.select_prev();
        assert_eq!(uc.get_active_index(), Some(2)); // wraps to end

        uc.select_prev();
        assert_eq!(uc.get_active_index(), Some(1));
    }

    #[test]
    fn select_next_noop_when_no_active() {
        let mut uc = make_usecase();

        uc.select_next();
        assert_eq!(uc.get_active_index(), None);
    }

    #[test]
    fn select_prev_noop_when_no_active() {
        let mut uc = make_usecase();

        uc.select_prev();
        assert_eq!(uc.get_active_index(), None);
    }

    #[test]
    fn select_next_single_terminal_stays_at_zero() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        assert_eq!(uc.get_active_index(), Some(0));

        uc.select_next();
        assert_eq!(uc.get_active_index(), Some(0));
    }

    #[test]
    fn select_prev_single_terminal_stays_at_zero() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        assert_eq!(uc.get_active_index(), Some(0));

        uc.select_prev();
        assert_eq!(uc.get_active_index(), Some(0));
    }

    // =========================================================================
    // Tests: select_by_index
    // =========================================================================

    #[test]
    fn select_by_index_valid_index() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("t1".to_string()), size).unwrap();
        uc.create_terminal(Some("t2".to_string()), size).unwrap();
        uc.create_terminal(Some("t3".to_string()), size).unwrap();

        uc.select_by_index(0);
        assert_eq!(uc.get_active_index(), Some(0));
        assert_eq!(uc.get_active_terminal().unwrap().name(), "t1");

        uc.select_by_index(2);
        assert_eq!(uc.get_active_index(), Some(2));
        assert_eq!(uc.get_active_terminal().unwrap().name(), "t3");
    }

    #[test]
    fn select_by_index_out_of_bounds_is_noop() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        assert_eq!(uc.get_active_index(), Some(0));

        uc.select_by_index(5);
        // Should remain unchanged
        assert_eq!(uc.get_active_index(), Some(0));
    }

    #[test]
    fn select_by_index_on_empty_list_is_noop() {
        let mut uc = make_usecase();

        uc.select_by_index(0);
        assert_eq!(uc.get_active_index(), None);
    }

    // =========================================================================
    // Tests: write_to_active
    // =========================================================================

    #[test]
    fn write_to_active_sends_data_to_pty() {
        let pty = MockPtyPort::new();
        let write_calls = pty.write_calls.clone();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        uc.write_to_active(b"hello").unwrap();

        let calls = write_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, TerminalId::new(1));
        assert_eq!(calls[0].1, b"hello");
    }

    #[test]
    fn write_to_active_with_no_active_returns_error() {
        let mut uc = make_usecase();

        let result = uc.write_to_active(b"hello");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }

    // =========================================================================
    // Tests: resize_all
    // =========================================================================

    #[test]
    fn resize_all_resizes_all_terminals() {
        let pty = MockPtyPort::new();
        let pty_resize_calls = pty.resize_calls.clone();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        uc.create_terminal(None, size).unwrap();
        uc.create_terminal(None, size).unwrap();

        let new_size = TerminalSize::new(120, 40);
        uc.resize_all(new_size).unwrap();

        let calls = pty_resize_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, new_size);
        assert_eq!(calls[1].1, new_size);
    }

    #[test]
    fn resize_all_on_empty_is_ok() {
        let mut uc = make_usecase();
        let new_size = TerminalSize::new(120, 40);

        let result = uc.resize_all(new_size);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: poll_all
    // =========================================================================

    #[test]
    fn poll_all_processes_pty_output_to_screen() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id = uc.create_terminal(None, size).unwrap();

        // Set up pty to return data
        uc.pty_port
            .set_read_result(id, Ok(b"hello world".to_vec()));

        uc.poll_all().unwrap();

        let calls = &uc.screen_port.process_calls;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, id);
        assert_eq!(calls[0].1, b"hello world");
    }

    #[test]
    fn poll_all_marks_terminal_exited_on_read_error() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id = uc.create_terminal(None, size).unwrap();

        // Set up pty to return read error
        uc.pty_port.set_read_result(
            id,
            Err(AppError::PtyIo {
                id,
                source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "eof"),
            }),
        );

        uc.poll_all().unwrap();

        let terminal = &uc.get_terminals()[0];
        assert!(!terminal.status().is_running());
        assert_eq!(*terminal.status(), TerminalStatus::Exited(-1));
    }

    #[test]
    fn poll_all_detects_process_exit() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id = uc.create_terminal(None, size).unwrap();

        // No data to read, but process has exited with code 0
        uc.pty_port.set_try_wait_result(id, Ok(Some(0)));

        uc.poll_all().unwrap();

        let terminal = &uc.get_terminals()[0];
        assert!(!terminal.status().is_running());
        assert_eq!(*terminal.status(), TerminalStatus::Exited(0));
    }

    #[test]
    fn poll_all_does_not_check_exit_for_already_exited() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id = uc.create_terminal(None, size).unwrap();

        // First poll: read error marks it as exited
        uc.pty_port.set_read_result(
            id,
            Err(AppError::PtyIo {
                id,
                source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "eof"),
            }),
        );
        uc.poll_all().unwrap();

        // Terminal is now exited(-1)
        assert!(!uc.get_terminals()[0].status().is_running());

        // Second poll: try_wait returns Some(42) but should not change
        // because the read error causes continue (skipping try_wait check)
        // and after that, is_running() is false so try_wait is skipped
        uc.pty_port.set_try_wait_result(id, Ok(Some(42)));
        uc.poll_all().unwrap();

        // Status should still be Exited(-1), not Exited(42)
        assert_eq!(
            *uc.get_terminals()[0].status(),
            TerminalStatus::Exited(-1)
        );
    }

    #[test]
    fn poll_all_skips_empty_read_data() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        uc.create_terminal(None, size).unwrap();

        // Default read returns empty vec, should not call screen.process
        uc.poll_all().unwrap();

        let calls = &uc.screen_port.process_calls;
        assert!(calls.is_empty());
    }

    #[test]
    fn poll_all_preserves_exit_code_across_multiple_cycles() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id = uc.create_terminal(None, size).unwrap();

        // First poll: process exits with code 0 via try_wait
        uc.pty_port.set_try_wait_result(id, Ok(Some(0)));
        uc.poll_all().unwrap();

        assert_eq!(
            *uc.get_terminals()[0].status(),
            TerminalStatus::Exited(0)
        );

        // Second poll: pty read would return an error (EIO after exit),
        // but since the terminal is already exited, it should be skipped entirely.
        // The exit code should remain 0, NOT be overwritten with -1.
        uc.pty_port.set_read_result(
            id,
            Err(AppError::PtyIo {
                id,
                source: std::io::Error::new(std::io::ErrorKind::Other, "EIO"),
            }),
        );
        uc.poll_all().unwrap();

        assert_eq!(
            *uc.get_terminals()[0].status(),
            TerminalStatus::Exited(0),
            "Exit code should be preserved as 0, not overwritten to -1"
        );

        // Third poll: even more cycles should not change exit code
        uc.poll_all().unwrap();
        assert_eq!(
            *uc.get_terminals()[0].status(),
            TerminalStatus::Exited(0)
        );
    }

    #[test]
    fn poll_all_on_empty_is_ok() {
        let mut uc = make_usecase();

        let result = uc.poll_all();
        assert!(result.is_ok());
    }

    // =========================================================================
    // Tests: screen_port accessor
    // =========================================================================

    #[test]
    fn screen_port_returns_reference() {
        let mut uc = make_usecase();
        let size = default_size();

        let id = uc.create_terminal(None, size).unwrap();

        // Verify we can access the screen port and get cells
        let cells = uc.screen_port().get_cells(id);
        assert!(cells.is_ok());
        let grid = cells.unwrap();
        assert_eq!(grid.len(), 24); // rows
        assert_eq!(grid[0].len(), 80); // cols
    }

    // =========================================================================
    // Tests: get_terminals / get_active_terminal
    // =========================================================================

    #[test]
    fn get_active_terminal_returns_correct_terminal() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("first".to_string()), size).unwrap();
        uc.create_terminal(Some("second".to_string()), size).unwrap();

        let active = uc.get_active_terminal().unwrap();
        assert_eq!(active.name(), "second");
        assert_eq!(active.id(), TerminalId::new(2));
    }

    #[test]
    fn get_terminals_returns_all_in_order() {
        let mut uc = make_usecase();
        let size = default_size();

        uc.create_terminal(Some("a".to_string()), size).unwrap();
        uc.create_terminal(Some("b".to_string()), size).unwrap();
        uc.create_terminal(Some("c".to_string()), size).unwrap();

        let terminals = uc.get_terminals();
        assert_eq!(terminals.len(), 3);
        assert_eq!(terminals[0].name(), "a");
        assert_eq!(terminals[1].name(), "b");
        assert_eq!(terminals[2].name(), "c");
    }

    // =========================================================================
    // Tests: poll_all notification collection
    // =========================================================================

    #[test]
    fn poll_all_sets_notification_on_inactive_terminal() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let size = default_size();

        let mut uc = make_usecase_with_ports(pty, screen);

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Active is index 1 (t2). Set notification for t1 (inactive).
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);

        uc.poll_all().unwrap();

        // t1 (index 0) is inactive, so it should have an unread notification
        assert!(uc.get_terminals()[0].has_unread_notification());
        assert_eq!(
            uc.get_terminals()[0].last_notification(),
            Some(&NotificationEvent::Bell)
        );
    }

    #[test]
    fn poll_all_does_not_set_notification_on_active_terminal() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();

        // Active is index 0 (t1). Set notification for active terminal.
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);

        uc.poll_all().unwrap();

        // Active terminal should NOT have notification set
        assert!(!uc.get_terminals()[0].has_unread_notification());
    }

    #[test]
    fn poll_all_uses_last_notification_when_multiple() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Multiple notifications for inactive t1 — only last should be stored
        let osc9 = NotificationEvent::Osc9 {
            message: "done".to_string(),
        };
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell, osc9.clone()]);

        uc.poll_all().unwrap();

        assert!(uc.get_terminals()[0].has_unread_notification());
        assert_eq!(uc.get_terminals()[0].last_notification(), Some(&osc9));
    }

    #[test]
    fn poll_all_empty_notifications_does_not_set_flag() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let _id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // No notifications set — default is empty vec
        uc.poll_all().unwrap();

        assert!(!uc.get_terminals()[0].has_unread_notification());
        assert!(!uc.get_terminals()[1].has_unread_notification());
    }

    // =========================================================================
    // Tests: notification clear on terminal switch
    // =========================================================================

    #[test]
    fn select_next_clears_notification_on_new_active() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Set notification on t1 (inactive, index 0)
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();
        assert!(uc.get_terminals()[0].has_unread_notification());

        // Switch to t1 via select_next (wraps from index 1 to index 0)
        uc.select_next();
        assert_eq!(uc.get_active_index(), Some(0));

        // t1 should now be cleared
        assert!(!uc.get_terminals()[0].has_unread_notification());
    }

    #[test]
    fn select_prev_clears_notification_on_new_active() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Set notification on t1 (inactive, index 0)
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();
        assert!(uc.get_terminals()[0].has_unread_notification());

        // Switch to t1 via select_prev (from index 1 to index 0)
        uc.select_prev();
        assert_eq!(uc.get_active_index(), Some(0));

        // t1 should now be cleared
        assert!(!uc.get_terminals()[0].has_unread_notification());
    }

    #[test]
    fn select_by_index_clears_notification_on_new_active() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Set notification on t1 (inactive, index 0)
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();
        assert!(uc.get_terminals()[0].has_unread_notification());

        // Switch to t1 directly
        uc.select_by_index(0);

        // t1 should now be cleared
        assert!(!uc.get_terminals()[0].has_unread_notification());
    }

    // =========================================================================
    // Tests: take_pending_notifications
    // =========================================================================

    #[test]
    fn take_pending_notifications_returns_collected_notifications() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Set notification on t1 (inactive)
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();

        let pending = uc.take_pending_notifications();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "t1");
        assert_eq!(pending[0].1, NotificationEvent::Bell);
    }

    #[test]
    fn take_pending_notifications_clears_after_take() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();

        let pending = uc.take_pending_notifications();
        assert_eq!(pending.len(), 1);

        // Second call should return empty
        let pending2 = uc.take_pending_notifications();
        assert!(pending2.is_empty());
    }

    #[test]
    fn take_pending_notifications_empty_when_no_notifications() {
        let mut uc = make_usecase();

        let pending = uc.take_pending_notifications();
        assert!(pending.is_empty());
    }

    #[test]
    fn take_pending_notifications_includes_active_terminal_for_desktop_notification() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();

        // t1 is active — desktop notification should still be forwarded
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();

        let pending = uc.take_pending_notifications();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "t1");
        assert_eq!(pending[0].1, NotificationEvent::Bell);
    }

    #[test]
    fn take_pending_notifications_does_not_set_sidebar_mark_for_active_terminal() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();

        // t1 is active — sidebar unread mark should NOT be set
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();

        assert!(!uc.terminals[0].has_unread_notification());
    }

    #[test]
    fn take_pending_notifications_multiple_terminals() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();
        let id3 = uc.create_terminal(Some("t3".to_string()), size).unwrap();

        // Active is t3 (index 2).
        uc.select_by_index(2);

        let osc9 = NotificationEvent::Osc9 {
            message: "build done".to_string(),
        };
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.screen_port.set_notifications(id3, vec![osc9.clone()]);

        uc.poll_all().unwrap();

        let pending = uc.take_pending_notifications();
        // Both t1 (inactive) and t3 (active) should be in pending for desktop notification
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].0, "t1");
        assert_eq!(pending[0].1, NotificationEvent::Bell);
        assert_eq!(pending[1].0, "t3");
        assert_eq!(pending[1].1, osc9);

        // Sidebar mark: t1 (inactive) has mark, t3 (active) does not
        assert!(uc.terminals[0].has_unread_notification());
        assert!(!uc.terminals[2].has_unread_notification());
    }

    #[test]
    fn poll_all_skips_notifications_for_exited_terminals() {
        let pty = MockPtyPort::new();
        let screen = MockScreenPort::new();
        let mut uc = make_usecase_with_ports(pty, screen);
        let size = default_size();

        let id1 = uc.create_terminal(Some("t1".to_string()), size).unwrap();
        let _id2 = uc.create_terminal(Some("t2".to_string()), size).unwrap();

        // Mark t1 as exited via read error
        uc.pty_port.set_read_result(
            id1,
            Err(AppError::PtyIo {
                id: id1,
                source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "eof"),
            }),
        );
        uc.poll_all().unwrap();
        assert!(!uc.get_terminals()[0].status().is_running());

        // Now set notification for exited t1
        uc.screen_port
            .set_notifications(id1, vec![NotificationEvent::Bell]);
        uc.poll_all().unwrap();

        // Exited terminals are skipped in poll_all, so no notification
        assert!(!uc.get_terminals()[0].has_unread_notification());
        let pending = uc.take_pending_notifications();
        assert!(pending.is_empty());
    }

    // =========================================================================
    // Tests: rename_active_terminal
    // =========================================================================

    #[test]
    fn rename_active_terminal_changes_name() {
        let mut uc = make_usecase();
        let size = default_size();
        uc.create_terminal(Some("original".to_string()), size).unwrap();

        uc.rename_active_terminal("renamed".to_string()).unwrap();

        assert_eq!(uc.get_active_terminal().unwrap().name(), "renamed");
    }

    #[test]
    fn rename_active_terminal_no_active_returns_error() {
        let mut uc = make_usecase();

        let result = uc.rename_active_terminal("name".to_string());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }

    // =========================================================================
    // Tests: get_active_memo / set_active_memo
    // =========================================================================

    #[test]
    fn get_active_memo_initial_is_empty() {
        let mut uc = make_usecase();
        let size = default_size();
        uc.create_terminal(None, size).unwrap();

        assert_eq!(uc.get_active_memo().unwrap(), "");
    }

    #[test]
    fn set_active_memo_stores_memo() {
        let mut uc = make_usecase();
        let size = default_size();
        uc.create_terminal(None, size).unwrap();

        uc.set_active_memo("my memo".to_string()).unwrap();
        assert_eq!(uc.get_active_memo().unwrap(), "my memo");
    }

    #[test]
    fn set_active_memo_no_active_returns_error() {
        let mut uc = make_usecase();

        let result = uc.set_active_memo("memo".to_string());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }

    #[test]
    fn get_active_memo_no_active_returns_error() {
        let uc = make_usecase();

        let result = uc.get_active_memo();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NoActiveTerminal));
    }
}

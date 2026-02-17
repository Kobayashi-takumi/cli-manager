use crate::domain::primitive::{Cell, CursorPos, NotificationEvent, TerminalId, TerminalSize};
use crate::shared::error::AppError;

/// Screen buffer operations port.
///
/// Defines the boundary between usecase and infrastructure for screen management.
/// Concrete implementations (e.g., VteScreenAdapter) live in infrastructure.
pub trait ScreenPort: Send + Sync {
    /// Initialize a screen buffer for the specified terminal.
    fn create(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError>;

    /// Parse ANSI byte sequences and update the screen buffer.
    fn process(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError>;

    /// Get the cell grid of the screen buffer.
    fn get_cells(&self, id: TerminalId) -> Result<&Vec<Vec<Cell>>, AppError>;

    /// Get the current cursor position.
    fn get_cursor(&self, id: TerminalId) -> Result<CursorPos, AppError>;

    /// Resize the screen buffer.
    fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError>;

    /// Remove the screen buffer.
    fn remove(&mut self, id: TerminalId) -> Result<(), AppError>;

    /// Get whether the cursor is visible (DECTCEM state).
    fn get_cursor_visible(&self, id: TerminalId) -> Result<bool, AppError>;

    /// Get whether application cursor keys mode is enabled (DECCKM state).
    fn get_application_cursor_keys(&self, id: TerminalId) -> Result<bool, AppError>;

    /// Get whether bracketed paste mode is enabled.
    fn get_bracketed_paste(&self, id: TerminalId) -> Result<bool, AppError>;

    /// Get the current working directory reported by OSC 7.
    /// Returns None if OSC 7 has not been received yet.
    fn get_cwd(&self, id: TerminalId) -> Result<Option<String>, AppError>;

    /// Drain and return all pending notification events for the specified terminal.
    /// After calling this method, the internal notification queue is cleared.
    fn drain_notifications(&mut self, id: TerminalId) -> Result<Vec<NotificationEvent>, AppError>;

    /// Set the scrollback offset for the specified terminal.
    /// 0 = live view (bottom), larger values = further into history.
    fn set_scrollback_offset(&mut self, id: TerminalId, offset: usize) -> Result<(), AppError>;

    /// Get the current scrollback offset for the specified terminal.
    /// 0 = live view (bottom).
    fn get_scrollback_offset(&self, id: TerminalId) -> Result<usize, AppError>;

    /// Get the maximum scrollback offset (total scrollback lines available).
    fn get_max_scrollback(&self, id: TerminalId) -> Result<usize, AppError>;

    /// Check whether the terminal is currently in alternate screen mode.
    fn is_alternate_screen(&self, id: TerminalId) -> Result<bool, AppError>;
}

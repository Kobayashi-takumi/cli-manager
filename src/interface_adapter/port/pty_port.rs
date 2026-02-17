use std::path::Path;

use crate::domain::primitive::{TerminalId, TerminalSize};
use crate::shared::error::AppError;

/// PTY (pseudo-terminal) operations port.
///
/// Defines the boundary between usecase and infrastructure for pty management.
/// Concrete implementations (e.g., PortablePtyAdapter) live in infrastructure.
pub trait PtyPort: Send + Sync {
    /// Spawn a shell process on a pty, associating it with the given id.
    fn spawn(
        &mut self,
        id: TerminalId,
        shell: &str,
        cwd: &Path,
        size: TerminalSize,
    ) -> Result<(), AppError>;

    /// Non-blocking read from the specified terminal's pty.
    fn read(&mut self, id: TerminalId) -> Result<Vec<u8>, AppError>;

    /// Write data to the specified terminal's pty.
    fn write(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError>;

    /// Resize the specified terminal's pty.
    fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError>;

    /// Non-blocking check for process exit. Returns exit code if exited.
    fn try_wait(&mut self, id: TerminalId) -> Result<Option<i32>, AppError>;

    /// Force-kill the process and release resources.
    fn kill(&mut self, id: TerminalId) -> Result<(), AppError>;
}

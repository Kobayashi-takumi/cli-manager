use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::domain::primitive::{TerminalId, TerminalSize};
use crate::interface_adapter::port::pty_port::PtyPort;
use crate::shared::error::AppError;

/// Internal state for a single spawned pty process.
struct PtyInstance {
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

/// Concrete implementation of `PtyPort` using the `portable-pty` crate.
///
/// Manages multiple pty instances indexed by `TerminalId`.
/// The inner `HashMap` is wrapped in a `Mutex` to satisfy the `Sync` bound
/// required by `PtyPort`. Since all `PtyPort` methods take `&mut self`,
/// exclusive access is guaranteed at the type level and the lock will never
/// actually contend.
pub struct PortablePtyAdapter {
    instances: Mutex<HashMap<TerminalId, PtyInstance>>,
}

impl PortablePtyAdapter {
    pub fn new() -> Self {
        Self {
            instances: Mutex::new(HashMap::new()),
        }
    }
}

/// Convert domain `TerminalSize` to portable-pty `PtySize`.
fn to_pty_size(size: TerminalSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Set the file descriptor to non-blocking mode using libc fcntl.
///
/// # Safety
/// Calls libc::fcntl which is an unsafe FFI function. The fd must be a valid
/// open file descriptor.
fn set_nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    // SAFETY: `fd` is a valid open file descriptor obtained from the pty master.
    // `fcntl` with `F_GETFL`/`F_SETFL` is safe for valid fds.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let result = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

impl PtyPort for PortablePtyAdapter {
    fn spawn(
        &mut self,
        id: TerminalId,
        shell: &str,
        cwd: &Path,
        size: TerminalSize,
    ) -> Result<(), AppError> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(to_pty_size(size))
            .map_err(|e| AppError::PtySpawn(io::Error::other(e.to_string())))?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);
        // Set TERM to xterm-256color — our VTE adapter now supports enough
        // escape sequences (alternate screen, scroll regions, SGR attributes,
        // line/character insert/delete, DEC private modes) to be compatible
        // with xterm-256color's terminfo capabilities.
        cmd.env("TERM", "xterm-256color");
        // Set TERM_PROGRAM so that macOS zsh loads /etc/zshrc_Apple_Terminal,
        // which registers a precmd hook to emit OSC 7 (current working directory)
        // on every directory change. This enables dynamic cwd tracking.
        cmd.env("TERM_PROGRAM", "Apple_Terminal");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| AppError::PtySpawn(io::Error::other(e.to_string())))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| AppError::PtySpawn(io::Error::other(e.to_string())))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| AppError::PtySpawn(io::Error::other(e.to_string())))?;

        // Set the reader to non-blocking mode via the master's raw fd.
        // The reader shares the same underlying fd as the master, so setting
        // O_NONBLOCK on the master fd affects reads as well.
        if let Some(raw_fd) = pair.master.as_raw_fd() {
            set_nonblocking(raw_fd).map_err(AppError::PtySpawn)?;
        }

        let instance = PtyInstance {
            master: pair.master,
            child,
            reader,
            writer,
        };

        let mut instances = self.instances.lock().expect("pty instances lock poisoned");
        instances.insert(id, instance);
        Ok(())
    }

    fn read(&mut self, id: TerminalId) -> Result<Vec<u8>, AppError> {
        let mut instances = self.instances.lock().expect("pty instances lock poisoned");
        let instance = instances
            .get_mut(&id)
            .ok_or(AppError::TerminalNotFound(id))?;

        let mut buf = [0u8; 4096];
        match instance.reader.read(&mut buf) {
            Ok(0) => Ok(Vec::new()),
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(Vec::new()),
            Err(e) => Err(AppError::PtyIo { id, source: e }),
        }
    }

    fn write(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError> {
        let mut instances = self.instances.lock().expect("pty instances lock poisoned");
        let instance = instances
            .get_mut(&id)
            .ok_or(AppError::TerminalNotFound(id))?;

        instance
            .writer
            .write_all(data)
            .map_err(|e| AppError::PtyIo { id, source: e })
    }

    fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
        let mut instances = self.instances.lock().expect("pty instances lock poisoned");
        let instance = instances
            .get_mut(&id)
            .ok_or(AppError::TerminalNotFound(id))?;

        instance
            .master
            .resize(to_pty_size(size))
            .map_err(|e| AppError::PtyIo {
                id,
                source: io::Error::other(e.to_string()),
            })
    }

    fn try_wait(&mut self, id: TerminalId) -> Result<Option<i32>, AppError> {
        let mut instances = self.instances.lock().expect("pty instances lock poisoned");
        let instance = instances
            .get_mut(&id)
            .ok_or(AppError::TerminalNotFound(id))?;

        match instance.child.try_wait() {
            Ok(Some(status)) => Ok(Some(status.exit_code() as i32)),
            Ok(None) => Ok(None),
            Err(e) => Err(AppError::PtyIo { id, source: e }),
        }
    }

    fn kill(&mut self, id: TerminalId) -> Result<(), AppError> {
        let mut instances = self.instances.lock().expect("pty instances lock poisoned");
        let mut instance = instances
            .remove(&id)
            .ok_or(AppError::TerminalNotFound(id))?;

        instance
            .child
            .kill()
            .map_err(|e| AppError::PtyIo { id, source: e })?;
        // Reap the child process to avoid zombies
        let _ = instance.child.wait();
        Ok(())
    }
}

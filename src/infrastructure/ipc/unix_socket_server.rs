//! Unix domain socket server implementing `IpcPort`.
//!
//! Provides a non-blocking Unix socket server for IPC. Each client connects,
//! sends a single JSON command (newline-delimited), receives a JSON response,
//! and the connection is closed (1-request-per-connection model).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use crate::domain::primitive::{IpcCommand, IpcResponse};
use crate::interface_adapter::port::ipc_port::{ConnectionId, IpcPort};
use crate::shared::error::AppError;

use super::protocol;

/// Non-blocking Unix domain socket server for IPC.
#[derive(Debug)]
pub struct UnixSocketServer {
    listener: UnixListener,
    socket_path: String,
    connections: HashMap<u64, UnixStream>,
    read_buffers: HashMap<u64, Vec<u8>>,
    next_conn_id: u64,
    pending_commands: Vec<(ConnectionId, IpcCommand)>,
}

/// Set a file descriptor to non-blocking mode using libc fcntl.
///
/// # Safety
/// Calls `libc::fcntl` which is an unsafe FFI function. The fd must be a valid
/// open file descriptor.
fn set_nonblocking(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    // SAFETY: `fd` is a valid open file descriptor obtained from the socket.
    // `fcntl` with `F_GETFL`/`F_SETFL` is safe for valid fds.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        if result < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Set file permissions to 0600 (owner read+write only).
///
/// # Safety
/// Calls `libc::chmod` which is an unsafe FFI function. The path must be a
/// valid null-terminated C string.
fn set_permissions_0600(path: &str) -> std::io::Result<()> {
    let c_path = std::ffi::CString::new(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // SAFETY: `c_path` is a valid null-terminated C string pointing to the socket file.
    unsafe {
        let result = libc::chmod(c_path.as_ptr(), 0o600);
        if result < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

impl UnixSocketServer {
    /// Create a new Unix socket server bound to the given path.
    ///
    /// Handles stale socket cleanup: if the socket file exists, attempts to
    /// connect to it. If the connection succeeds, another instance is running
    /// and an error is returned. If the connection fails, the stale socket
    /// file is removed and binding proceeds.
    pub fn new(socket_path: &str) -> Result<Self, AppError> {
        // Stale socket handling
        if std::path::Path::new(socket_path).exists() {
            match UnixStream::connect(socket_path) {
                Ok(_) => {
                    return Err(AppError::IpcError(format!(
                        "another instance is already running on {socket_path}"
                    )));
                }
                Err(_) => {
                    // Stale socket — remove it
                    let _ = std::fs::remove_file(socket_path);
                }
            }
        }

        let listener = UnixListener::bind(socket_path).map_err(|e| {
            AppError::IpcError(format!("failed to bind socket {socket_path}: {e}"))
        })?;

        // Set listener to non-blocking
        use std::os::fd::AsRawFd;
        set_nonblocking(listener.as_raw_fd()).map_err(|e| {
            AppError::IpcError(format!("failed to set non-blocking on listener: {e}"))
        })?;

        // Set socket file permissions to 0600
        set_permissions_0600(socket_path).map_err(|e| {
            AppError::IpcError(format!("failed to set socket permissions: {e}"))
        })?;

        Ok(Self {
            listener,
            socket_path: socket_path.to_string(),
            connections: HashMap::new(),
            read_buffers: HashMap::new(),
            next_conn_id: 1,
            pending_commands: Vec::new(),
        })
    }

    /// Accept pending connections from the listener (non-blocking).
    fn accept_connections(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    use std::os::fd::AsRawFd;
                    let _ = set_nonblocking(stream.as_raw_fd());
                    let conn_id = self.next_conn_id;
                    self.next_conn_id += 1;
                    self.connections.insert(conn_id, stream);
                    self.read_buffers.insert(conn_id, Vec::new());
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(_) => {
                    break;
                }
            }
        }
    }

    /// Read from all active connections (non-blocking).
    ///
    /// For each connection with a complete line (newline-terminated), parse the
    /// JSON command. On parse error, queue an error response. On EOF, remove
    /// the connection.
    fn read_commands(&mut self) {
        let conn_ids: Vec<u64> = self.connections.keys().copied().collect();
        let mut to_remove: Vec<u64> = Vec::new();
        let mut error_responses: Vec<(u64, String)> = Vec::new();

        for conn_id in conn_ids {
            let stream = match self.connections.get_mut(&conn_id) {
                Some(s) => s,
                None => continue,
            };

            let mut buf = [0u8; 4096];
            match stream.read(&mut buf) {
                Ok(0) => {
                    // EOF — client disconnected
                    to_remove.push(conn_id);
                }
                Ok(n) => {
                    let read_buf = self.read_buffers.entry(conn_id).or_default();
                    read_buf.extend_from_slice(&buf[..n]);

                    // Check for complete line(s)
                    while let Some(newline_pos) = read_buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = read_buf.drain(..=newline_pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                        if line.is_empty() {
                            continue;
                        }

                        match protocol::parse_command(&line) {
                            Ok(cmd) => {
                                self.pending_commands.push((ConnectionId(conn_id), cmd));
                            }
                            Err(err_msg) => {
                                error_responses.push((conn_id, err_msg));
                            }
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data available — skip
                }
                Err(_) => {
                    // Read error — remove connection
                    to_remove.push(conn_id);
                }
            }
        }

        // Send error responses and remove those connections
        for (conn_id, err_msg) in error_responses {
            let response = IpcResponse::Error(err_msg);
            let json = protocol::serialize_response(&response);
            if let Some(mut stream) = self.connections.remove(&conn_id) {
                let _ = write!(stream, "{json}\n");
            }
            self.read_buffers.remove(&conn_id);
        }

        // Remove disconnected connections
        for conn_id in to_remove {
            self.connections.remove(&conn_id);
            self.read_buffers.remove(&conn_id);
        }
    }
}

impl IpcPort for UnixSocketServer {
    fn poll_commands(&mut self) -> Vec<(ConnectionId, IpcCommand)> {
        self.accept_connections();
        self.read_commands();
        std::mem::take(&mut self.pending_commands)
    }

    fn send_response(&mut self, conn_id: ConnectionId, response: IpcResponse) {
        let json = protocol::serialize_response(&response);
        if let Some(mut stream) = self.connections.remove(&conn_id.0) {
            let _ = write!(stream, "{json}\n");
        }
        self.read_buffers.remove(&conn_id.0);
    }

    fn socket_path(&self) -> &str {
        &self.socket_path
    }

    fn shutdown(&mut self) {
        // Close all connections
        self.connections.clear();
        self.read_buffers.clear();
        // Remove socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for UnixSocketServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    /// Helper: create a unique temporary socket path for each test.
    fn temp_socket_path(suffix: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Include thread ID for additional uniqueness when tests run in parallel
        let tid = std::thread::current().id();
        format!("/tmp/cm-test-{ts}-{tid:?}-{suffix}.sock")
    }

    /// Helper: ensure socket file is cleaned up on test completion.
    struct SocketCleanup(String);
    impl Drop for SocketCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    // ========================================================================
    // Server lifecycle tests
    // ========================================================================

    #[test]
    fn server_creates_socket_file() {
        let path = temp_socket_path("creates");
        let _cleanup = SocketCleanup(path.clone());
        let _server = UnixSocketServer::new(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    #[test]
    fn server_socket_permissions_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_socket_path("perms");
        let _cleanup = SocketCleanup(path.clone());
        let _server = UnixSocketServer::new(&path).unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "socket permissions should be 0600, got {mode:o}"
        );
    }

    #[test]
    fn shutdown_removes_socket_file() {
        let path = temp_socket_path("shutdown");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
        server.shutdown();
        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn socket_path_returns_path() {
        let path = temp_socket_path("pathret");
        let _cleanup = SocketCleanup(path.clone());
        let server = UnixSocketServer::new(&path).unwrap();
        assert_eq!(server.socket_path(), path);
    }

    // ========================================================================
    // Client connect + command tests
    // ========================================================================

    #[test]
    fn client_sends_command_poll_returns_it() {
        let path = temp_socket_path("pollcmd");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        // Client connects and sends a command
        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(client, r#"{{"cmd": "list-windows"}}"#).unwrap();
        client.flush().unwrap();

        // Give OS time to deliver data
        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands = server.poll_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].1, IpcCommand::ListWindows);
    }

    #[test]
    fn client_sends_send_keys_command() {
        let path = temp_socket_path("sendkeys");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(
            client,
            r#"{{"cmd": "send-keys", "target": 2, "keys": ["cargo test", "Enter"]}}"#
        )
        .unwrap();
        client.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands = server.poll_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].1,
            IpcCommand::SendKeys {
                target: 2,
                keys: vec!["cargo test".to_string(), "Enter".to_string()],
            }
        );
    }

    #[test]
    fn response_sent_to_client() {
        let path = temp_socket_path("resp");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(client, r#"{{"cmd": "list-windows"}}"#).unwrap();
        client.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands = server.poll_commands();
        assert_eq!(commands.len(), 1);
        let conn_id = commands[0].0;

        // Send response
        server.send_response(conn_id, IpcResponse::Ok);

        // Client reads response
        let reader = BufReader::new(&client);
        let mut line = String::new();
        reader.take(1024).read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn invalid_json_returns_error_response() {
        let path = temp_socket_path("invalidjson");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(client, "not valid json").unwrap();
        client.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        // poll_commands processes the invalid JSON and sends error response automatically
        let commands = server.poll_commands();
        assert!(commands.is_empty(), "no valid commands should be returned");

        // Client reads the error response
        let reader = BufReader::new(&client);
        let mut line = String::new();
        reader.take(1024).read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("invalid JSON"));
    }

    #[test]
    fn stale_socket_cleanup() {
        let path = temp_socket_path("stale");
        let _cleanup = SocketCleanup(path.clone());

        // Create a stale socket file by binding then dropping the listener
        {
            let _listener = UnixListener::bind(&path).unwrap();
            // Drop the listener — leaves stale socket file behind
        }

        // Now we have a stale socket file (no listener)
        assert!(std::path::Path::new(&path).exists());

        // A new server should be able to bind despite the stale file
        let server = UnixSocketServer::new(&path);
        assert!(
            server.is_ok(),
            "should clean up stale socket and bind successfully"
        );
    }

    #[test]
    fn multiple_clients_sequential() {
        let path = temp_socket_path("multicli");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        // First client
        let mut client1 = UnixStream::connect(&path).unwrap();
        writeln!(client1, r#"{{"cmd": "list-windows"}}"#).unwrap();
        client1.flush().unwrap();

        // Second client
        let mut client2 = UnixStream::connect(&path).unwrap();
        writeln!(client2, r#"{{"cmd": "show-buffer"}}"#).unwrap();
        client2.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands = server.poll_commands();
        assert_eq!(commands.len(), 2);

        // Connection IDs should be distinct
        assert_ne!(commands[0].0, commands[1].0);

        // Both commands should be present (order may vary by accept order)
        let cmds: Vec<&IpcCommand> = commands.iter().map(|(_, cmd)| cmd).collect();
        assert!(cmds.contains(&&IpcCommand::ListWindows));
        assert!(cmds.contains(&&IpcCommand::ShowBuffer));
    }

    #[test]
    fn poll_with_no_clients_returns_empty() {
        let path = temp_socket_path("noclients");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        let commands = server.poll_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn connection_id_starts_at_one() {
        let path = temp_socket_path("connid");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(client, r#"{{"cmd": "show-buffer"}}"#).unwrap();
        client.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands = server.poll_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, ConnectionId(1));
    }

    #[test]
    fn connection_ids_increment() {
        let path = temp_socket_path("connidincr");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        // Connect first client and poll to ensure it gets ID 1
        let mut client1 = UnixStream::connect(&path).unwrap();
        writeln!(client1, r#"{{"cmd": "show-buffer"}}"#).unwrap();
        client1.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands1 = server.poll_commands();
        assert_eq!(commands1.len(), 1);
        assert_eq!(commands1[0].0, ConnectionId(1));

        // Connect second client and poll to ensure it gets ID 2
        let mut client2 = UnixStream::connect(&path).unwrap();
        writeln!(client2, r#"{{"cmd": "list-windows"}}"#).unwrap();
        client2.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands2 = server.poll_commands();
        assert_eq!(commands2.len(), 1);
        assert_eq!(commands2[0].0, ConnectionId(2));
    }

    #[test]
    fn another_instance_running_error() {
        let path = temp_socket_path("dupeinstance");
        let _cleanup = SocketCleanup(path.clone());

        let _server1 = UnixSocketServer::new(&path).unwrap();

        // Trying to bind a second server to the same path should fail
        let result = UnixSocketServer::new(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("another instance"),
            "expected 'another instance' error, got: {msg}"
        );
    }

    #[test]
    fn drop_removes_socket_file() {
        let path = temp_socket_path("dropclean");
        // Don't use SocketCleanup here since we test that Drop does cleanup
        {
            let _server = UnixSocketServer::new(&path).unwrap();
            assert!(std::path::Path::new(&path).exists());
        }
        // After drop, socket file should be removed
        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn send_response_with_data() {
        let path = temp_socket_path("respdata");
        let _cleanup = SocketCleanup(path.clone());
        let mut server = UnixSocketServer::new(&path).unwrap();

        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(client, r#"{{"cmd": "show-buffer"}}"#).unwrap();
        client.flush().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let commands = server.poll_commands();
        assert_eq!(commands.len(), 1);
        let conn_id = commands[0].0;

        let response =
            IpcResponse::OkWithData(crate::domain::primitive::IpcResponseData::Buffer {
                text: Some("yanked text".to_string()),
            });
        server.send_response(conn_id, response);

        let reader = BufReader::new(&client);
        let mut line = String::new();
        reader.take(4096).read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["text"], "yanked text");
    }
}

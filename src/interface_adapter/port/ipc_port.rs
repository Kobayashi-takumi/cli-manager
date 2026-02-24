use crate::domain::primitive::{IpcCommand, IpcResponse};

/// Unique identifier for an IPC client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Port trait for IPC server abstraction.
///
/// Usecase/app_runner interacts with the IPC server through this trait,
/// receiving commands from external clients and sending responses back.
pub trait IpcPort: Send + Sync {
    /// Non-blocking: drain received IPC commands and return them.
    fn poll_commands(&mut self) -> Vec<(ConnectionId, IpcCommand)>;

    /// Send a response to the specified connection.
    fn send_response(&mut self, conn_id: ConnectionId, response: IpcResponse);

    /// Return the socket path (for exposing to child processes via env var).
    fn socket_path(&self) -> &str;

    /// Cleanup (close connections, delete socket file).
    fn shutdown(&mut self);
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub struct MockIpcPort {
        pub pending_commands: Vec<(ConnectionId, IpcCommand)>,
        pub sent_responses: Vec<(ConnectionId, IpcResponse)>,
        pub path: String,
    }

    impl MockIpcPort {
        pub fn new() -> Self {
            Self {
                pending_commands: Vec::new(),
                sent_responses: Vec::new(),
                path: "/tmp/test-cli-manager.sock".to_string(),
            }
        }
    }

    impl IpcPort for MockIpcPort {
        fn poll_commands(&mut self) -> Vec<(ConnectionId, IpcCommand)> {
            std::mem::take(&mut self.pending_commands)
        }

        fn send_response(&mut self, conn_id: ConnectionId, response: IpcResponse) {
            self.sent_responses.push((conn_id, response));
        }

        fn socket_path(&self) -> &str {
            &self.path
        }

        fn shutdown(&mut self) {
            // no-op for mock
        }
    }

    #[test]
    fn mock_poll_commands_returns_pending() {
        let mut mock = MockIpcPort::new();
        mock.pending_commands.push((
            ConnectionId(1),
            IpcCommand::ListWindows,
        ));
        mock.pending_commands.push((
            ConnectionId(2),
            IpcCommand::ShowBuffer,
        ));

        let commands = mock.poll_commands();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].0, ConnectionId(1));
        assert_eq!(commands[0].1, IpcCommand::ListWindows);
        assert_eq!(commands[1].0, ConnectionId(2));
        assert_eq!(commands[1].1, IpcCommand::ShowBuffer);
    }

    #[test]
    fn mock_poll_commands_drains() {
        let mut mock = MockIpcPort::new();
        mock.pending_commands.push((
            ConnectionId(1),
            IpcCommand::ListWindows,
        ));

        let _ = mock.poll_commands();
        let commands = mock.poll_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn mock_send_response_stores() {
        let mut mock = MockIpcPort::new();
        mock.send_response(ConnectionId(1), IpcResponse::Ok);
        mock.send_response(ConnectionId(2), IpcResponse::Error("test".to_string()));

        assert_eq!(mock.sent_responses.len(), 2);
        assert_eq!(mock.sent_responses[0].0, ConnectionId(1));
        assert_eq!(mock.sent_responses[0].1, IpcResponse::Ok);
        assert_eq!(mock.sent_responses[1].0, ConnectionId(2));
        assert_eq!(mock.sent_responses[1].1, IpcResponse::Error("test".to_string()));
    }

    #[test]
    fn mock_socket_path() {
        let mock = MockIpcPort::new();
        assert_eq!(mock.socket_path(), "/tmp/test-cli-manager.sock");
    }

    #[test]
    fn connection_id_equality() {
        assert_eq!(ConnectionId(1), ConnectionId(1));
        assert_ne!(ConnectionId(1), ConnectionId(2));
    }

    #[test]
    fn connection_id_copy() {
        let id = ConnectionId(42);
        let id2 = id;
        assert_eq!(id, id2);
    }
}

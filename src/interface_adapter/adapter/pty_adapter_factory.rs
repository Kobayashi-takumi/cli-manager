use crate::infrastructure::pty::PortablePtyAdapter;

/// Creates a concrete PtyPort implementation.
/// Future: can be swapped for a stub/mock adapter for testing.
pub fn create_pty_adapter() -> PortablePtyAdapter {
    PortablePtyAdapter::new()
}

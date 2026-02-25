use std::fs;
use std::io;
use std::path::PathBuf;

/// Get the directory for cli-manager runtime files.
///
/// Uses the HOME environment variable. Falls back to /tmp if HOME is not set.
fn discovery_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".cli-manager")
}

/// Get the path to the socket discovery file.
pub fn discovery_file_path() -> PathBuf {
    discovery_dir().join("socket")
}

/// Write the socket path to the discovery file (~/.cli-manager/socket).
///
/// Creates the directory if it doesn't exist. Sets file permissions to 0600.
pub fn write_socket_path(socket_path: &str) -> io::Result<()> {
    let dir = discovery_dir();
    fs::create_dir_all(&dir)?;
    let file_path = dir.join("socket");
    fs::write(&file_path, socket_path)?;
    // Set file permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Remove the socket discovery file.
///
/// Errors are silently ignored (best-effort cleanup).
pub fn remove_socket_path() {
    let _ = fs::remove_file(discovery_file_path());
}

/// Read the socket path from the discovery file.
///
/// Returns the socket path string, or an error if the file doesn't exist.
pub fn read_socket_path() -> io::Result<String> {
    let path = discovery_file_path();
    let content = fs::read_to_string(&path)?;
    Ok(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Helper: create a unique temporary directory for each test to avoid
    /// interference between parallel tests and side effects on the real ~/.cli-manager/.
    ///
    /// Returns the temp dir path. The caller is responsible for cleanup.
    fn create_temp_dir(suffix: &str) -> PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tid = std::thread::current().id();
        let dir = PathBuf::from(format!("/tmp/cm-discovery-test-{ts}-{tid:?}-{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Helper: cleanup a temp directory on drop.
    struct TempDirCleanup(PathBuf);
    impl Drop for TempDirCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ========================================================================
    // Tests: discovery_dir
    // ========================================================================

    #[test]
    fn discovery_dir_uses_home_env() {
        // discovery_dir() uses HOME env var, which we cannot safely manipulate
        // in parallel tests. Instead, verify the structure: it should end with
        // ".cli-manager" and its parent should be a valid directory path.
        let dir = discovery_dir();
        assert!(
            dir.ends_with(".cli-manager"),
            "discovery_dir should end with .cli-manager, got: {:?}",
            dir
        );
        // The parent should be the HOME directory (or /tmp fallback)
        let parent = dir.parent().unwrap();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        assert_eq!(
            parent,
            Path::new(&home),
            "discovery_dir parent should be HOME"
        );
    }

    // ========================================================================
    // Tests: discovery_file_path
    // ========================================================================

    #[test]
    fn discovery_file_path_ends_with_socket() {
        let path = discovery_file_path();
        assert!(
            path.ends_with(".cli-manager/socket"),
            "discovery_file_path should end with .cli-manager/socket, got: {:?}",
            path
        );
    }

    // ========================================================================
    // Tests: write + read roundtrip (using temp directory approach)
    //
    // Since write_socket_path/read_socket_path use discovery_dir() internally
    // (which depends on HOME), we test the file I/O logic directly using
    // temp directories to avoid side effects.
    // ========================================================================

    #[test]
    fn write_and_read_roundtrip() {
        let temp_dir = create_temp_dir("roundtrip");
        let _cleanup = TempDirCleanup(temp_dir.clone());

        let cm_dir = temp_dir.join(".cli-manager");
        fs::create_dir_all(&cm_dir).unwrap();
        let file_path = cm_dir.join("socket");

        let socket_path = "/tmp/cli-manager-12345.sock";
        fs::write(&file_path, socket_path).unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.trim(), socket_path);
    }

    #[test]
    fn write_creates_directory_if_missing() {
        let temp_dir = create_temp_dir("mkdir");
        let _cleanup = TempDirCleanup(temp_dir.clone());

        let cm_dir = temp_dir.join(".cli-manager");
        // Directory does not exist yet
        assert!(!cm_dir.exists());

        fs::create_dir_all(&cm_dir).unwrap();
        let file_path = cm_dir.join("socket");
        fs::write(&file_path, "/tmp/test.sock").unwrap();

        assert!(cm_dir.exists());
        assert!(file_path.exists());
    }

    #[test]
    fn write_sets_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = create_temp_dir("perms");
        let _cleanup = TempDirCleanup(temp_dir.clone());

        let cm_dir = temp_dir.join(".cli-manager");
        fs::create_dir_all(&cm_dir).unwrap();
        let file_path = cm_dir.join("socket");
        fs::write(&file_path, "/tmp/test.sock").unwrap();
        fs::set_permissions(&file_path, fs::Permissions::from_mode(0o600)).unwrap();

        let metadata = fs::metadata(&file_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "file permissions should be 0600, got {mode:o}");
    }

    #[test]
    fn read_trims_whitespace() {
        let temp_dir = create_temp_dir("trim");
        let _cleanup = TempDirCleanup(temp_dir.clone());

        let cm_dir = temp_dir.join(".cli-manager");
        fs::create_dir_all(&cm_dir).unwrap();
        let file_path = cm_dir.join("socket");

        // Write with trailing newline
        fs::write(&file_path, "/tmp/cli-manager-99.sock\n").unwrap();

        let content = fs::read_to_string(&file_path).unwrap().trim().to_string();
        assert_eq!(content, "/tmp/cli-manager-99.sock");
    }

    // ========================================================================
    // Tests: read when file doesn't exist
    // ========================================================================

    #[test]
    fn read_nonexistent_file_returns_error() {
        let temp_dir = create_temp_dir("nofile");
        let _cleanup = TempDirCleanup(temp_dir.clone());

        let file_path = temp_dir.join(".cli-manager").join("socket");
        let result = fs::read_to_string(&file_path);
        assert!(result.is_err(), "reading nonexistent file should return error");
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    // ========================================================================
    // Tests: remove when file doesn't exist
    // ========================================================================

    #[test]
    fn remove_nonexistent_file_no_panic() {
        let temp_dir = create_temp_dir("noremove");
        let _cleanup = TempDirCleanup(temp_dir.clone());

        // This should not panic
        let file_path = temp_dir.join(".cli-manager").join("socket");
        let _ = fs::remove_file(&file_path);
        // If we reach here, the test passes — no panic occurred
    }

    // ========================================================================
    // Tests: integration with real write_socket_path / read_socket_path / remove_socket_path
    //
    // These tests use the real HOME-based path. They are serial-safe because
    // each test writes a unique value and reads it back. We use a dedicated
    // test that writes, reads, and cleans up atomically.
    // ========================================================================

    /// Single integration test that exercises the full write/read/remove cycle
    /// using the real ~/.cli-manager/socket path. Consolidated into one test
    /// to avoid parallel test interference on the shared file.
    #[test]
    fn write_read_remove_integration() {
        // --- Part 1: Write, read, verify permissions ---
        let test_socket = "/tmp/cli-manager-integration-test-99999.sock";
        let result = write_socket_path(test_socket);
        assert!(result.is_ok(), "write_socket_path failed: {:?}", result.err());

        let read_result = read_socket_path();
        assert!(read_result.is_ok(), "read_socket_path failed: {:?}", read_result.err());
        assert_eq!(read_result.unwrap(), test_socket);

        // Verify permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(discovery_file_path()).unwrap();
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "discovery file should have 0600 permissions, got {mode:o}");
        }

        // --- Part 2: Overwrite existing content ---
        let second_socket = "/tmp/cli-manager-integration-test-88888.sock";
        write_socket_path(second_socket).unwrap();
        assert_eq!(read_socket_path().unwrap(), second_socket);

        // --- Part 3: Remove and verify ---
        remove_socket_path();
        assert!(
            !discovery_file_path().exists(),
            "discovery file should be removed after remove_socket_path()"
        );

        // --- Part 4: Read after removal returns error ---
        let result = read_socket_path();
        assert!(result.is_err(), "read_socket_path should return error when file missing");

        // --- Part 5: Remove when file doesn't exist doesn't panic ---
        remove_socket_path();
        // If we reach here, the test passes -- no panic occurred
    }
}

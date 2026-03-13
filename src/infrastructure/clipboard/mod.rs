use std::io::Write;
use std::process::{Command, Stdio};

/// Copy text to OS clipboard via pbcopy (macOS).
/// Best-effort: errors are silently ignored.
pub fn copy_to_clipboard(text: &str) {
    let Ok(mut child) = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_to_clipboard_empty_string_does_not_panic() {
        copy_to_clipboard("");
    }

    #[test]
    fn copy_to_clipboard_normal_string_does_not_panic() {
        copy_to_clipboard("hello world");
    }

    #[test]
    fn copy_to_clipboard_long_string_does_not_panic() {
        let long_text = "a".repeat(100_000);
        copy_to_clipboard(&long_text);
    }

    #[test]
    fn copy_to_clipboard_multibyte_string_does_not_panic() {
        copy_to_clipboard("日本語テスト 🎉");
    }

    #[test]
    fn copy_to_clipboard_newlines_does_not_panic() {
        copy_to_clipboard("line1\nline2\nline3");
    }

    #[test]
    #[ignore] // Requires macOS with pbcopy/pbpaste available
    fn copy_to_clipboard_roundtrip_via_pbpaste() {
        let test_text = "cli_manager_clipboard_test_12345";
        copy_to_clipboard(test_text);

        let output = Command::new("pbpaste")
            .output()
            .expect("pbpaste should be available on macOS");
        let pasted = String::from_utf8_lossy(&output.stdout);
        assert_eq!(pasted, test_text);
    }
}

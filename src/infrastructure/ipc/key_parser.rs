/// Parses a sequence of key strings into a byte vector.
/// Special key names are converted to their control byte equivalents.
/// Plain text is converted to UTF-8 bytes.
///
/// Key mappings (case-insensitive):
/// - `Enter`  -> `\r`
/// - `Tab`    -> `\t`
/// - `Escape` -> `\x1b`
/// - `Space`  -> `\x20`
/// - `BSpace` -> `\x7f`
/// - `C-a` through `C-z` -> `\x01` through `\x1a`
///
/// Processing rules:
/// - Each string in the array is processed independently
/// - Special keys are matched case-insensitively
/// - `C-` prefix: the character after `C-` must be a single lowercase letter a-z (case-insensitive)
/// - If a string doesn't match any special key, treat it as literal text (UTF-8 bytes)
/// - `C-` followed by a non-letter returns an error
/// - Empty input array returns Ok(empty vec)
pub fn parse_keys(keys: &[String]) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();

    for key in keys {
        let bytes = parse_single_key(key)?;
        result.extend_from_slice(&bytes);
    }

    Ok(result)
}

/// Parse a single key string into bytes.
/// Returns the special key byte(s) if it matches a known key name,
/// otherwise returns the literal UTF-8 bytes of the string.
fn parse_single_key(key: &str) -> Result<Vec<u8>, String> {
    let lower = key.to_ascii_lowercase();

    // Check named special keys (case-insensitive)
    match lower.as_str() {
        "enter" => return Ok(vec![b'\r']),
        "tab" => return Ok(vec![b'\t']),
        "escape" => return Ok(vec![0x1b]),
        "space" => return Ok(vec![0x20]),
        "bspace" => return Ok(vec![0x7f]),
        _ => {}
    }

    // Check C- prefix (ctrl key combinations)
    if let Some(rest) = lower.strip_prefix("c-") {
        let mut chars = rest.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) if ch.is_ascii_lowercase() => {
                // Convert a-z to control byte 0x01-0x1a
                let ctrl_byte = ch as u8 - b'a' + 1;
                return Ok(vec![ctrl_byte]);
            }
            _ => {
                return Err(format!("invalid ctrl key: {key}"));
            }
        }
    }

    // Literal text: return UTF-8 bytes
    Ok(key.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_text_with_enter() {
        let keys = vec!["cargo test".to_string(), "Enter".to_string()];
        let result = parse_keys(&keys).unwrap();
        let mut expected = b"cargo test".to_vec();
        expected.push(b'\r');
        assert_eq!(result, expected);
    }

    #[test]
    fn ctrl_c() {
        let keys = vec!["C-c".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x03]);
    }

    #[test]
    fn ctrl_d() {
        let keys = vec!["C-d".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x04]);
    }

    #[test]
    fn escape_then_q() {
        let keys = vec!["Escape".to_string(), "q".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x1b, b'q']);
    }

    #[test]
    fn tab_key() {
        let keys = vec!["Tab".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![b'\t']);
    }

    #[test]
    fn bspace_key() {
        let keys = vec!["BSpace".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x7f]);
    }

    #[test]
    fn space_key() {
        let keys = vec!["Space".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x20]);
    }

    #[test]
    fn ctrl_a() {
        let keys = vec!["C-a".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x01]);
    }

    #[test]
    fn ctrl_z() {
        let keys = vec!["C-z".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x1a]);
    }

    #[test]
    fn case_insensitive_enter_uppercase() {
        let keys = vec!["ENTER".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![b'\r']);
    }

    #[test]
    fn case_insensitive_enter_lowercase() {
        let keys = vec!["enter".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![b'\r']);
    }

    #[test]
    fn case_insensitive_ctrl_lowercase_prefix() {
        let keys = vec!["c-c".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x03]);
    }

    #[test]
    fn case_insensitive_ctrl_uppercase_letter() {
        let keys = vec!["C-C".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, vec![0x03]);
    }

    #[test]
    fn empty_array() {
        let keys: Vec<String> = vec![];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, Vec::<u8>::new());
    }

    #[test]
    fn japanese_text() {
        let keys = vec!["テスト".to_string()];
        let result = parse_keys(&keys).unwrap();
        assert_eq!(result, "テスト".as_bytes().to_vec());
    }

    #[test]
    fn ctrl_digit_error() {
        let keys = vec!["C-1".to_string()];
        let result = parse_keys(&keys);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "invalid ctrl key: C-1");
    }

    #[test]
    fn ctrl_no_letter_error() {
        let keys = vec!["C-".to_string()];
        let result = parse_keys(&keys);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "invalid ctrl key: C-");
    }
}

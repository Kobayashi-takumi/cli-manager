/// Parse an OSC 7 URI and extract the path component.
/// Input format: `file://hostname/path/to/dir`
/// Returns the decoded path, or None if the URI is not a valid file:// URI.
pub(crate) fn parse_osc7_uri(uri: &str) -> Option<String> {
    const PREFIX: &str = "file://";

    let after_prefix = uri.strip_prefix(PREFIX)?;

    // Find the first '/' after the hostname.
    // For "file:///home/user", after_prefix is "/home/user" and find('/') returns 0 (empty hostname).
    // For "file://host/path", after_prefix is "host/path" and find('/') returns 4.
    // For "file://", after_prefix is "" and find('/') returns None -> we return None.
    let slash_pos = after_prefix.find('/')?;
    let path_encoded = &after_prefix[slash_pos..];

    Some(percent_decode(path_encoded))
}

/// Decode percent-encoded bytes in a path string.
/// Invalid percent sequences (e.g. `%ZZ`) are preserved as-is.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut decoded: Vec<u8> = Vec::with_capacity(len);
    let mut i = 0;

    while i < len {
        if bytes[i] == b'%' && i + 2 < len {
            let hi = bytes[i + 1];
            let lo = bytes[i + 2];
            if let (Some(h), Some(l)) = (hex_val(hi), hex_val(lo)) {
                decoded.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(decoded)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Convert an ASCII hex digit to its numeric value, or None if not a valid hex digit.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_path() {
        assert_eq!(
            parse_osc7_uri("file://localhost/Users/user/project"),
            Some("/Users/user/project".to_string())
        );
    }

    #[test]
    fn parse_empty_hostname() {
        assert_eq!(
            parse_osc7_uri("file:///home/user"),
            Some("/home/user".to_string())
        );
    }

    #[test]
    fn parse_with_hostname() {
        assert_eq!(
            parse_osc7_uri("file://myhost.local/home/user"),
            Some("/home/user".to_string())
        );
    }

    #[test]
    fn parse_percent_encoded_space() {
        assert_eq!(
            parse_osc7_uri("file://host/path%20with%20space"),
            Some("/path with space".to_string())
        );
    }

    #[test]
    fn parse_percent_encoded_japanese() {
        assert_eq!(
            parse_osc7_uri("file://host/%E3%83%86%E3%82%B9%E3%83%88"),
            Some("/テスト".to_string())
        );
    }

    #[test]
    fn parse_no_file_prefix() {
        assert_eq!(parse_osc7_uri("http://example.com/path"), None);
    }

    #[test]
    fn parse_empty_string() {
        assert_eq!(parse_osc7_uri(""), None);
    }

    #[test]
    fn parse_file_only() {
        // "file://" has no hostname and no path
        assert_eq!(parse_osc7_uri("file://"), None);
    }

    #[test]
    fn parse_root_path() {
        assert_eq!(
            parse_osc7_uri("file://host/"),
            Some("/".to_string())
        );
    }

    #[test]
    fn parse_invalid_percent_encoding() {
        // %ZZ is invalid hex, should be preserved as-is
        assert_eq!(
            parse_osc7_uri("file://host/path%ZZ"),
            Some("/path%ZZ".to_string())
        );
    }
}

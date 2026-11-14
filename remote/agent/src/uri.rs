//! `file:` URIs for POSIX names (RFC 8089). A backslash is a filename character
//! here, not a separator, so URIs containing one are refused.
use crate::error::{Result, error};

pub fn encode(path: &str) -> String {
    let mut out = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/_.-~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn decode(uri: &str) -> Result<String> {
    let invalid = || error("invalid_request", "invalid resource URI");
    let bytes = uri.as_bytes();
    if uri.is_empty() || uri.len() > 128 * 1024 || !uri.is_ascii()
        || bytes.iter().any(|b| *b <= 32 || *b == 127 || b"?#\\".contains(b)) {
        return Err(invalid());
    }
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'%' && !(bytes.get(at + 1..at + 3).is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))) {
            return Err(invalid());
        }
    }
    let unsupported = || error("unsupported", "URI does not name a file in this environment");
    let (scheme, rest) = uri.split_once(':').ok_or_else(unsupported)?;
    if !scheme.eq_ignore_ascii_case("file") { return Err(unsupported()); }
    let path = match rest.strip_prefix("//") {
        Some(authority) => {
            let (host, path) = authority.split_at(authority.find('/').unwrap_or(authority.len()));
            if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) { return Err(unsupported()); }
            path
        }
        None => rest,
    };
    if !path.starts_with('/') { return Err(unsupported()); }
    let mut out = Vec::with_capacity(path.len());
    let raw = path.as_bytes();
    let mut at = 0;
    while at < raw.len() {
        if raw[at] == b'%' {
            out.push(u8::from_str_radix(&path[at + 1..at + 3], 16).map_err(|_| invalid())?);
            at += 3;
        } else {
            out.push(raw[at]);
            at += 1;
        }
    }
    let text = String::from_utf8(out).map_err(|_| error("invalid_request", "URI path is not UTF-8"))?;
    if text.len() > 32 * 1024 { return Err(error("limit_exceeded", "URI path exceeds the resource budget")); }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn posix_names_round_trip_and_backslash_is_a_name_character() {
        let path = "/work/a b/中文#1.txt";
        assert_eq!(decode(&encode(path)).unwrap(), path);
        assert!(encode("/a\\b").contains("%5C"));
        assert!(decode("file:///a%5Cb").unwrap().contains('\\'));
    }
    #[test]
    fn ambiguous_or_foreign_locations_are_refused() {
        for bad in ["file:///a?x", "file:///a#x", "file:///a\\b", "file:///a%zz", "file:///a b", ""] {
            assert!(decode(bad).is_err(), "{bad}");
        }
        assert_eq!(decode("http://x/a").unwrap_err().kind, "unsupported");
        assert_eq!(decode("file://other/a").unwrap_err().kind, "unsupported");
        assert_eq!(decode("file://localhost/a").unwrap(), "/a");
        assert_eq!(decode("file:/a").unwrap(), "/a");
        assert_eq!(decode("file:///%FF").unwrap_err().kind, "invalid_request");
    }
}

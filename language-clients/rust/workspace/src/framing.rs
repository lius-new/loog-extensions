//! Plugin-owned Content-Length framing for raw target-environment byte streams.
const HEADER_LIMIT: usize = 8 * 1024;
const OUTPUT_CHUNK_LIMIT: usize = 64 * 1024;

pub struct Frames {
    pending: Vec<u8>,
    length: Option<usize>,
    limit: usize,
    failed: bool,
}

impl Frames {
    pub fn new(limit: usize) -> Result<Self, String> {
        if !(1..=32 * 1024 * 1024).contains(&limit) {
            return Err("Invalid language message budget".into());
        }
        Ok(Self {
            pending: Vec::new(),
            length: None,
            limit,
            failed: false,
        })
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        if self.failed {
            return Err("Language stream framing has failed".into());
        }
        let result = self.decode(bytes);
        if result.is_err() {
            self.pending.clear();
            self.length = None;
            self.failed = true;
        }
        result
    }
    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        // One raw output chunk can finish a maximum-sized message and start
        // another. Budget that boundary explicitly instead of rejecting a valid
        // stream merely because transport chunk boundaries differ from frames.
        if bytes.len() > OUTPUT_CHUNK_LIMIT
            || self.pending.len().saturating_add(bytes.len())
                > self.limit + HEADER_LIMIT + OUTPUT_CHUNK_LIMIT
        {
            return Err("Language stream exceeds its buffer budget".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut messages = Vec::new();
        let mut at = 0;
        loop {
            if self.length.is_none() {
                let remaining = &self.pending[at..];
                let Some(end) = remaining
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                else {
                    if remaining.len() > HEADER_LIMIT {
                        return Err("Language message header is too large".into());
                    }
                    break;
                };
                if end > HEADER_LIMIT {
                    return Err("Language message header is too large".into());
                }
                let header = std::str::from_utf8(&remaining[..end])
                    .map_err(|_| "Invalid language header encoding")?;
                let mut length = None;
                for line in header.split("\r\n") {
                    let (name, value) = line.split_once(':').ok_or("Malformed language header")?;
                    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                        || name.is_empty()
                    {
                        return Err("Invalid language header name".into());
                    }
                    if name.eq_ignore_ascii_case("Content-Length") {
                        let value = value.trim();
                        if length.is_some()
                            || value.is_empty()
                            || !value.bytes().all(|b| b.is_ascii_digit())
                        {
                            return Err("Ambiguous language content length".into());
                        }
                        let value: usize = value
                            .parse()
                            .map_err(|_| "Invalid language content length")?;
                        if value == 0 || value > self.limit {
                            return Err("Language message exceeds its budget".into());
                        }
                        length = Some(value);
                    }
                }
                self.length = Some(length.ok_or("Language content length is missing")?);
                at += end + 4;
            }
            let length = self.length.unwrap();
            if self.pending.len() - at < length {
                break;
            }
            if messages.len() == 256 {
                return Err("Too many language messages in one output chunk".into());
            }
            let text = std::str::from_utf8(&self.pending[at..at + length])
                .map_err(|_| "Language message is not UTF-8")?
                .to_owned();
            messages.push(text);
            at += length;
            self.length = None;
        }
        self.pending.drain(..at);
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_unicode_headers_and_adjacent_messages_preserve_exact_text() {
        let text = "{\"text\":\"中文😀\"}";
        let bytes = format!("content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{text}Content-Length: 2\r\n\r\n{{}}", text.len()).into_bytes();
        let mut decoder = Frames::new(1024).unwrap();
        let mut result = Vec::new();
        for byte in bytes {
            result.extend(decoder.push(&[byte]).unwrap());
        }
        assert_eq!(result, [text, "{}"]);
    }
    #[test]
    fn a_chunk_can_finish_a_maximum_message_and_contain_the_next_message() {
        let first = "a".repeat(16 * 1024);
        let second = "b".repeat(16 * 1024);
        let prefix = format!(
            "Content-Length: {}\r\n\r\n{}",
            first.len(),
            &first[..first.len() - 2]
        );
        let suffix = format!("aaContent-Length: {}\r\n\r\n{second}", second.len());
        let mut decoder = Frames::new(first.len()).unwrap();
        assert!(decoder.push(prefix.as_bytes()).unwrap().is_empty());
        assert_eq!(decoder.push(suffix.as_bytes()).unwrap(), [first, second]);
    }
    #[test]
    fn invalid_lengths_headers_encoding_and_unbounded_input_fail_closed() {
        for bytes in [
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
            b"Content-Length: 9000\r\n\r\n",
            b"Other: value\r\n\r\n{}",
            b"Content-Length: +2\r\n\r\n{}",
            b"Content-Length: 1\r\n\r\n\xff",
        ] {
            let mut decoder = Frames::new(1024).unwrap();
            assert!(decoder.push(bytes).is_err());
            assert!(decoder.push(b"Content-Length: 2\r\n\r\n{}").is_err());
        }
        assert!(
            Frames::new(1024)
                .unwrap()
                .push(&vec![b'x'; HEADER_LIMIT + 1])
                .is_err()
        );
    }
}

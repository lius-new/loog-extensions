//! Incremental project configuration reads. Paths are formed by the provider;
//! the plugin owns the filename and the host later validates setting declarations.
use crate::api::{Answer, Failure, FailureKind, Operation, ProviderPath};
use serde_json::Value;

pub const MAX_SETTINGS: usize = 256 * 1024;
const CHUNK: u32 = 64 * 1024;
enum Step {
    Directory,
    File,
    Read,
    Done,
}
pub struct ProjectSettings {
    step: Step,
    operation: Operation,
    bytes: Vec<u8>,
    revision: Option<String>,
}
impl ProjectSettings {
    pub fn new(project: ProviderPath) -> Self {
        Self {
            step: Step::Directory,
            operation: Operation::ChildPath {
                parent: project,
                name: ".loog".into(),
            },
            bytes: vec![],
            revision: None,
        }
    }
    pub fn operation(&self) -> Option<Operation> {
        (!matches!(self.step, Step::Done)).then(|| self.operation.clone())
    }
    pub fn receive(&mut self, result: Result<Answer, Failure>) -> Result<Option<Value>, String> {
        if matches!(self.step, Step::Done) {
            return Err("Project settings read already finished".into());
        }
        let answer = match result {
            Ok(answer) => answer,
            Err(error) if error.kind == FailureKind::NotFound && self.bytes.is_empty() => {
                self.step = Step::Done;
                return Ok(Some(serde_json::json!({})));
            }
            Err(error) => {
                self.step = Step::Done;
                return Err(error.message);
            }
        };
        let result = (|| {
            match (&self.step, answer) {
                (Step::Directory, Answer::Path { path }) => {
                    self.operation = Operation::ChildPath {
                        parent: path,
                        name: "extension-settings.json".into(),
                    };
                    self.step = Step::File;
                }
                (Step::File, Answer::Path { path }) => {
                    self.operation = Operation::Read {
                        path,
                        offset: 0,
                        limit: CHUNK,
                    };
                    self.step = Step::Read;
                }
                (
                    Step::Read,
                    Answer::Bytes {
                        bytes,
                        revision,
                        eof,
                    },
                ) => {
                    if bytes.len() > CHUNK as usize
                        || self.bytes.len() + bytes.len() > MAX_SETTINGS
                        || !eof
                            && (bytes.is_empty() || self.bytes.len() + bytes.len() == MAX_SETTINGS)
                    {
                        return Err(
                            "Project settings read exceeds its budget or makes no progress".into(),
                        );
                    }
                    if self.bytes.is_empty() {
                        self.revision = revision.clone();
                    } else if revision != self.revision {
                        return Err("Project settings changed during reading".into());
                    }
                    if !eof && revision.is_none() {
                        return Err("Chunked settings require a stable revision".into());
                    }
                    self.bytes.extend(bytes);
                    if eof {
                        let text = std::str::from_utf8(&self.bytes)
                            .map_err(|_| "Project settings are not UTF-8")?;
                        let value: Value =
                            serde_json::from_str(text.trim_start_matches('\u{feff}'))
                                .map_err(|e| format!("Invalid project settings: {e}"))?;
                        if !value.is_object() {
                            return Err("Project settings must be an object".into());
                        }
                        self.step = Step::Done;
                        return Ok(Some(value));
                    }
                    let Operation::Read { offset, .. } = &mut self.operation else {
                        unreachable!()
                    };
                    *offset = self.bytes.len() as u64;
                }
                _ => return Err("Unexpected project settings response".into()),
            }
            Ok(None)
        })();
        if result.is_err() {
            self.step = Step::Done;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reader() -> ProjectSettings {
        let mut reader = ProjectSettings::new(ProviderPath::new("opaque:root\\中").unwrap());
        assert!(
            matches!(reader.operation(), Some(Operation::ChildPath { name, .. }) if name == ".loog")
        );
        for path in ["opaque-config-dir", "opaque-settings"] {
            reader
                .receive(Ok(Answer::Path {
                    path: ProviderPath::new(path).unwrap(),
                }))
                .unwrap();
        }
        reader
    }
    fn chunk(bytes: &[u8], revision: Option<&str>, eof: bool) -> Result<Answer, Failure> {
        Ok(Answer::Bytes {
            bytes: bytes.into(),
            revision: revision.map(str::to_owned),
            eof,
        })
    }
    #[test]
    fn settings_preserve_chunked_unicode_and_revision_and_reject_mixed_reads() {
        let mut read = reader();
        let text = "\u{feff}{\"plugin.value\":\"中😀\"}".as_bytes();
        let split = text.len() - 4;
        assert!(
            read.receive(chunk(&text[..split], Some("one"), false))
                .unwrap()
                .is_none()
        );
        assert!(
            matches!(read.operation(), Some(Operation::Read { offset, .. }) if offset == split as u64)
        );
        assert_eq!(
            read.receive(chunk(&text[split..], Some("one"), true))
                .unwrap()
                .unwrap()["plugin.value"],
            "中😀"
        );
        assert!(read.operation().is_none());
        assert!(read.receive(chunk(b"{}", Some("one"), true)).is_err());
        let mut read = reader();
        read.receive(chunk(b"{", Some("one"), false)).unwrap();
        assert!(read.receive(chunk(b"}", Some("two"), true)).is_err());
        assert!(reader().receive(chunk(b"{", None, false)).is_err());
    }
    #[test]
    fn missing_configuration_is_empty_but_denials_invalid_json_and_oversize_are_errors() {
        assert_eq!(
            reader()
                .receive(Err(Failure::new(FailureKind::NotFound, "absent")))
                .unwrap(),
            Some(serde_json::json!({}))
        );
        assert!(
            reader()
                .receive(Err(Failure::new(FailureKind::PermissionDenied, "denied")))
                .is_err()
        );
        for text in [b"[]".as_slice(), b"invalid", &[255]] {
            assert!(reader().receive(chunk(text, None, true)).is_err());
        }
        assert!(
            reader()
                .receive(chunk(&vec![b' '; CHUNK as usize + 1], Some("one"), true))
                .is_err()
        );
        let mut read = reader();
        for _ in 0..3 {
            read.receive(chunk(&vec![b' '; CHUNK as usize], Some("one"), false))
                .unwrap();
        }
        assert!(
            read.receive(chunk(&vec![b' '; CHUNK as usize], Some("one"), false))
                .is_err()
        );
    }
}

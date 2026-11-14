//! The agent's error kinds are the protocol's: the client maps them to
//! `FailureKind` by name, so these names are part of the agent protocol.
use std::io;

#[derive(Debug)]
pub struct AgentError {
    pub kind: &'static str,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, AgentError>;

pub fn error(kind: &'static str, message: impl Into<String>) -> AgentError {
    AgentError { kind, message: message.into() }
}

impl From<io::Error> for AgentError {
    fn from(source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::NotFound => error("not_found", "resource does not exist"),
            io::ErrorKind::AlreadyExists => error("conflict", "resource already exists"),
            io::ErrorKind::PermissionDenied => error("permission_denied", "resource access denied"),
            _ => {
                // "No such file (os error 2)" -> "No such file"
                let text = source.to_string();
                let text = text.rsplit_once(" (os error").map_or(text.as_str(), |(head, _)| head);
                error("failed", text)
            }
        }
    }
}

/// A name that is not UTF-8 never becomes a lossy identity.
pub fn not_utf8() -> AgentError {
    error("unsupported", "this provider requires UTF-8 resource names")
}

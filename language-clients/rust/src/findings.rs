//! What the language server found in files that are not open, told to Loog so its
//! Problems panel can list them. An open file's findings go the usual way, with
//! the document, and follow its edits; these are the rest of the workspace, such
//! as the output of a build that the server publishes for every file.
//!
//! A finding keeps the server's own line and character, and the unit the server
//! counts in, so no text is needed to place it.
#[cfg(test)]
use crate::host::findings as service;
#[cfg(not(test))]
use crate::loog::service::findings as service;
use crate::loog::extension::types::ColumnUnit;
use crate::protocol::{optional, string};
use serde_json::Value;

/// Loog's bounds on one file; more is cut here rather than refused there.
const MAX_PER_FILE: usize = 500;
const MAX_MESSAGE: usize = 2000;
const MAX_FIELD: usize = 128;

fn cut(text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn position(value: &Value) -> Option<service::Position> {
    Some(service::Position {
        line: u32::try_from(value["line"].as_u64()?).ok()?,
        character: u32::try_from(value["character"].as_u64()?).ok()?,
    })
}

/// The server's diagnostics as findings; one that cannot be placed is left out.
pub fn convert(values: &[Value]) -> Vec<service::Finding> {
    values
        .iter()
        .filter_map(|v| {
            Some(service::Finding {
                start: position(&v["range"]["start"])?,
                end: position(&v["range"]["end"])?,
                severity: v["severity"].as_u64().filter(|s| (1..=4).contains(s)).unwrap_or(1) as u8,
                message: cut(string(&v["message"]), MAX_MESSAGE),
                source: optional(&v["source"])
                    .or_else(|| Some(crate::NAME.into()))
                    .map(|s| cut(s, MAX_FIELD)),
                code: (!v["code"].is_null()).then(|| {
                    cut(
                        v["code"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v["code"].to_string()),
                        MAX_FIELD,
                    )
                }),
            })
        })
        .take(MAX_PER_FILE)
        .collect()
}

/// Replace what session `key` found in `path`; no diagnostics take the file out.
pub fn publish(key: &str, path: &str, unit: ColumnUnit, diagnostics: &[Value]) {
    let encoding = match unit {
        ColumnUnit::Utf8 => service::Encoding::Utf8,
        ColumnUnit::Utf16 => service::Encoding::Utf16,
    };
    if let Err(error) = service::publish(key, path, encoding, &convert(diagnostics)) {
        // Loog keeps what it already has; this is only worth a line in Output.
        crate::host::log(
            1,
            &format!("{}: findings for {path} were refused: {error}", crate::NAME),
        );
    }
}

/// Take back everything session `key` found.
pub fn withdraw(key: &str) {
    let _ = service::withdraw(key);
}

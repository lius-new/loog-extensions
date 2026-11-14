//! Path-free formatting requests and bounded edits for an exact resource snapshot.
use crate::{loog::extension::types::*, protocol::Text};
use serde_json::{Value, json};

pub(super) fn request(
    request: &ProviderRequest,
    text: &Text,
    uri: &str,
    capabilities: &Value,
) -> Result<Option<(&'static str, Value)>, String> {
    let context = request.context.as_ref();
    let mut params = json!({"textDocument":{"uri":uri},"options":{
        "tabSize":context.map_or(4, |c| c.tab_size),
        "insertSpaces":context.is_none_or(|c| c.insert_spaces)
    }});
    let (method, capability) = if request.kind == ProviderKind::FormatOnType {
        params["position"] = text.position(request.cursor.unwrap_or(0))?;
        params["ch"] = json!(request.trigger.as_ref().ok_or("Missing format trigger")?);
        (
            "textDocument/onTypeFormatting",
            "documentOnTypeFormattingProvider",
        )
    } else if context.is_some_and(|c| c.mode == Some(ProviderMode::Range)) {
        let range = request.range.ok_or("Missing format range")?;
        if range.start > range.end {
            return Err("Invalid format range".into());
        }
        params["range"] =
            json!({"start":text.position(range.start)?,"end":text.position(range.end)?});
        (
            "textDocument/rangeFormatting",
            "documentRangeFormattingProvider",
        )
    } else {
        ("textDocument/formatting", "documentFormattingProvider")
    };
    Ok(
        (!capabilities[capability].is_null() && capabilities[capability] != false)
            .then_some((method, params)),
    )
}

pub(super) fn edits(value: &Value, text: &Text) -> Result<Vec<TextEdit>, String> {
    if value.is_null() {
        return Ok(vec![]);
    }
    let items = value
        .as_array()
        .filter(|v| v.len() <= 1024)
        .ok_or("Invalid or excessive formatting edits")?;
    let mut bytes = 0usize;
    items
        .iter()
        .map(|item| {
            let replacement = item["newText"].as_str().ok_or("Invalid replacement text")?;
            bytes = bytes.saturating_add(replacement.len());
            if bytes > 512 * 1024 {
                return Err("Formatting result exceeds its budget".into());
            }
            let range = text.range(&item["range"])?;
            // A malformed position must not silently become a different edit.
            if text.position(range.start)? != item["range"]["start"]
                || text.position(range.end)? != item["range"]["end"]
            {
                return Err("Formatting range is outside the document".into());
            }
            Ok(TextEdit {
                range,
                text: replacement.into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_validate_unicode_positions_payload_and_budget_before_returning() {
        let text = Text::new("中😀x\n", ColumnUnit::Utf16);
        let edit = json!({"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":4}},"newText":"值"});
        let edits = edits(&json!([edit.clone()]), &text).unwrap();
        assert_eq!((edits[0].range.start, edits[0].range.end), (7, 8));
        let mut invalid = edit.clone();
        invalid["range"]["start"]["character"] = json!(2);
        assert!(super::edits(&json!([invalid]), &text).is_err());
        let mut invalid = edit.clone();
        invalid["range"]["end"]["line"] = json!(99);
        assert!(super::edits(&json!([invalid]), &text).is_err());
        let mut invalid = edit.clone();
        invalid["newText"] = Value::Null;
        assert!(super::edits(&json!([invalid]), &text).is_err());
        let mut large = edit.clone();
        large["newText"] = json!("x".repeat(512 * 1024 + 1));
        assert!(super::edits(&json!([large]), &text).is_err());
        assert!(super::edits(&json!(vec![edit; 1025]), &text).is_err());
    }
}

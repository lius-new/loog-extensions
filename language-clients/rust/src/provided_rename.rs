//! Text-only rename proposals. URI resolution remains provider-owned; no URI
//! here is interpreted as a path on the machine running the component.
use super::*;
use api::session::{
    ColumnEncoding, DocumentVersion, ResourceDocumentEdit, ResourcePosition, ResourceRange,
    ResourceTextEdit,
};

pub struct Document {
    pub uri: String,
    pub version: Option<DocumentVersion>,
    pub edits: Vec<ResourceTextEdit>,
}
pub fn prepare(value: &Value, text: &Text) -> Result<Option<RenameTarget>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let range = if value.get("range").is_some() || value.get("start").is_some() {
        let raw = value.get("range").unwrap_or(value);
        let range = text.range(raw)?;
        if text.position(range.start)? != raw["start"] || text.position(range.end)? != raw["end"] {
            return Err("Rename target is outside the document".into());
        }
        Some(range)
    } else {
        None
    };
    let placeholder = match value.get("placeholder") {
        None => None,
        Some(Value::String(text)) if text.len() <= 16 * 1024 => Some(text.clone()),
        _ => return Err("Invalid rename placeholder".into()),
    };
    if range.is_none() && value["defaultBehavior"] != true {
        return Err("Missing rename target".into());
    }
    Ok(Some(RenameTarget {
        range,
        placeholder,
        default_behavior: value["defaultBehavior"] == true,
    }))
}
pub fn parse(
    value: &Value,
    unit: ColumnUnit,
    mut version: impl FnMut(&str, Option<i64>) -> Result<Option<DocumentVersion>, String>,
) -> Result<Vec<Document>, String> {
    if value.is_null() {
        return Ok(vec![]);
    }
    if !value.is_object()
        || value
            .get("changeAnnotations")
            .is_some_and(|v| !v.as_object().is_some_and(|m| m.is_empty()))
    {
        return Err("Annotated workspace edits are not supported".into());
    }
    let mut result = vec![];
    let mut count = 0usize;
    let mut bytes = 0usize;
    let mut seen = BTreeSet::new();
    let mut add = |uri: &str, raw: Option<i64>, edits: &Value| -> Result<(), String> {
        if result.len() >= 16
            || uri.is_empty()
            || uri.len() > 16 * 1024
            || !seen.insert(uri.to_owned())
        {
            return Err("Invalid or excessive rename documents".into());
        }
        let edits = edits.as_array().ok_or("Invalid rename edits")?;
        count += edits.len();
        bytes += uri.len();
        if count > 1024 {
            return Err("Too many rename edits".into());
        }
        let edits = edits
            .iter()
            .map(|edit| {
                if edit.get("annotationId").is_some() {
                    return Err("Annotated edits are not supported".into());
                }
                let text = edit["newText"].as_str().ok_or("Invalid replacement text")?;
                bytes += text.len();
                if bytes > 512 * 1024 {
                    return Err("Rename edit payload exceeds its budget".into());
                }
                let range = locations::parse(&json!({"uri":uri,"range":edit["range"]}), unit)?[0].1;
                Ok(ResourceTextEdit {
                    range,
                    new_text: text.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        result.push(Document {
            uri: uri.into(),
            version: version(uri, raw)?,
            edits,
        });
        Ok(())
    };
    match (value.get("documentChanges"), value.get("changes")) {
        (Some(changes), None) => {
            for change in changes.as_array().ok_or("Invalid document changes")? {
                if change.get("kind").is_some() {
                    return Err("File creation, movement and deletion are not supported in rename proposals".into());
                }
                let uri = change["textDocument"]["uri"]
                    .as_str()
                    .ok_or("Missing edited URI")?;
                let raw = match &change["textDocument"]["version"] {
                    Value::Null => None,
                    value => Some(value.as_i64().ok_or("Invalid edited document version")?),
                };
                add(uri, raw, &change["edits"])?;
            }
        }
        (None, Some(changes)) => {
            for (uri, edits) in changes.as_object().ok_or("Invalid URI changes")? {
                add(uri, None, edits)?;
            }
        }
        (None, None) if value.as_object().is_some_and(|m| m.is_empty()) => {}
        _ => return Err("Ambiguous workspace edit".into()),
    }
    Ok(result)
}
pub fn uris(documents: &[Document]) -> Vec<(String, ResourceRange)> {
    documents
        .iter()
        .map(|doc| {
            (
                doc.uri.clone(),
                ResourceRange {
                    start: ResourcePosition {
                        line: 0,
                        character: 0,
                    },
                    end: ResourcePosition {
                        line: 0,
                        character: 0,
                    },
                    encoding: ColumnEncoding::Utf8,
                },
            )
        })
        .collect()
}
pub fn resolved(
    documents: Vec<Document>,
    locations: Vec<api::session::ResourceLocation>,
) -> Result<Vec<ResourceDocumentEdit>, String> {
    if documents.len() != locations.len() {
        return Err("Incomplete edit URI mapping".into());
    }
    let mut seen = BTreeSet::new();
    documents
        .into_iter()
        .zip(locations)
        .map(|(doc, location)| {
            if !seen.insert(location.resource.clone()) {
                return Err("Several URIs identify the same edited resource".into());
            }
            Ok(ResourceDocumentEdit {
                resource: location.resource,
                version: doc.version,
                edits: doc.edits,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit() -> Value {
        json!({"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":4}},"newText":"值"})
    }
    #[test]
    fn rename_preserves_uri_units_versions_and_rejects_unsupported_operations_atomically() {
        let stamp = DocumentVersion {
            document: 4,
            lifecycle: 2,
            version: 3,
        };
        let uri = "file:///work/a%5C%E4%B8%AD.rs";
        let value =
            json!({"documentChanges":[{"textDocument":{"uri":uri,"version":17},"edits":[edit()]}]});
        let docs = parse(&value, ColumnUnit::Utf16, |found, v| {
            assert_eq!(found, uri);
            assert_eq!(v, Some(17));
            Ok(Some(stamp))
        })
        .unwrap();
        assert_eq!(docs[0].uri, uri);
        assert_eq!(docs[0].version, Some(stamp));
        assert_eq!(docs[0].edits[0].range.encoding, ColumnEncoding::Utf16);
        let mut bad = value.clone();
        bad["documentChanges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"create","uri":"file:///other"}));
        assert!(parse(&bad, ColumnUnit::Utf16, |_, _| Ok(None)).is_err());
        assert!(parse(&value, ColumnUnit::Utf16, |_, _| Err("stale".into())).is_err());
        let mut bad = value.clone();
        bad["documentChanges"][0]["edits"][0]["annotationId"] = json!("confirmation");
        assert!(parse(&bad, ColumnUnit::Utf16, |_, _| Ok(None)).is_err());
        let mut bad = value.clone();
        bad["changes"] = json!({});
        assert!(parse(&bad, ColumnUnit::Utf16, |_, _| Ok(None)).is_err());
        let mut bad = value.clone();
        bad["documentChanges"][0]["edits"] = json!(vec![edit(); 1025]);
        assert!(parse(&bad, ColumnUnit::Utf16, |_, _| Ok(None)).is_err());
        let text = Text::new("中😀x", ColumnUnit::Utf16);
        assert_eq!(
            prepare(&edit()["range"], &text)
                .unwrap()
                .unwrap()
                .range
                .unwrap()
                .start,
            7
        );
        let mut bad = edit()["range"].clone();
        bad["start"]["character"] = json!(2);
        assert!(prepare(&bad, &text).is_err());
        assert!(
            prepare(&json!({"defaultBehavior":true}), &text)
                .unwrap()
                .unwrap()
                .default_behavior
        );
        assert!(prepare(&json!({}), &text).is_err());
    }
}

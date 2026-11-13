//! Bounded completion conversion. Resolve data belongs to an exact resource
//! revision and process incarnation; it never contains a client-side file path.
use super::*;

const ITEM_BYTES: usize = 32 * 1024;
const RESULT_BYTES: usize = 512 * 1024;
const ITEMS: usize = 256;

pub fn key(scope: u64, process: Process, stamp: api::session::DocumentVersion) -> String {
    format!(
        "resource:{scope}:{}:{}:{}:{}",
        process.0, stamp.document, stamp.lifecycle, stamp.version
    )
}

pub fn original(request: &ProviderRequest, key: &str) -> Result<Value, String> {
    let data = request
        .item
        .as_ref()
        .and_then(|item| item.data.as_deref())
        .filter(|data| data.len() <= ITEM_BYTES * 2)
        .ok_or("Completion resolve data is absent or too large")?;
    let value = crate::results::original(Some(data), key)?;
    if !value.is_object() {
        return Err("Invalid completion resolve item".into());
    }
    Ok(value)
}

pub fn answer(
    value: &Value,
    request: &ProviderRequest,
    text: &Text,
    key: &str,
    can_resolve: bool,
) -> Result<(Vec<CompletionItem>, bool), String> {
    let resolving = request.kind == ProviderKind::CompletionResolve;
    let merged;
    let offered;
    let defaults;
    if resolving {
        let mut raw = original(request, key)?;
        if let (Some(raw), Some(answer)) = (raw.as_object_mut(), value.as_object()) {
            raw.extend(answer.clone());
        }
        merged = vec![raw];
        offered = merged.as_slice();
        defaults = &Value::Null;
    } else {
        offered = if value.is_array() {
            array(value)
        } else {
            array(&value["items"])
        };
        defaults = &value["itemDefaults"];
    }
    let defaults_bytes = defaults.to_string().len();
    let query = request.context.as_ref().and_then(|c| c.query.as_deref());
    let mut bytes = 0usize;
    let mut incomplete = !resolving && value["isIncomplete"] == true;
    let mut items = vec![];
    for raw in offered {
        if !resolving && !crate::results::completion_matches(query, raw) {
            incomplete = true;
            continue;
        }
        // Include repeated defaults and an opaque resolve copy in the budget.
        let size = raw.to_string().len().saturating_add(defaults_bytes);
        let cost = size.saturating_mul(3).saturating_add(1024);
        if size > ITEM_BYTES || bytes + cost > RESULT_BYTES || items.len() >= ITEMS {
            incomplete = true;
            continue;
        }
        let Ok(mut item) =
            crate::results::completion(raw, defaults, text, key, can_resolve && !resolving)
        else {
            incomplete = true;
            continue;
        };
        // Resource execute-command is not bound yet. Never hand an environment
        // server's command to the independent local-path client.
        item.command = None;
        bytes += cost;
        items.push(item);
    }
    Ok((items, incomplete))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ProviderRequest {
        ProviderRequest {
            request: 1,
            provider: "completion".into(),
            kind: ProviderKind::Completion,
            document: 3,
            version: 7,
            snapshot: 8,
            length: 8,
            cursor: Some(8),
            range: None,
            trigger: None,
            item: None,
            context: None,
        }
    }
    #[test]
    fn defaults_unicode_resolve_and_resource_revision_isolation() {
        let stamp = api::session::DocumentVersion {
            document: 3,
            lifecycle: 2,
            version: 7,
        };
        let identity = key(4, Process(5), stamp);
        let text = Text::new("😀\nval", ColumnUnit::Utf16);
        let raw = json!({"items":[{"label":"value","textEditText":"value()","command":{"command":"remote"}}],
            "itemDefaults":{"data":{"token":9},"editRange":{"start":{"line":1,"character":0},"end":{"line":1,"character":3}}}});
        let (items, incomplete) = answer(&raw, &request(), &text, &identity, true).unwrap();
        assert!(!incomplete);
        assert_eq!(items[0].text_edit.as_ref().unwrap().range.start, 5);
        assert_eq!(items[0].text_edit.as_ref().unwrap().range.end, 8);
        assert!(items[0].command.is_none());
        let mut resolve = request();
        resolve.kind = ProviderKind::CompletionResolve;
        resolve.item = Some(items[0].clone());
        assert_eq!(original(&resolve, &identity).unwrap()["data"]["token"], 9);
        for other in [
            key(6, Process(5), stamp),
            key(4, Process(6), stamp),
            key(
                4,
                Process(5),
                api::session::DocumentVersion {
                    version: 8,
                    ..stamp
                },
            ),
            key(
                4,
                Process(5),
                api::session::DocumentVersion {
                    lifecycle: 3,
                    ..stamp
                },
            ),
        ] {
            assert!(original(&resolve, &other).is_err());
        }
        let (resolved, _) = answer(
            &json!({"documentation":"details"}),
            &resolve,
            &text,
            &identity,
            true,
        )
        .unwrap();
        assert_eq!(resolved[0].label, "value");
        assert_eq!(resolved[0].documentation.as_deref(), Some("details"));
        assert!(resolved[0].data.is_none());
    }
    #[test]
    fn result_limits_and_invalid_ranges_do_not_poison_other_items() {
        let text = Text::new("x", ColumnUnit::Utf16);
        let mut values = vec![
            json!({"label":"oversized", "detail":"x".repeat(ITEM_BYTES) }),
            json!({"label":"bad", "textEdit":{"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":0}},"newText":"y"}}),
        ];
        values.extend((0..1000).map(|i| json!({"label":format!("valid{i}")})));
        let (items, incomplete) =
            answer(&json!(values), &request(), &text, "session", true).unwrap();
        assert!(incomplete);
        assert_eq!(items.len(), ITEMS);
        assert_eq!(items[0].label, "valid0");
        let (items, incomplete) = answer(
            &json!({"items":[{"label":"small"}],"itemDefaults":{"data":"x".repeat(ITEM_BYTES)}}),
            &request(),
            &text,
            "session",
            true,
        )
        .unwrap();
        assert!(items.is_empty() && incomplete);
    }
}

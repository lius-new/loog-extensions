//! Lazy action offers retain protocol data in the originating resource revision.
//! Only the chosen proposal resolves URIs. Commands stay bound to that process
//! and document lifecycle, allowing the action's own edits to advance its version.
use super::*;
use api::session::{DocumentVersion, ResourceCodeAction as Action, ResourceCommand};

const ITEM: usize = 24 * 1024;
fn label(value: &Value, limit: usize) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= limit && !s.contains(['\r', '\n', '\0']))
        .map(str::to_owned)
        .ok_or_else(|| "Invalid action metadata".into())
}
fn metadata(value: &Value) -> Result<Action, String> {
    Ok(Action {
        title: label(&value["title"], 1024)?,
        kind: value.get("kind").map(|v| label(v, 256)).transpose()?,
        preferred: value["isPreferred"] == true,
        disabled: value
            .get("disabled")
            .map(|v| label(&v["reason"], 1024))
            .transpose()?,
        data: None,
        documents: None,
        command: None,
    })
}
pub fn offers(value: &Value, key: &str) -> Result<Vec<Action>, String> {
    if value.is_null() {
        return Ok(vec![]);
    }
    let values = value.as_array().ok_or("Invalid code action response")?;
    let mut actions = vec![];
    let mut bytes = 0;
    for value in values {
        let size = value.to_string().len();
        if size > ITEM || actions.len() >= 64 || bytes + size * 3 > 512 * 1024 {
            continue;
        }
        let Ok(mut action) = metadata(value) else {
            continue;
        };
        action.data = Some(crate::results::opaque(key, value));
        bytes += size * 3;
        actions.push(action);
    }
    Ok(actions)
}
pub fn original(request: &ProviderRequest, key: &str) -> Result<Value, String> {
    let data = request
        .context
        .as_ref()
        .and_then(|c| c.data.as_deref())
        .filter(|v| v.len() <= 64 * 1024)
        .ok_or("Missing action resolve data")?;
    let value = crate::results::original(Some(data), key)?;
    metadata(&value)?;
    if value.to_string().len() > ITEM {
        return Err("Action data exceeds its budget".into());
    }
    Ok(value)
}
pub fn command_key(scope: u64, process: Process, stamp: DocumentVersion) -> String {
    completion::key(
        scope,
        process,
        DocumentVersion {
            version: 0,
            ..stamp
        },
    )
}
fn allowed(capabilities: &Value, name: &str) -> bool {
    array(&capabilities["executeCommandProvider"]["commands"])
        .iter()
        .any(|v| v == name)
}
pub fn resolved(value: &Value, key: &str, capabilities: &Value) -> Result<Action, String> {
    if value.to_string().len() > 512 * 1024 {
        return Err("Resolved action exceeds its budget".into());
    }
    let mut action = metadata(value)?;
    if action.disabled.is_some() {
        return Ok(action);
    }
    if let Some(raw) = value.get("command").filter(|v| !v.is_null()) {
        let command = if raw.is_string() { value } else { raw };
        let name = label(&command["command"], 256)?;
        if !allowed(capabilities, &name) {
            return Err("The action command was not advertised by this server".into());
        }
        let args = command.get("arguments").cloned().unwrap_or(json!([]));
        if !args.as_array().is_some_and(|v| v.len() <= 64) || args.to_string().len() > 64 * 1024 {
            return Err("Action command arguments exceed their budget".into());
        }
        action.command = Some(ResourceCommand {
            name,
            arguments: vec![json!({"$loogSession":key,"arguments":args})],
        });
    }
    if value.get("edit").is_none_or(Value::is_null) && action.command.is_none() {
        return Err("The server did not resolve this action".into());
    }
    Ok(action)
}
pub fn command(
    request: &ProviderRequest,
    key: &str,
    capabilities: &Value,
) -> Result<Value, String> {
    let context = request.context.as_ref().ok_or("Missing command context")?;
    let name = context.name.as_deref().ok_or("Missing command name")?;
    let data = context.data.as_deref().ok_or("Missing command arguments")?;
    decode_command(name, data, key, capabilities)
}
fn decode_command(
    name: &str,
    data: &str,
    key: &str,
    capabilities: &Value,
) -> Result<Value, String> {
    if name.len() > 256 || !allowed(capabilities, name) {
        return Err("Command is not advertised by this server".into());
    }
    if data.len() > 128 * 1024 {
        return Err("Command envelope exceeds its budget".into());
    }
    let values: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    let values = values
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or("Invalid resource command envelope")?;
    let envelope = &values[0];
    if envelope["$loogSession"] != key {
        return Err("Command belongs to another resource or server incarnation".into());
    }
    let args = &envelope["arguments"];
    if !args.as_array().is_some_and(|v| v.len() <= 64) || args.to_string().len() > 64 * 1024 {
        return Err("Invalid command argument budget".into());
    }
    Ok(json!({"command":name,"arguments":args}))
}
pub fn params(request: &ProviderRequest, doc: &Document) -> Result<Value, String> {
    let start = request
        .range
        .map_or(request.cursor.unwrap_or(0), |r| r.start);
    let end = request.range.map_or(start, |r| r.end);
    if start > end {
        return Err("Reversed action range".into());
    }
    let diagnostics = doc
        .diagnostics
        .iter()
        .filter(|v| {
            doc.text
                .range(&v["range"])
                .is_ok_and(|r| r.start <= end && r.end >= start)
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut params = json!({"textDocument":{"uri":doc.uri},
        "range":{"start":doc.text.position(start)?,"end":doc.text.position(end)?},
        "context":{"diagnostics":diagnostics,"triggerKind":request.context.as_ref().map_or(1, |c| c.trigger_kind.clamp(1, 2))}});
    if let Some(context) = &request.context
        && !context.action_kinds.is_empty()
    {
        params["context"]["only"] = json!(context.action_kinds);
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_actions_and_commands_retain_resource_and_process_identity() {
        let stamp = DocumentVersion {
            document: 1,
            lifecycle: 2,
            version: 3,
        };
        let key = completion::key(4, Process(5), stamp);
        let raw = json!({"title":"Fix 中","kind":"quickfix","edit":{"changes":{}},
            "command":{"command":"after","arguments":[{"uri":"file:///tmp/a"}]}});
        let offered = offers(&json!([raw.clone()]), &key).unwrap().pop().unwrap();
        assert!(offered.documents.is_none() && offered.command.is_none());
        assert_eq!(
            crate::results::original(offered.data.as_deref(), &key).unwrap(),
            raw
        );
        assert!(
            crate::results::original(
                offered.data.as_deref(),
                &completion::key(9, Process(5), stamp)
            )
            .is_err()
        );
        let capabilities = json!({"executeCommandProvider":{"commands":["after"]}});
        let stable = command_key(4, Process(5), stamp);
        assert_eq!(
            stable,
            command_key(
                4,
                Process(5),
                DocumentVersion {
                    version: 4,
                    ..stamp
                }
            )
        );
        assert_ne!(stable, command_key(4, Process(6), stamp));
        assert_ne!(
            stable,
            command_key(
                4,
                Process(5),
                DocumentVersion {
                    lifecycle: 3,
                    ..stamp
                }
            )
        );
        let resolved = resolved(&raw, &stable, &capabilities).unwrap();
        let command = resolved.command.unwrap();
        let data = serde_json::to_string(&command.arguments).unwrap();
        assert_eq!(
            decode_command("after", &data, &stable, &capabilities).unwrap()["arguments"],
            raw["command"]["arguments"]
        );
        for other in [
            command_key(9, Process(5), stamp),
            command_key(4, Process(6), stamp),
            command_key(
                4,
                Process(5),
                DocumentVersion {
                    document: 2,
                    ..stamp
                },
            ),
            command_key(
                4,
                Process(5),
                DocumentVersion {
                    lifecycle: 4,
                    ..stamp
                },
            ),
        ] {
            assert!(decode_command("after", &data, &other, &capabilities).is_err());
        }
        assert!(decode_command("unadvertised", &data, &stable, &capabilities).is_err());
        assert!(decode_command("after", "[]", &stable, &capabilities).is_err());
        assert!(super::resolved(&raw, &stable, &json!({})).is_err());
        assert!(
            offers(&json!([{"title":"x","data":"a".repeat(ITEM)}]), &key)
                .unwrap()
                .is_empty()
        );
    }
}

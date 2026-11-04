use crate::{loom::extension::types::*, protocol::*};
use serde_json::{Value, json};
pub fn opaque(session: &str, value: &Value) -> String {
    json!({"session":session,"value":value}).to_string()
}
pub fn original(data: Option<&str>, session: &str) -> Result<Value> {
    let Some(data) = data else {
        return Ok(Value::Null);
    };
    let mut value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    if let Some(text) = value.as_str() {
        // COMPAT(quoted-provider-data): Older consumers may JSON-quote opaque data; retain until supported consumers all use raw JSON payload text, which new callers should send. No removal version has been scheduled.
        value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    }
    if value["session"].as_str() != Some(session) {
        return Err("result belongs to another server session".into());
    }
    Ok(value["value"].take())
}
pub fn command(value: &Value, session: &str) -> Option<ProviderCommand> {
    value["command"].as_str().map(|name|ProviderCommand{name:name.to_owned(),arguments:json!([{"$loomSession":session,"arguments":value.get("arguments").unwrap_or(&json!([]))}]).to_string()})
}
pub fn location(value: &Value, text: &Text) -> Result<FileLocation> {
    Ok(FileLocation {
        path: path(
            value["targetUri"]
                .as_str()
                .or(value["uri"].as_str())
                .ok_or("missing location URI")?,
        )?,
        range: text.file_range(
            value
                .get("targetSelectionRange")
                .or(value.get("targetRange"))
                .or(value.get("range"))
                .unwrap_or(
                    &json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}}),
                ),
        ),
    })
}
pub fn workspace_edit(
    value: &Value,
    text: &Text,
    version: &impl Fn(&str, u64) -> Result<u64>,
) -> Result<WorkspaceEdit> {
    let mut changes = Vec::new();
    let edits = |values: &Value| {
        array(values)
            .iter()
            .map(|v| FileTextEdit {
                range: text.file_range(&v["range"]),
                text: string(&v["newText"]),
            })
            .collect()
    };
    if let Some(values) = value["changes"].as_object() {
        for (url, values) in values {
            changes.push(WorkspaceChange::Text(FileEdits {
                path: path(url)?,
                version: None,
                edits: edits(values),
            }));
        }
    }
    for change in array(&value["documentChanges"]) {
        let options = &change["options"];
        changes.push(match change["kind"].as_str() {
            Some("create") => WorkspaceChange::Create(WorkspaceCreate {
                path: path(&string(&change["uri"]))?,
                overwrite: options["overwrite"] == true,
                ignore_if_exists: options["ignoreIfExists"] == true,
            }),
            Some("rename") => WorkspaceChange::Rename(WorkspaceRename {
                source: path(&string(&change["oldUri"]))?,
                target: path(&string(&change["newUri"]))?,
                overwrite: options["overwrite"] == true,
                ignore_if_exists: options["ignoreIfExists"] == true,
            }),
            Some("delete") => WorkspaceChange::Delete(WorkspaceDelete {
                path: path(&string(&change["uri"]))?,
                recursive: options["recursive"] == true,
                ignore_if_not_exists: options["ignoreIfNotExists"] == true,
            }),
            None if change["textDocument"].is_object() => {
                let doc = &change["textDocument"];
                let url = doc["uri"].as_str().ok_or("missing document URI")?;
                WorkspaceChange::Text(FileEdits {
                    path: path(url)?,
                    version: doc["version"]
                        .as_u64()
                        .map(|v| version(url, v))
                        .transpose()?,
                    edits: edits(&change["edits"]),
                })
            }
            _ => return Err("unsupported workspace change".into()),
        });
    }
    Ok(WorkspaceEdit { changes })
}
pub fn completion_matches(query: Option<&str>, item: &Value) -> bool {
    let Some(query) = query.filter(|q| !q.is_empty() && q.len() <= 256 && q.is_ascii()) else {
        return true;
    };
    let Some(candidate) = item["filterText"].as_str().or(item["label"].as_str()) else {
        return true;
    };
    if !candidate.is_ascii() {
        return true;
    }
    let sensitive = query.bytes().any(|c| c.is_ascii_uppercase());
    let mut chars = candidate.bytes();
    query.bytes().all(|wanted| {
        chars.any(|c| {
            if sensitive {
                c == wanted
            } else {
                c.eq_ignore_ascii_case(&wanted)
            }
        })
    })
}
pub fn completion(
    item: &Value,
    defaults: &Value,
    text: &Text,
    session: &str,
    resolve: bool,
) -> Result<CompletionItem> {
    let field = |key: &str| {
        if item[key].is_null() {
            &defaults[key]
        } else {
            &item[key]
        }
    };
    let edit = &item["textEdit"];
    Ok(CompletionItem {
        label: string(&item["label"]),
        label_detail: optional(&item["labelDetails"]["detail"]),
        label_description: optional(&item["labelDetails"]["description"]),
        deprecated: item["deprecated"]
            .as_bool()
            .or_else(|| item["tags"].as_array().map(|a| a.contains(&json!(1)))),
        preselect: item["preselect"].as_bool(),
        insert_text_mode: field("insertTextMode").as_u64().map(|v| v as u8),
        detail: optional(&item["detail"]),
        documentation: markdown(&item["documentation"]),
        kind: item["kind"].as_u64().map(|v| v as u32),
        insert_text: optional(&item["insertText"]),
        text_edit: if edit.is_null() && !defaults["editRange"].is_null() {
            let range = &defaults["editRange"];
            Some(TextEdit {
                range: text.range(range.get("replace").unwrap_or(range))?,
                text: string(item.get("textEditText").or(item.get("insertText")).unwrap_or(&item["label"])),
            })
        } else if edit.is_null() {
            None
        } else {
            Some(TextEdit {
                range: text.range(edit.get("replace").unwrap_or(&edit["range"]))?,
                text: string(&edit["newText"]),
            })
        },
        additional_text_edits: array(&item["additionalTextEdits"])
            .iter()
            .map(|v| text.edit(v))
            .collect::<Result<_>>()?,
        insert_text_format: field("insertTextFormat").as_u64().map(|v| v as u8),
        commit_characters: array(field("commitCharacters"))
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        command: command(&item["command"], session),
        sort_text: optional(&item["sortText"]),
        filter_text: optional(&item["filterText"]),
        // Compact completion batches retain the original item once. Only the
        // over-budget/resolve fallback needs an individual opaque JSON payload.
        data: resolve.then(|| {
    let mut original = item.clone();
    for field in [
        "data",
        "insertTextFormat",
        "insertTextMode",
        "commitCharacters",
    ] {
        if original[field].is_null() {
            original[field] = defaults[field].clone();
        }
    }
    if original["textEdit"].is_null() && !defaults["editRange"].is_null() {
        original["textEdit"] = json!({"range":defaults["editRange"].get("replace").unwrap_or(&defaults["editRange"]),"newText":item.get("textEditText").or(item.get("insertText")).unwrap_or(&item["label"])});
    }

            opaque(session, &original)
        }),
    })
}
pub fn diagnostics(values: &[Value], text: &Text) -> Vec<Diagnostic> {
    values
        .iter()
        .filter_map(|v| {
            Some(Diagnostic {
                range: text.range(&v["range"]).ok()?,
                severity: v["severity"].as_u64().unwrap_or(1) as u8,
                message: string(&v["message"]),
                source: optional(&v["source"]).or_else(|| Some(crate::NAME.into())),
                code: (!v["code"].is_null()).then(|| {
                    v["code"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v["code"].to_string())
                }),
                documentation_url: optional(&v["codeDescription"]["href"]),
            })
        })
        .collect()
}
pub fn symbols(values: &[Value], text: &Text) -> Result<Vec<DocumentSymbol>> {
    fn visit(
        values: &[Value],
        text: &Text,
        parent: Option<u32>,
        depth: usize,
        out: &mut Vec<DocumentSymbol>,
    ) -> Result<()> {
        if depth > 128 {
            return Err("symbol tree depth exceeded".into());
        }
        for v in values {
            let range = v.get("range").or(v["location"].get("range"));
            let Some(range) = range else { continue };
            let index = out.len() as u32;
            out.push(DocumentSymbol {
                name: string(&v["name"]),
                detail: optional(&v["detail"]),
                deprecated: v["deprecated"]
                    .as_bool()
                    .or_else(|| v["tags"].as_array().map(|a| a.contains(&json!(1)))),
                kind: number(&v["kind"]),
                range: text.range(range)?,
                selection: text.range(v.get("selectionRange").unwrap_or(range))?,
                parent,
            });
            visit(array(&v["children"]), text, Some(index), depth + 1, out)?;
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(values, text, None, 0, &mut out)?;
    Ok(out)
}
pub fn action(
    value: &Value,
    text: &Text,
    session: &str,
    version: &impl Fn(&str, u64) -> Result<u64>,
) -> Result<ProviderAction> {
    Ok(ProviderAction {
        title: string(&value["title"]),
        kind: optional(&value["kind"]),
        preferred: value["isPreferred"] == true,
        disabled: optional(&value["disabled"]["reason"]),
        edit: if value["edit"].is_null() {
            None
        } else {
            Some(workspace_edit(&value["edit"], text, version)?)
        },
        command: command(
            if value["command"].is_string() {
                value
            } else {
                &value["command"]
            },
            session,
        ),
        data: Some(opaque(session, value)),
    })
}
pub fn hierarchy(value: &Value, text: &Text, session: &str) -> Result<HierarchyItem> {
    Ok(HierarchyItem {
        name: string(&value["name"]),
        kind: number(&value["kind"]),
        detail: optional(&value["detail"]),
        location: location(value, text)?,
        selection: text.file_range(&value["selectionRange"]),
        data: Some(opaque(session, value)),
    })
}
pub fn lens(value: &Value, text: &Text, session: &str) -> Result<CodeLens> {
    Ok(CodeLens {
        range: text.range(&value["range"])?,
        title: optional(&value["command"]["title"]),
        command: command(&value["command"], session),
        data: Some(opaque(session, value)),
    })
}
pub fn signatures(value: &Value) -> SignatureSet {
    fn slice(label: &str, start: u64, end: u64) -> (String, ByteRange) {
        let mut units = 0;
        let mut first = label.len();
        let mut last = label.len();
        for (i, c) in label.char_indices() {
            if units >= start && first == label.len() {
                first = i;
            }
            if units >= end {
                last = i;
                break;
            }
            units += c.len_utf16() as u64;
        }
        (
            label[first..last.max(first)].to_owned(),
            ByteRange {
                start: first as u64,
                end: last.max(first) as u64,
            },
        )
    }
    SignatureSet {
        active_signature: number(&value["activeSignature"]),
        active_parameter: value["activeParameter"].as_u64().map(|v| v as u32),
        signatures: array(&value["signatures"])
            .iter()
            .map(|v| SignatureEntry {
                label: string(&v["label"]),
                documentation: markdown(&v["documentation"]),
                active_parameter: v["activeParameter"].as_u64().map(|v| v as u32),
                parameters: array(&v["parameters"])
                    .iter()
                    .map(|p| {
                        let (label, range) = if p["label"].is_array() {
                            let (label, range) = slice(
                                &string(&v["label"]),
                                p["label"][0].as_u64().unwrap_or(0),
                                p["label"][1].as_u64().unwrap_or(0),
                            );
                            (label, Some(range))
                        } else {
                            (string(&p["label"]), None)
                        };
                        SignatureParameter {
                            label,
                            range,
                            documentation: markdown(&p["documentation"]),
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}
pub fn selections(values: &[Value], text: &Text) -> Result<Vec<SelectionRange>> {
    fn visit(v: &Value, t: &Text, depth: usize, out: &mut Vec<SelectionRange>) -> Result<u32> {
        if depth >= 64 || out.len() >= 256 {
            return Err("selection tree budget exceeded".into());
        }
        let parent = if v["parent"].is_null() {
            None
        } else {
            Some(visit(&v["parent"], t, depth + 1, out)?)
        };
        let index = out.len() as u32;
        out.push(SelectionRange {
            range: t.range(&v["range"])?,
            parent,
        });
        Ok(index)
    }
    let mut out = Vec::new();
    for v in values {
        visit(v, text, 0, &mut out)?;
    }
    Ok(out)
}
pub fn semantic_delta(previous: &[u32], edits: &[Value]) -> Result<Vec<u32>> {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|v| std::cmp::Reverse(v["start"].as_u64().unwrap_or(u64::MAX)));
    let mut boundary = previous.len();
    let mut data = previous.to_vec();
    for edit in edits {
        let start = edit["start"].as_u64().ok_or("invalid semantic delta")? as usize;
        let count = edit["deleteCount"]
            .as_u64()
            .ok_or("invalid semantic delta")? as usize;
        if start.checked_add(count).is_none_or(|n| n > boundary) {
            return Err("invalid or overlapping semantic delta".into());
        }
        boundary = start;
        let added = array(&edit["data"])
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| "invalid semantic data".to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        if !edit["data"].is_null() && !edit["data"].is_array() {
            return Err("invalid semantic data".into());
        }
        data.splice(start..start + count, added);
    }
    Ok(data)
}
pub fn semantic(
    data: &[u32],
    legend: &Value,
    modifiers: &Value,
    text: &Text,
) -> Result<Vec<Highlight>> {
    if !data.len().is_multiple_of(5) {
        return Err("malformed semantic token data".into());
    }
    let mut out: Vec<Highlight> = Vec::new();
    let mut line = 0u64;
    let mut column = 0u64;
    for token in data.as_chunks::<5>().0 {
        line += token[0] as u64;
        column = if token[0] != 0 {
            token[1] as u64
        } else {
            column + token[1] as u64
        };
        let kind = legend[token[3] as usize].as_str().unwrap_or_default();
        let has = |name: &str| {
            array(modifiers)
                .iter()
                .position(|v| v == name)
                .is_some_and(|i| i < 32 && token[4] & (1 << i) != 0)
        };
        let theme = match kind {
            "namespace" => "module",
            "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => {
                if has("defaultLibrary") {
                    "type.builtin"
                } else {
                    "type"
                }
            }
            "parameter" => "variable.parameter",
            "variable" => {
                if has("readonly") && has("static") {
                    "constant"
                } else {
                    "variable"
                }
            }
            "property" => "property",
            "enumMember" => "constant",
            "function" => {
                if has("defaultLibrary") {
                    "function.builtin"
                } else {
                    "function"
                }
            }
            "method" => "function.method",
            "macro" => "function.macro",
            "keyword" | "modifier" => "keyword",
            "comment" => "comment",
            "string" => "string",
            "number" => "number",
            "regexp" => "string.regex",
            "operator" => "operator",
            "decorator" => "attribute",
            _ => continue,
        };
        if let Ok(range)=text.range(&json!({"start":{"line":line,"character":column},"end":{"line":line,"character":column+token[2] as u64}}))&& range.start<range.end&&out.last().is_none_or(|v|v.range.end<=range.start){out.push(Highlight{range,theme:theme.into()});}
    }
    Ok(out)
}

/// Map cached push diagnostics through the same replacement sent to the server.
pub fn rebase_diagnostics(
    values: &mut Vec<Value>,
    before: &Text,
    after: &Text,
    change: &Value,
) -> Result<()> {
    let replaced = before.range(&change["range"])?;
    let inserted = change["text"]
        .as_str()
        .ok_or("invalid document change")?
        .len() as u64;
    let shift = |offset: u64| offset - replaced.end + replaced.start + inserted;
    values.retain_mut(|value| {
        let Ok(range) = before.range(&value["range"]) else {
            return false;
        };
        let start = if range.start >= replaced.end {
            shift(range.start)
        } else if range.start >= replaced.start {
            replaced.start + inserted
        } else {
            range.start
        };
        let end = if range.end > replaced.end {
            shift(range.end)
        } else if range.end > replaced.start {
            replaced.start
        } else {
            range.end
        };
        if start > end {
            return false;
        }
        let (Ok(start), Ok(end)) = (after.position(start), after.position(end)) else {
            return false;
        };
        value["range"] = json!({"start":start,"end":end});
        true
    });
    Ok(())
}

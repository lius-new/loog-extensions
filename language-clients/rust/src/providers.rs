//! Provider requests and typed result conversion.
use super::*;

impl Client {
    pub(super) fn provide(&mut self, request: ProviderRequest) -> Result<()> {
        if request.kind == ProviderKind::WorkspaceSymbols {
            let processes = self
                .sessions
                .values()
                .filter(|s| {
                    !s.stopped && (!s.ready || yes(&s.effective(None)["workspaceSymbolProvider"]))
                })
                .map(|s| s.process)
                .collect::<Vec<_>>();
            if processes.is_empty() {
                host::release_snapshot(request.snapshot);
                return host::provide_language(
                    request.request,
                    &LanguageAnswer::WorkspaceSymbols(vec![]),
                )
                .map_err(|e| e.message);
            }
            let owner = request.request;
            self.symbols.insert(
                owner,
                SymbolGroup {
                    remaining: processes.len(),
                    values: vec![],
                },
            );
            for process in processes {
                let query = request
                    .context
                    .as_ref()
                    .and_then(|c| c.query.clone())
                    .unwrap_or_default();
                self.with_session(process, |c, s| {
                    if s.ready {
                        c.rpc(
                            s,
                            "workspace/symbol",
                            json!({"query":query}),
                            Purpose::Symbols(owner),
                            5000,
                        )
                    } else {
                        s.waiting_symbols.push((owner, query));
                        Ok(())
                    }
                })?;
            }
            host::release_snapshot(request.snapshot);
            return Ok(());
        }
        if request.kind == ProviderKind::ExecuteCommand {
            let ctx = request.context.as_ref();
            let command = ProviderCommand {
                name: ctx.and_then(|c| c.name.clone()).unwrap_or_default(),
                arguments: ctx
                    .and_then(|c| c.data.clone())
                    .unwrap_or_else(|| "[]".into()),
            };
            return self.execute(command, Some(request));
        }
        let id = request.document;
        if !self.documents.contains_key(&id) {
            let (reference, _) = self.capture(id)?;
            let process = self.session(&reference)?;
            self.documents.insert(
                id,
                Document {
                    reference,
                    process,
                    text: Text::new("", ColumnUnit::Utf16),
                    server_version: None,
                    diagnostics: vec![],
                    signature: Value::Null,
                    dirty: true,
                },
            );
            self.refresh_status();
        }
        let process = self.documents[&id].process;
        let result = self.with_session(process, |c, s| {
            if !s.ready {
                s.waiting.push_back(request);
                return Ok(());
            }
            c.dispatch_provider(s, request)
        });
        result.map(|_| ())
    }
    pub(super) fn dispatch_provider(
        &mut self,
        s: &mut Session,
        request: ProviderRequest,
    ) -> Result<()> {
        let snapshot = request.snapshot;
        let result = (|| {
            let (reference, captured) = self.capture(request.document)?;
            if reference.version != request.version {
                return Err("provider snapshot is no longer current".into());
            }
            let supplied = if matches!(
                request.kind,
                ProviderKind::Formatting
                    | ProviderKind::FormatOnType
                    | ProviderKind::CodeActions
                    | ProviderKind::CodeActionResolve
                    | ProviderKind::Save
            ) {
                host::read_range(
                    snapshot,
                    ByteRange {
                        start: 0,
                        end: request.length,
                    },
                )
                .map(Rc::<str>::from)
                .map_err(|e| e.message)?
            } else {
                captured
            };
            self.synchronize(s, request.document, Some((reference, supplied)))?;
            let doc = self
                .documents
                .get(&request.document)
                .ok_or("document closed")?;
            let text = doc.text.clone();
            let reference = doc.reference.clone();
            let default = context();
            let ctx = request.context.as_ref().unwrap_or(&default);
            let supported = s.effective(Some(&reference));
            let mut diagnostic_pull = None;
            let resolve = matches!(
                request.kind,
                ProviderKind::CompletionResolve
                    | ProviderKind::CodeActionResolve
                    | ProviderKind::CodeLensResolve
                    | ProviderKind::PrepareRename
            );
            if request.kind != ProviderKind::Diagnostics
                && !resolve
                && !fields(kind_name(request.kind))
                    .iter()
                    .any(|f| yes(&supported[*f]))
            {
                return Err("provider is not registered for this document".into());
            }
            let data = request
                .item
                .as_ref()
                .and_then(|i| i.data.as_deref())
                .or(ctx.data.as_deref());
            let raw = if let Some(data) = data {
                let mut value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
                if let Some(text) = value.as_str() {
                    // COMPAT(quoted-provider-data): Older consumers may JSON-quote opaque data; retain until supported consumers all use raw JSON payload text, which new callers should send. No removal version has been scheduled.
                    value = serde_json::from_str(text).map_err(|e| e.to_string())?;
                }
                if let Some(pair) = value.as_array().filter(|v| v.len() == 2) {
                    let batch = pair[0].as_u64().ok_or("invalid completion batch")?;
                    let index = pair[1].as_u64().ok_or("invalid completion index")? as usize;
                    let set = self
                        .completion_sets
                        .get(&batch)
                        .filter(|set| {
                            set.process == s.process
                                && set.document == request.document
                                && set.lifecycle == reference.lifecycle
                        })
                        .ok_or("completion result has expired")?;
                    let mut item = set
                        .items
                        .get(index)
                        .ok_or("completion result has expired")?
                        .clone();
                    for field in [
                        "data",
                        "insertTextFormat",
                        "insertTextMode",
                        "commitCharacters",
                    ] {
                        if item[field].is_null() {
                            item[field] = set.defaults[field].clone();
                        }
                    }
                    if item["textEdit"].is_null() && !set.defaults["editRange"].is_null() {
                        item["textEdit"] = json!({"range":set.defaults["editRange"].get("replace").unwrap_or(&set.defaults["editRange"]),"newText":item.get("textEditText").or(item.get("insertText")).unwrap_or(&item["label"])});
                    }
                    item
                } else {
                    results::original(Some(data), &s.key)?
                }
            } else {
                Value::Null
            };
            let position = text.position(
                request
                    .cursor
                    .unwrap_or(request.range.map_or(0, |r| r.start)),
            )?;
            let range = json!({"start":text.position(request.range.map_or(0,|r|r.start))?,"end":text.position(request.range.map_or(text.value.len() as u64,|r|r.end))?});
            let mut target = json!({"textDocument":{"uri":uri(&reference.path)}});
            let mut point = target.clone();
            point["position"] = position;
            let options = json!({"tabSize":ctx.tab_size,"insertSpaces":ctx.insert_spaces});
            let mut semantic_delta = false;
            let (method, params) = match request.kind {
                ProviderKind::Completion => {
                    point["context"] = json!({"triggerKind":if ctx.trigger_kind==0{if request.trigger.is_some(){2}else{1}}else{ctx.trigger_kind}});
                    if let Some(trigger) = &request.trigger {
                        point["context"]["triggerCharacter"] = json!(trigger);
                    }
                    ("textDocument/completion", point)
                }
                ProviderKind::CompletionResolve => ("completionItem/resolve", raw.clone()),
                ProviderKind::Hover => ("textDocument/hover", point),
                ProviderKind::Definition => (
                    match ctx.mode {
                        Some(ProviderMode::Declaration) => "textDocument/declaration",
                        Some(ProviderMode::TypeDefinition) => "textDocument/typeDefinition",
                        Some(ProviderMode::Implementation) => "textDocument/implementation",
                        _ => "textDocument/definition",
                    },
                    point,
                ),
                ProviderKind::References => {
                    point["context"] = json!({"includeDeclaration":ctx.include_declaration});
                    ("textDocument/references", point)
                }
                ProviderKind::DocumentSymbols => ("textDocument/documentSymbol", target),
                ProviderKind::SignatureHelp => {
                    point["context"] =
                        json!({"triggerKind":ctx.trigger_kind.max(1),"isRetrigger":ctx.retrigger});
                    if let Some(trigger) = &request.trigger {
                        point["context"]["triggerCharacter"] = json!(trigger);
                    }
                    if ctx.retrigger && !doc.signature.is_null() {
                        let mut active = doc.signature.clone();
                        if let Some(v) = ctx.active_signature {
                            active["activeSignature"] = json!(v);
                        }
                        if let Some(v) = ctx.active_parameter {
                            active["activeParameter"] = json!(v);
                        }
                        point["context"]["activeSignatureHelp"] = active;
                    }
                    ("textDocument/signatureHelp", point)
                }
                ProviderKind::Diagnostics => {
                    if !yes(&supported["diagnosticProvider"]) {
                        host::provide_diagnostics(
                            request.request,
                            &results::diagnostics(&doc.diagnostics, &text),
                        )
                        .map_err(|e| e.message)?;
                        host::release_snapshot(snapshot);
                        return Ok(());
                    }
                    let mut identifiers = s.diagnostic_sources(&reference).into_iter();
                    if let Some(Some(identifier)) = identifiers.next() {
                        target["identifier"] = json!(identifier);
                    }
                    let remaining: VecDeque<_> = identifiers.flatten().collect();
                    if !remaining.is_empty() {
                        diagnostic_pull = Some(DiagnosticPull {
                            remaining,
                            items: vec![],
                            bytes: 0,
                        });
                    }
                    ("textDocument/diagnostic", target)
                }
                ProviderKind::Formatting | ProviderKind::Save => {
                    target["options"] = options;
                    if ctx.mode == Some(ProviderMode::Range) {
                        target["range"] = range;
                        ("textDocument/rangeFormatting", target)
                    } else {
                        ("textDocument/formatting", target)
                    }
                }
                ProviderKind::FormatOnType => {
                    point["options"] = options;
                    point["ch"] = json!(request.trigger);
                    ("textDocument/onTypeFormatting", point)
                }
                ProviderKind::DocumentHighlights => ("textDocument/documentHighlight", point),
                ProviderKind::PrepareRename => ("textDocument/prepareRename", point),
                ProviderKind::Rename => {
                    point["newName"] = json!(ctx.name);
                    ("textDocument/rename", point)
                }
                ProviderKind::FoldingRanges => ("textDocument/foldingRange", target),
                ProviderKind::SelectionRanges => {
                    target["positions"] = json!([point["position"]]);
                    ("textDocument/selectionRange", target)
                }
                ProviderKind::InlayHints => {
                    target["range"] = range;
                    ("textDocument/inlayHint", target)
                }
                ProviderKind::CodeActions => {
                    let start = request.range.map_or(0, |r| r.start);
                    let end = request.range.map_or(text.value.len() as u64, |r| r.end);
                    let diagnostics = doc
                        .diagnostics
                        .iter()
                        .filter(|v| {
                            text.range(&v["range"])
                                .is_ok_and(|r| r.start <= end && r.end >= start)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    target["range"] = range;
                    target["context"] =
                        json!({"diagnostics":diagnostics,"triggerKind":ctx.trigger_kind.max(1)});
                    if !ctx.action_kinds.is_empty() {
                        target["context"]["only"] = json!(ctx.action_kinds);
                    }
                    ("textDocument/codeAction", target)
                }
                ProviderKind::CodeActionResolve => ("codeAction/resolve", raw.clone()),
                ProviderKind::CodeLens => ("textDocument/codeLens", target),
                ProviderKind::CodeLensResolve => ("codeLens/resolve", raw.clone()),
                ProviderKind::CallHierarchy => match ctx.mode {
                    Some(ProviderMode::Incoming) => {
                        ("callHierarchy/incomingCalls", json!({"item":raw}))
                    }
                    Some(ProviderMode::Outgoing) => {
                        ("callHierarchy/outgoingCalls", json!({"item":raw}))
                    }
                    _ => ("textDocument/prepareCallHierarchy", point),
                },
                ProviderKind::TypeHierarchy => match ctx.mode {
                    Some(ProviderMode::Supertypes) => {
                        ("typeHierarchy/supertypes", json!({"item":raw}))
                    }
                    Some(ProviderMode::Subtypes) => ("typeHierarchy/subtypes", json!({"item":raw})),
                    _ => ("textDocument/prepareTypeHierarchy", point),
                },
                ProviderKind::Highlights => {
                    let cap = &supported["semanticTokensProvider"];
                    if !yes(cap) {
                        host::provide_language(
                            request.request,
                            &LanguageAnswer::Highlights(vec![]),
                        )
                        .map_err(|e| e.message)?;
                        host::release_snapshot(snapshot);
                        return Ok(());
                    }
                    let previous = s.semantic.get(&uri(&reference.path));
                    if let Some(id) = previous
                        .and_then(|v| v.0.as_ref())
                        .filter(|_| yes(&cap["full"]["delta"]))
                    {
                        target["previousResultId"] = json!(id);
                        semantic_delta = true;
                        ("textDocument/semanticTokens/full/delta", target)
                    } else if yes(&cap["full"]) {
                        ("textDocument/semanticTokens/full", target)
                    } else {
                        target["range"] = range;
                        ("textDocument/semanticTokens/range", target)
                    }
                }
                _ => return Err("unsupported provider".into()),
            };
            self.rpc(
                s,
                method,
                params,
                Purpose::Provider(Box::new(Call {
                    request,
                    text,
                    reference,
                    raw,
                    semantic_delta,
                    diagnostic_pull,
                })),
                5000,
            )
        })();
        if result.is_err() {
            host::release_snapshot(snapshot);
        }
        result
    }
    pub(super) fn answer_provider(
        &mut self,
        s: &mut Session,
        call: &Call,
        answer: Value,
        wire_bytes: usize,
    ) -> Result<()> {
        let request = &call.request;
        let text = &call.text;
        let default = context();
        let ctx = request.context.as_ref().unwrap_or(&default);
        let unit = s.unit;
        let version = |url: &str, v: u64| self.protocol_version(url, v);
        // Provider answers are valid only for the synchronized document lifecycle.
        if self.documents.get(&request.document).is_none_or(|d| {
            d.reference.lifecycle != call.reference.lifecycle
                || d.reference.version != call.reference.version
                || !std::rc::Rc::ptr_eq(&d.text.value, &call.text.value)
        }) {
            return Err("provider result is no longer current".into());
        }
        let mut language = None;
        match request.kind {
            ProviderKind::Completion | ProviderKind::CompletionResolve => {
                let resolve = request.kind == ProviderKind::CompletionResolve;
                let mut answer = if resolve {
                    merge_item(&call.raw, &answer)
                } else {
                    answer
                };
                let server_incomplete = answer["isIncomplete"] == true;
                let defaults = if resolve {
                    Value::Null
                } else {
                    answer
                        .get_mut("itemDefaults")
                        .map(Value::take)
                        .unwrap_or(Value::Null)
                };
                let offered = if resolve {
                    vec![answer]
                } else {
                    let items = if answer.is_array() {
                        answer
                    } else {
                        answer["items"].take()
                    };
                    match items {
                        Value::Array(items) => items,
                        _ => vec![],
                    }
                };
                let supported = s.effective(Some(&call.reference));
                let can_resolve = yes(&supported["completionProvider"]["resolveProvider"]);
                // The complete response is a conservative cache budget: count its
                // already available wire bytes rather than serializing the result again.
                let bytes = wire_bytes;
                let mut batch = None;
                if !resolve && can_resolve && bytes <= 8 * 1024 * 1024 {
                    while !self.completion_sets.is_empty()
                        && (self.completion_sets.len() >= 4
                            || self
                                .completion_sets
                                .values()
                                .map(|s| s.bytes)
                                .sum::<usize>()
                                + bytes
                                > 8 * 1024 * 1024)
                    {
                        let first = *self.completion_sets.keys().next().unwrap();
                        self.completion_sets.remove(&first);
                    }
                    self.next_completion += 1;
                    batch = Some(self.next_completion);
                }
                let mut items = Vec::new();
                for (index, item) in offered.iter().enumerate() {
                    if resolve || results::completion_matches(ctx.query.as_deref(), item) {
                        let mut item = results::completion(
                            item,
                            &defaults,
                            text,
                            &s.key,
                            can_resolve && batch.is_none(),
                        )?;
                        if let Some(batch) = batch {
                            item.data = Some(format!("[{batch},{index}]"));
                        }
                        items.push(item);
                    }
                }
                let incomplete = server_incomplete || items.len() < offered.len();
                if let Some(batch) = batch {
                    self.completion_sets.insert(
                        batch,
                        CompletionSet {
                            process: s.process,
                            document: request.document,
                            lifecycle: call.reference.lifecycle,
                            items: offered,
                            defaults,
                            bytes,
                        },
                    );
                }
                host::provide_completions(request.request, &items, incomplete)
                    .map_err(|e| e.message)?;
            }
            ProviderKind::Hover => {
                let value = if answer.is_null() {
                    None
                } else {
                    Some(HoverAnswer {
                        contents: markdown(&answer["contents"]).unwrap_or_default(),
                        range: answer.get("range").map(|v| text.range(v)).transpose()?,
                    })
                };
                host::provide_hover(request.request, value.as_ref()).map_err(|e| e.message)?;
            }
            ProviderKind::Definition | ProviderKind::References => {
                let values = if answer.is_array() {
                    array(&answer).to_vec()
                } else if answer.is_null() {
                    vec![]
                } else {
                    vec![answer]
                };
                language = Some(LanguageAnswer::Places(
                    values
                        .iter()
                        .map(|v| results::location(v, text))
                        .collect::<Result<_>>()?,
                ));
            }
            ProviderKind::DocumentSymbols => {
                host::provide_symbols(request.request, &results::symbols(array(&answer), text)?)
                    .map_err(|e| e.message)?
            }
            ProviderKind::SignatureHelp => {
                if let Some(doc) = self.documents.get_mut(&request.document) {
                    doc.signature = answer.clone();
                }
                language = Some(LanguageAnswer::Signatures(results::signatures(&answer)));
            }
            ProviderKind::Diagnostics => {
                if answer["kind"] == "full"
                    && let Some(doc) = self.documents.get_mut(&request.document)
                {
                    doc.diagnostics = array(&answer["items"]).to_vec();
                }
                let values = self
                    .documents
                    .get(&request.document)
                    .map_or(&[][..], |d| d.diagnostics.as_slice());
                host::provide_diagnostics(request.request, &results::diagnostics(values, text))
                    .map_err(|e| e.message)?;
            }
            ProviderKind::Formatting | ProviderKind::FormatOnType | ProviderKind::Save => {
                host::provide_edits(
                    request.request,
                    &array(&answer)
                        .iter()
                        .map(|v| text.edit(v))
                        .collect::<Result<Vec<_>>>()?,
                )
                .map_err(|e| e.message)?
            }
            ProviderKind::DocumentHighlights => {
                language = Some(LanguageAnswer::DocumentHighlights(
                    array(&answer)
                        .iter()
                        .map(|v| {
                            Ok(RangeKind {
                                range: text.range(&v["range"])?,
                                kind: v["kind"].as_u64().unwrap_or(1) as u32,
                            })
                        })
                        .collect::<Result<_>>()?,
                ))
            }
            ProviderKind::PrepareRename => {
                language = Some(LanguageAnswer::RenameTarget(if answer.is_null() {
                    None
                } else {
                    Some(RenameTarget {
                        range: if answer.get("range").is_some() || answer.get("start").is_some() {
                            Some(text.range(answer.get("range").unwrap_or(&answer))?)
                        } else {
                            None
                        },
                        placeholder: optional(&answer["placeholder"]),
                        default_behavior: answer["defaultBehavior"] == true,
                    })
                }))
            }
            ProviderKind::Rename => {
                language = Some(LanguageAnswer::WorkspaceEdit(results::workspace_edit(
                    &answer, text, &version,
                )?))
            }
            ProviderKind::FoldingRanges => {
                language = Some(LanguageAnswer::Folds(
                    array(&answer)
                        .iter()
                        .map(|v| FoldRange {
                            start: number(&v["startLine"]),
                            end: number(&v["endLine"]),
                            start_column: v["startCharacter"].as_u64().map(|v| v as u32),
                            end_column: v["endCharacter"].as_u64().map(|v| v as u32),
                            kind: optional(&v["kind"]),
                            label: optional(&v["collapsedText"]),
                        })
                        .collect(),
                ))
            }
            ProviderKind::SelectionRanges => {
                language = Some(LanguageAnswer::Selections(results::selections(
                    array(&answer),
                    text,
                )?))
            }
            ProviderKind::InlayHints => host::provide_inlay_hints(
                request.request,
                &array(&answer)
                    .iter()
                    .map(|v| {
                        Ok(InlayHint {
                            offset: text.offset(&v["position"])?,
                            label: v["label"].as_str().map(str::to_owned).unwrap_or_else(|| {
                                array(&v["label"])
                                    .iter()
                                    .map(|v| string(&v["value"]))
                                    .collect::<String>()
                            }),
                            kind: v["kind"].as_u64().map(|v| v as u8),
                            padding_left: v["paddingLeft"] == true,
                            padding_right: v["paddingRight"] == true,
                            tooltip: markdown(&v["tooltip"]),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
            .map_err(|e| e.message)?,
            ProviderKind::CodeActions | ProviderKind::CodeActionResolve => {
                let values = if request.kind == ProviderKind::CodeActionResolve {
                    vec![answer]
                } else {
                    array(&answer).to_vec()
                };
                language = Some(LanguageAnswer::Actions(
                    values
                        .iter()
                        .map(|v| results::action(v, text, &s.key, &version))
                        .collect::<Result<_>>()?,
                ));
            }
            ProviderKind::CodeLens | ProviderKind::CodeLensResolve => {
                let values = if request.kind == ProviderKind::CodeLensResolve {
                    vec![answer]
                } else {
                    array(&answer).to_vec()
                };
                language = Some(LanguageAnswer::Lenses(
                    values
                        .iter()
                        .map(|v| results::lens(v, text, &s.key))
                        .collect::<Result<_>>()?,
                ));
            }
            ProviderKind::CallHierarchy
                if matches!(
                    ctx.mode,
                    Some(ProviderMode::Incoming | ProviderMode::Outgoing)
                ) =>
            {
                language = Some(LanguageAnswer::Calls(
                    array(&answer)
                        .iter()
                        .map(|v| {
                            Ok(HierarchyEdge {
                                item: results::hierarchy(
                                    &v[if ctx.mode == Some(ProviderMode::Incoming) {
                                        "from"
                                    } else {
                                        "to"
                                    }],
                                    text,
                                    &s.key,
                                )?,
                                sites: array(&v["fromRanges"])
                                    .iter()
                                    .map(|v| text.file_range(v))
                                    .collect(),
                            })
                        })
                        .collect::<Result<_>>()?,
                ))
            }
            ProviderKind::CallHierarchy | ProviderKind::TypeHierarchy => {
                language = Some(LanguageAnswer::Hierarchy(
                    array(&answer)
                        .iter()
                        .map(|v| results::hierarchy(v, text, &s.key))
                        .collect::<Result<_>>()?,
                ))
            }
            ProviderKind::Highlights => {
                let url = uri(&call.reference.path);
                let data = if answer["edits"].is_array() {
                    if !call.semantic_delta {
                        return Err("unexpected semantic delta".into());
                    }
                    let previous = s
                        .semantic
                        .get(&url)
                        .ok_or("semantic delta has no baseline")?;
                    results::semantic_delta(&previous.1, array(&answer["edits"]))?
                } else {
                    array(&answer["data"])
                        .iter()
                        .map(|v| {
                            v.as_u64()
                                .and_then(|v| u32::try_from(v).ok())
                                .ok_or_else(|| "invalid semantic data".into())
                        })
                        .collect::<Result<Vec<_>>>()?
                };
                let caps = s.effective(Some(&call.reference));
                let cap = &caps["semanticTokensProvider"];
                let values = results::semantic(
                    &data,
                    &cap["legend"]["tokenTypes"],
                    &cap["legend"]["tokenModifiers"],
                    &Text::new(text.value.clone(), unit),
                )?;
                s.semantic
                    .insert(url, (optional(&answer["resultId"]), data));
                language = Some(LanguageAnswer::Highlights(values));
            }
            _ => return Err("unsupported provider response".into()),
        }
        if let Some(answer) = language {
            host::provide_language(request.request, &answer).map_err(|e| e.message)?;
        }
        Ok(())
    }
    pub(super) fn execute(
        &mut self,
        command: ProviderCommand,
        request: Option<ProviderRequest>,
    ) -> Result<()> {
        let value: Value = serde_json::from_str(&command.arguments).map_err(|e| e.to_string())?;
        let args = value
            .as_array()
            .ok_or("command arguments must be an array")?;
        let origin = if args.len() == 1 {
            args[0]["$loogSession"].as_str()
        } else {
            None
        };
        let processes = self
            .sessions
            .values()
            .filter(|s| {
                s.ready
                    && !s.stopped
                    && if let Some(key) = origin {
                        s.key == key
                    } else {
                        array(&s.effective(None)["executeCommandProvider"]["commands"])
                            .iter()
                            .any(|v| v == &command.name)
                    }
            })
            .map(|s| s.process)
            .collect::<Vec<_>>();
        if processes.len() != 1 {
            if let Some(request) = request {
                host::release_snapshot(request.snapshot);
            }
            return Err("command origin needs an unambiguous session".into());
        }
        let params = json!({"command":command.name,"arguments":if origin.is_some(){&args[0]["arguments"]}else{&value}});
        self.with_session(processes[0], |c, s| {
            c.rpc(
                s,
                "workspace/executeCommand",
                params,
                Purpose::Command(request.map(Box::new)),
                5000,
            )
        })?;
        Ok(())
    }
}
fn merge_item(left: &Value, right: &Value) -> Value {
    let mut value = left.clone();
    if let (Some(a), Some(b)) = (value.as_object_mut(), right.as_object()) {
        a.extend(b.clone());
    }
    if !value["label"].as_str().is_some_and(|v| !v.is_empty()) {
        value["label"] = left["label"].clone();
    }
    value
}

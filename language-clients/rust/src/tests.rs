use super::*;
include!("config_tests.rs");
use crate::host::{Answer, state};

fn span() -> Value {
    json!({"start":{"line":0,"character":0},"end":{"line":0,"character":7}})
}
fn root(id: u64, path: &str) -> RootRef {
    RootRef {
        id,
        name: format!("root-{id}"),
        path: path.into(),
    }
}
fn doc(id: u64, path: &str, text: &str) -> DocumentRef {
    let reference = DocumentRef {
        document: id,
        lifecycle: 1,
        path: path.into(),
        language: "go".into(),
        variant: Some("go".into()),
        version: 0,
    };
    state(|s| {
        s.documents.insert(id, (reference.clone(), text.into()));
    });
    reference
}
fn setup() -> (Client, u64) {
    state(|s| {
        *s = Default::default();
        s.roots = vec![root(1, "C:/project")];
    });
    let mut c = Client {
        config: crate::config::GOPLS,
        ..Default::default()
    };
    let reference = doc(
        1,
        "C:/project/main.go",
        "package main\nfunc add(a int,b int) int {return a+b}\nfunc main(){add(1,2)}\n",
    );
    c.message(Inbound::DocumentOpened(reference)).unwrap();
    let process = state(|s| s.spawned[0].0);
    let request = sent("initialize");
    response(
        &mut c,
        process,
        request["id"].clone(),
        json!({"capabilities":caps()}),
    );
    (c, process)
}
fn caps() -> Value {
    let mut caps = json!({"textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":true}},"positionEncoding":"utf-16","semanticTokensProvider":{"full":{"delta":true},"legend":{"tokenTypes":["keyword"],"tokenModifiers":[]}}});
    for kind in KINDS {
        for field in fields(kind) {
            if caps[*field].is_null() {
                caps[*field] = json!({"resolveProvider":true,"prepareProvider":true,"commands":["gopls.test"],"triggerCharacters":["."]});
            }
        }
    }
    caps
}
fn sent(method: &str) -> Value {
    state(|s| {
        s.messages
            .iter()
            .rev()
            .find(|(_, v)| v["method"] == method)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("missing {method}; messages {:?}", s.messages))
    })
}
fn response(c: &mut Client, process: u64, id: Value, result: Value) {
    input(c, process, json!({"jsonrpc":"2.0","id":id,"result":result}));
}
fn input(c: &mut Client, process: u64, value: Value) {
    c.message(Inbound::ProcessOutput(ProcessOutput {
        process,
        channel: OutputChannel::Stdout,
        bytes: vec![],
        text: Some(value.to_string()),
    }))
    .unwrap();
}
fn server(c: &mut Client, process: u64, id: u64, method: &str, params: Value) {
    input(
        c,
        process,
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    );
}
fn request(kind: ProviderKind, id: u64) -> ProviderRequest {
    let captured = host::capture_document(1).unwrap();
    ProviderRequest {
        request: id,
        provider: format!("{ID}.{}", kind_name(kind)),
        kind,
        snapshot: captured.snapshot,
        document: 1,
        version: captured.document.version,
        length: captured.length,
        range: None,
        cursor: Some(1),
        trigger: None,
        item: None,
        context: Some(context()),
    }
}
fn ask(c: &mut Client, process: u64, kind: ProviderKind, method: &str, answer: Value) {
    let request = request(kind, 100);
    c.message(Inbound::ProviderRequest(request)).unwrap();
    response(c, process, sent(method)["id"].clone(), answer);
    assert!(
        state(|s| !s.answers.is_empty()),
        "no answer for {kind:?}: {:?}",
        state(|s| s.logs.clone())
    );
    assert!(state(|s| s.snapshots.is_empty()));
    state(|s| s.answers.clear());
}

#[test]
fn provider_surface_preserves_all_declared_features_and_typed_results() {
    let (mut c, p) = setup();
    let location = json!({"uri":"file:///C:/project/main.go","range":span()});
    ask(
        &mut c,
        p,
        ProviderKind::Hover,
        "textDocument/hover",
        json!({"contents":{"kind":"markdown","value":"hover"},"range":span()}),
    );
    ask(
        &mut c,
        p,
        ProviderKind::Definition,
        "textDocument/definition",
        json!([location]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::References,
        "textDocument/references",
        json!([location]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::DocumentSymbols,
        "textDocument/documentSymbol",
        json!([{"name":"main","kind":12,"range":span(),"selectionRange":span()}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::SignatureHelp,
        "textDocument/signatureHelp",
        json!({"signatures":[{"label":"add(a int)","parameters":[{"label":[4,9]}]}]}),
    );
    ask(
        &mut c,
        p,
        ProviderKind::Formatting,
        "textDocument/formatting",
        json!([{"range":span(),"newText":"package"}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::FormatOnType,
        "textDocument/onTypeFormatting",
        json!([]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::DocumentHighlights,
        "textDocument/documentHighlight",
        json!([{"range":span(),"kind":2}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::PrepareRename,
        "textDocument/prepareRename",
        json!({"range":span(),"placeholder":"package"}),
    );
    ask(
        &mut c,
        p,
        ProviderKind::Rename,
        "textDocument/rename",
        json!({"documentChanges":[{"textDocument":{"uri":"file:///C:/project/main.go","version":1},"edits":[{"range":span(),"newText":"module"}]}]}),
    );
    ask(
        &mut c,
        p,
        ProviderKind::FoldingRanges,
        "textDocument/foldingRange",
        json!([{"startLine":0,"endLine":2}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::SelectionRanges,
        "textDocument/selectionRange",
        json!([{"range":span(),"parent":{"range":span()}}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::InlayHints,
        "textDocument/inlayHint",
        json!([{"position":{"line":0,"character":1},"label":[{"value":"hint"}]}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::CodeActions,
        "textDocument/codeAction",
        json!([{"title":"fix","edit":{"changes":{"file:///C:/project/main.go":[{"range":span(),"newText":"package"}]}}}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::CodeLens,
        "textDocument/codeLens",
        json!([{"range":span(),"command":{"command":"gopls.test","title":"Run","arguments":[]}}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::CallHierarchy,
        "textDocument/prepareCallHierarchy",
        json!([{"name":"main","kind":12,"uri":"file:///C:/project/main.go","range":span(),"selectionRange":span()}]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::TypeHierarchy,
        "textDocument/prepareTypeHierarchy",
        json!([]),
    );
    ask(
        &mut c,
        p,
        ProviderKind::Highlights,
        "textDocument/semanticTokens/full",
        json!({"data":[0,0,7,0,0],"resultId":"one"}),
    );
    ask(
        &mut c,
        p,
        ProviderKind::Highlights,
        "textDocument/semanticTokens/full/delta",
        json!({"edits":[{"start":0,"deleteCount":5,"data":[0,0,3,0,0]}],"resultId":"two"}),
    );
    ask(
        &mut c,
        p,
        ProviderKind::WorkspaceSymbols,
        "workspace/symbol",
        json!([{"name":"main","kind":12,"location":location}]),
    );
}

#[test]
fn completion_filtering_keeps_original_resolve_indices_defaults_and_commands() {
    let (mut c, p) = setup();
    let mut query = request(ProviderKind::Completion, 1);
    query.context.as_mut().unwrap().query = Some("beta".into());
    c.provide(query).unwrap();
    response(
        &mut c,
        p,
        sent("textDocument/completion")["id"].clone(),
        json!({"items":[{"label":"alpha"},{"label":"beta","command":{"command":"gopls.test","arguments":[1]}}],"itemDefaults":{"data":{"key":42},"insertTextFormat":2,"editRange":span()}}),
    );
    let item = state(|s| match s.answers.pop().unwrap() {
        Answer::Completions(_, items, incomplete) => {
            assert!(incomplete);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].insert_text_format, Some(2));
            items[0].clone()
        }
        other => panic!("{other:?}"),
    });
    let mut resolve = request(ProviderKind::CompletionResolve, 2);
    resolve.item = Some(item);
    c.provide(resolve).unwrap();
    let rpc = sent("completionItem/resolve");
    assert_eq!(rpc["params"]["label"], "beta");
    assert_eq!(rpc["params"]["data"]["key"], 42);
    response(&mut c, p, rpc["id"].clone(), json!({"detail":"resolved"}));
    state(|s| match s.answers.pop().unwrap() {
        Answer::Completions(_, items, _) => {
            assert_eq!(items[0].detail.as_deref(), Some("resolved"))
        }
        other => panic!("{other:?}"),
    });
    c.close(1).unwrap();
    assert!(c.completion_sets.is_empty());
    assert!(state(|s| s.snapshots.is_empty()));
}

#[test]
fn unicode_coordinates_incremental_sync_and_stale_diagnostics() {
    let (mut c, p) = setup();
    state(|s| {
        let (doc, text) = s.documents.get_mut(&1).unwrap();
        doc.version = 1;
        *text = "package 中😀\r\n".into();
    });
    c.message(Inbound::DocumentChanged(DocumentChange {
        document: 1,
        lifecycle: 1,
        from_version: 0,
        to_version: 1,
        edits: 1,
    }))
    .unwrap();
    let sync = sent("textDocument/didChange");
    assert!(sync["params"]["contentChanges"][0]["range"].is_object());
    let version = c.documents[&1].server_version.unwrap();
    server(
        &mut c,
        p,
        1,
        "textDocument/publishDiagnostics",
        json!({"uri":"file:///C:/project/main.go","version":version-1,"diagnostics":[{"range":span(),"message":"old"}]}),
    );
    assert!(state(|s| s.diagnostics.is_empty()));
    server(
        &mut c,
        p,
        2,
        "textDocument/publishDiagnostics",
        json!({"uri":"file:///C:/project/main.go","version":version,"diagnostics":[{"range":span(),"message":"new"}]}),
    );
    assert_eq!(state(|s| s.diagnostics.len()), 1);
    for unit in [ColumnUnit::Utf8, ColumnUnit::Utf16] {
        let text = Text::new("中😀\r\nx", unit);
        for offset in [0, 3, 7, 8, 9, 10] {
            assert_eq!(
                text.offset(&text.position(offset).unwrap()).unwrap(),
                if offset == 8 { 7 } else { offset }
            );
        }
        assert!(text.position(4).is_err());
    }
}

#[test]
fn dynamic_registration_waits_for_watches_and_rolls_back_failures() {
    let (mut c, p) = setup();
    server(
        &mut c,
        p,
        100,
        "client/registerCapability",
        json!({"registrations":[{"id":"watch","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":[{"globPattern":"**/*.go","kind":1}]}},{"id":"completion","method":"textDocument/completion","registerOptions":{"triggerCharacters":["@"],"documentSelector":[{"language":"go","pattern":"**/*.go"}]}}]}),
    );
    let watch = state(|s| *s.watches.keys().next().unwrap());
    assert!(!state(|s| s
        .messages
        .iter()
        .any(|(_, v)| v["id"] == 100 && v.get("result").is_some())));
    c.message(Inbound::WatchReady(watch)).unwrap();
    assert!(state(|s| s
        .messages
        .iter()
        .any(|(_, v)| v["id"] == 100 && v.get("result").is_some())));
    assert!(state(|s| s.configurations[&format!("{ID}.completion")]
        .triggers
        .contains(&"@".into())));
    c.message(Inbound::FileChanged(FileChanged {
        watch,
        root: 1,
        changes: vec![FileChange {
            path: "main.go".into(),
            kind: FileChangeKind::Change,
        }],
    }))
    .unwrap();
    assert!(!state(|s| s
        .messages
        .iter()
        .any(|(_, v)| v["method"] == "workspace/didChangeWatchedFiles")));
    c.message(Inbound::FileChanged(FileChanged {
        watch,
        root: 1,
        changes: vec![FileChange {
            path: "main.go".into(),
            kind: FileChangeKind::Create,
        }],
    }))
    .unwrap();
    assert_eq!(
        sent("workspace/didChangeWatchedFiles")["params"]["changes"][0]["type"],
        1
    );
    state(|s| s.fail_watch = true);
    server(
        &mut c,
        p,
        101,
        "client/registerCapability",
        json!({"registrations":[{"id":"completion","method":"textDocument/hover","registerOptions":{}},{"id":"second","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":[{"globPattern":"**"}]}}]}),
    );
    assert_eq!(
        c.sessions[&p].dynamic["completion"].field,
        "completionProvider"
    );
    assert!(state(|s| s.watches.contains_key(&watch)));
}

#[test]
fn commands_wait_for_native_edit_receipts_and_preserve_origin() {
    let (mut c, p) = setup();
    let mut command = request(ProviderKind::ExecuteCommand, 90);
    command.context.as_mut().unwrap().name = Some("gopls.test".into());
    command.context.as_mut().unwrap().data =
        Some(json!([{"$loomSession":"1:","arguments":[42]}]).to_string());
    c.provide(command).unwrap();
    let rpc = sent("workspace/executeCommand");
    assert_eq!(rpc["params"]["arguments"], json!([42]));
    server(
        &mut c,
        p,
        400,
        "workspace/applyEdit",
        json!({"edit":{"documentChanges":[{"textDocument":{"uri":"file:///C:/project/main.go","version":1},"edits":[{"range":span(),"newText":"module"}]}]}}),
    );
    let operation = state(|s| s.operations.last().unwrap().0);
    response(&mut c, p, rpc["id"].clone(), Value::Null);
    assert!(state(|s| s.answers.is_empty()));
    c.message(Inbound::OperationComplete((
        operation,
        Completion::Committed(CommittedReceipt {
            transaction: 1,
            version: 1,
        }),
    )))
    .unwrap();
    assert!(state(|s| matches!(
        s.answers.last(),
        Some(Answer::Language(90, LanguageAnswer::Completed))
    )));
    assert!(state(|s| s
        .messages
        .iter()
        .any(|(_, v)| v["id"] == 400 && v["result"]["applied"] == true)));
}

#[test]
fn cancellation_retries_backpressure_and_timeout_are_bounded() {
    let (mut c, p) = setup();
    state(|s| s.blocked = true);
    c.provide(request(ProviderKind::Hover, 1)).unwrap();
    assert_eq!(c.sessions[&p].outbox.len(), 1);
    c.cancel(1);
    assert!(c.sessions[&p].pending.is_empty());
    assert!(state(|s| s.snapshots.is_empty()));
    state(|s| s.blocked = false);
    c.message(Inbound::ProcessWritable(ProcessWritable {
        process: p,
        bytes: 1000,
    }))
    .unwrap();
    c.provide(request(ProviderKind::Hover, 2)).unwrap();
    let first = sent("textDocument/hover");
    input(
        &mut c,
        p,
        json!({"jsonrpc":"2.0","id":first["id"],"error":{"code":-32801,"message":"modified"}}),
    );
    let retry = sent("textDocument/hover");
    assert_ne!(first["id"], retry["id"]);
    response(&mut c, p, retry["id"].clone(), json!({"contents":"okay"}));
    assert!(state(|s| s.snapshots.is_empty()));
    c.provide(request(ProviderKind::Hover, 3)).unwrap();
    let timer = c.sessions[&p].pending.values().next().unwrap().timer;
    c.message(Inbound::TimerElapsed(timer)).unwrap();
    assert!(c.sessions[&p].pending.is_empty());
    assert!(state(|s| s.snapshots.is_empty()));
}

#[test]
fn roots_settings_idle_and_crash_recovery_keep_sessions_independent() {
    let (mut c, p) = setup();
    state(|s| s.roots.push(root(2, "C:/second")));
    let second = doc(2, "C:/second/main.go", "package second\n");
    c.message(Inbound::DocumentOpened(second)).unwrap();
    let p2 = state(|s| s.spawned.last().unwrap().0);
    let initialize = sent("initialize");
    response(
        &mut c,
        p2,
        initialize["id"].clone(),
        json!({"capabilities":caps()}),
    );
    assert_eq!(c.sessions.len(), 2);
    server(
        &mut c,
        p,
        401,
        "window/showMessage",
        json!({"type":1,"message":"load failed"}),
    );
    assert_eq!(c.statuses.values().max_by_key(|s| s.rank).unwrap().rank, 3);
    c.message(Inbound::ProcessExited(ProcessExit {
        process: p,
        code: Some(1),
        killed: false,
        error: None,
    }))
    .unwrap();
    let timer = c
        .timers
        .iter()
        .find_map(|(id, t)| matches!(t, Timer::Restart { .. }).then_some(*id))
        .unwrap();
    assert_eq!(state(|s| s.timers[&timer]), 1000);
    c.message(Inbound::TimerElapsed(timer)).unwrap();
    let next = state(|s| s.spawned.last().unwrap().0);
    assert_ne!(next, p);
    response(
        &mut c,
        next,
        sent("initialize")["id"].clone(),
        json!({"capabilities":caps()}),
    );
    assert_eq!(c.documents[&1].process, next);
    assert_eq!(c.documents[&2].process, p2);
    state(|s| {
        s.settings.insert(
            format!("{ID}.initialization"),
            json!("{\"test\":true}").to_string(),
        );
    });
    c.message(Inbound::SettingsChanged(vec![])).unwrap();
    assert!(state(|s| s.killed.contains(&next) && s.killed.contains(&p2)));
    c.message(Inbound::RootsChanged(vec![])).unwrap();
    assert!(c.sessions.is_empty());
    assert!(c.documents.is_empty());
}

#[test]
fn save_uses_captured_text_and_temporary_formatting_does_not_poison_queries() {
    let (mut c, p) = setup();
    let saved = host::capture_document(1).unwrap();
    c.message(Inbound::DocumentSaved(saved)).unwrap();
    assert!(
        sent("textDocument/didSave")["params"]["text"]
            .as_str()
            .unwrap()
            .starts_with("package main")
    );
    let mut formatting = request(ProviderKind::Formatting, 1);
    let temp = state(|s| {
        let snapshot = s.snapshots.get_mut(&formatting.snapshot).unwrap();
        snapshot.1 = "package temp\n".into();
        snapshot.1.len() as u64
    });
    formatting.length = temp;
    c.provide(formatting).unwrap();
    assert_eq!(c.documents[&1].text.value.as_ref(), "package temp\n");
    response(
        &mut c,
        p,
        sent("textDocument/formatting")["id"].clone(),
        json!([]),
    );
    c.provide(request(ProviderKind::Hover, 2)).unwrap();
    assert!(c.documents[&1].text.value.starts_with("package main"));
    response(
        &mut c,
        p,
        sent("textDocument/hover")["id"].clone(),
        json!({"contents":"restored"}),
    );
    assert!(state(|s| s.snapshots.is_empty()));
}

#[test]
fn paths_selectors_and_semantic_delta_validate_boundaries() {
    for path in [
        "C:\\中 x\\😀.go",
        "\\\\?\\C:\\project\\main.go",
        "//server/share/a b.go",
        "/CaseSensitive/main.go",
    ] {
        assert_eq!(key(&crate::protocol::path(&uri(path)).unwrap()), key(path));
    }
    assert!(!inside("/foo/project2/a.go", "/foo/project"));
    assert!(!inside("/case/a.go", "/Case"));
    assert!(inside("C:/PROJECT/a.go", "c:/project"));
    assert!(inside("C:/a.go", "C:/"));
    let reference = DocumentRef {
        document: 1,
        lifecycle: 1,
        path: "C:/project/main.go".into(),
        language: "go".into(),
        variant: None,
        version: 0,
    };
    assert!(selector(
        &json!([{"language":"go","pattern":"**/*.go"}]),
        &reference
    ));
    assert!(!selector(&json!([{"language":"rust"}]), &reference));
    assert!(
        results::semantic_delta(
            &[0, 0, 1, 0, 0],
            &[
                json!({"start":0,"deleteCount":3}),
                json!({"start":2,"deleteCount":1})
            ]
        )
        .is_err()
    );
    assert_eq!(
        results::semantic_delta(
            &[0, 0, 1, 0, 0],
            &[json!({"start":2,"deleteCount":1,"data":[2]})]
        )
        .unwrap(),
        vec![0, 0, 2, 0, 0]
    );
}

#[test]
fn repeated_large_document_queries_reuse_the_immutable_capture() {
    let (mut c, p) = setup();
    let text = "中😀\r\n".repeat(1_048_576);
    state(|s| {
        let (reference, value) = s.documents.get_mut(&1).unwrap();
        reference.version += 1;
        *value = text;
    });
    c.message(Inbound::DocumentChanged(DocumentChange {
        document: 1,
        lifecycle: 1,
        from_version: 0,
        to_version: 1,
        edits: 1,
    }))
    .unwrap();
    let before = state(|s| s.read_bytes);
    for id in 1..=4 {
        let mut call = request(ProviderKind::Hover, id);
        call.cursor = Some(0);
        c.provide(call).unwrap();
        response(
            &mut c,
            p,
            sent("textDocument/hover")["id"].clone(),
            json!({"contents":"same immutable revision"}),
        );
    }
    assert_eq!(
        state(|s| s.read_bytes),
        before,
        "query-only calls must not recopy the complete document"
    );
    assert!(state(|s| s.snapshots.is_empty()));
}

#[test]
fn workspace_symbols_wait_for_an_initializing_second_root_and_merge_both() {
    let (mut c, p) = setup();
    state(|s| s.roots.push(root(2, "C:/second")));
    let reference = doc(2, "C:/second/main.go", "package second\n");
    c.message(Inbound::DocumentOpened(reference)).unwrap();
    let second = state(|s| s.spawned.last().unwrap().0);
    let initialize = sent("initialize");
    c.provide(request(ProviderKind::WorkspaceSymbols, 100))
        .unwrap();
    let first = sent("workspace/symbol");
    response(
        &mut c,
        p,
        first["id"].clone(),
        json!([{"name":"one","kind":12,"location":{"uri":"file:///C:/project/main.go","range":span()}}]),
    );
    assert!(
        state(|s| s.answers.is_empty()),
        "must wait for the second root"
    );
    response(
        &mut c,
        second,
        initialize["id"].clone(),
        json!({"capabilities":caps()}),
    );
    response(
        &mut c,
        second,
        sent("workspace/symbol")["id"].clone(),
        json!([{"name":"two","kind":12,"location":{"uri":"file:///C:/second/main.go"}}]),
    );
    state(|s| match s.answers.last().unwrap() {
        Answer::Language(100, LanguageAnswer::WorkspaceSymbols(values)) => {
            assert_eq!(values.len(), 2);
            assert_eq!(values[1].location.range.start.line, 0);
        }
        other => panic!("{other:?}"),
    });
}

#[test]
fn folder_addition_keeps_a_failed_root_document_and_closing_it_cancels_recovery() {
    let (mut c, p) = setup();
    c.message(Inbound::ProcessExited(ProcessExit {
        process: p,
        code: Some(1),
        killed: false,
        error: None,
    }))
    .unwrap();
    let roots = vec![root(1, "C:/project"), root(2, "C:/second")];
    state(|s| s.roots = roots.clone());
    c.message(Inbound::RootsChanged(roots)).unwrap();
    assert!(c.documents.contains_key(&1));
    assert!(
        c.timers
            .values()
            .any(|t| matches!(t, Timer::Restart { .. }))
    );
    c.close(1).unwrap();
    assert!(
        !c.timers
            .values()
            .any(|t| matches!(t, Timer::Restart { .. }))
    );
    assert!(!c.statuses.contains_key("1:"));
}

#[test]
fn resolve_actions_lenses_and_hierarchies_keep_their_origin() {
    let (mut c, p) = setup();
    for (kind, method, raw, answer) in [
        (
            ProviderKind::CodeActionResolve,
            "codeAction/resolve",
            json!({"title":"fix","data":{"id":1}}),
            json!({"title":"fix","command":{"command":"gopls.test"}}),
        ),
        (
            ProviderKind::CodeLensResolve,
            "codeLens/resolve",
            json!({"range":span(),"data":{"id":2}}),
            json!({"range":span(),"command":{"command":"gopls.test","title":"Run"}}),
        ),
        (
            ProviderKind::CallHierarchy,
            "callHierarchy/incomingCalls",
            json!({"name":"main","uri":"file:///C:/project/main.go","range":span(),"selectionRange":span(),"kind":12}),
            json!([{"from":{"name":"main","uri":"file:///C:/project/main.go","range":span(),"selectionRange":span(),"kind":12},"fromRanges":[span()]}]),
        ),
        (
            ProviderKind::TypeHierarchy,
            "typeHierarchy/supertypes",
            json!({"name":"Counter","uri":"file:///C:/project/main.go","range":span(),"selectionRange":span(),"kind":23}),
            json!([]),
        ),
    ] {
        let mut call = request(kind, 100);
        let context = call.context.as_mut().unwrap();
        context.data = Some(results::opaque("1:", &raw));
        if kind == ProviderKind::CallHierarchy {
            context.mode = Some(ProviderMode::Incoming);
        }
        if kind == ProviderKind::TypeHierarchy {
            context.mode = Some(ProviderMode::Supertypes);
        }
        c.provide(call).unwrap();
        let rpc = sent(method);
        response(&mut c, p, rpc["id"].clone(), answer);
        assert!(state(|s| matches!(
            s.answers.last(),
            Some(Answer::Language(100, _))
        )));
        state(|s| s.answers.clear());
    }
    let mut call = request(ProviderKind::CodeLensResolve, 101);
    call.context.as_mut().unwrap().data =
        Some(results::opaque("other-root", &json!({"range":span()})));
    assert!(
        c.provide(call)
            .unwrap_err()
            .contains("another server session")
    );
    assert!(state(|s| s.snapshots.is_empty()));
}

#[test]
fn recovery_uses_one_two_four_second_backoff_then_stops() {
    let (mut c, mut p) = setup();
    for milliseconds in [1000, 2000, 4000] {
        c.message(Inbound::ProcessExited(ProcessExit {
            process: p,
            code: Some(1),
            killed: false,
            error: None,
        }))
        .unwrap();
        let timer = c
            .timers
            .iter()
            .find_map(|(id, t)| matches!(t, Timer::Restart { .. }).then_some(*id))
            .unwrap();
        assert_eq!(state(|s| s.timers[&timer]), milliseconds);
        c.message(Inbound::TimerElapsed(timer)).unwrap();
        p = state(|s| s.spawned.last().unwrap().0);
        response(
            &mut c,
            p,
            sent("initialize")["id"].clone(),
            json!({"capabilities":caps()}),
        );
    }
    c.message(Inbound::ProcessExited(ProcessExit {
        process: p,
        code: Some(1),
        killed: false,
        error: None,
    }))
    .unwrap();
    assert!(
        !c.timers
            .values()
            .any(|t| matches!(t, Timer::Restart { .. }))
    );
}

#[test]
fn status_progress_idle_shutdown_and_workspace_changes_follow_the_contract() {
    let (mut c, p) = setup();
    server(
        &mut c,
        p,
        1,
        "$/progress",
        json!({"token":1,"value":{"kind":"begin","title":"Index","message":"Loading"}}),
    );
    server(
        &mut c,
        p,
        2,
        "$/progress",
        json!({"token":1,"value":{"kind":"report","percentage":50}}),
    );
    assert!(state(|s| s.reports.values().any(|report| {
        report.state == crate::loom::service::status::State::Working
    })));
    server(
        &mut c,
        p,
        3,
        "$/progress",
        json!({"token":1,"value":{"kind":"end"}}),
    );
    assert_eq!(c.statuses["1:"].rank, 1);
    c.message(Inbound::WorkspaceFilesChanged(vec![
        WorkspaceFileChange::Created("C:/project/new.go".into()),
        WorkspaceFileChange::Renamed(("C:/project/a.go".into(), "C:/project/b.go".into())),
        WorkspaceFileChange::Deleted("C:/project/old.go".into()),
    ]))
    .unwrap();
    assert_eq!(
        sent("workspace/didRenameFiles")["params"]["files"][0]["newUri"],
        "file:///C:/project/b.go"
    );
    c.close(1).unwrap();
    let timer = c.sessions[&p].idle.unwrap();
    c.message(Inbound::TimerElapsed(timer)).unwrap();
    response(&mut c, p, sent("shutdown")["id"].clone(), Value::Null);
    assert!(state(|s| s.killed.contains(&p)));
    assert_eq!(sent("exit")["params"], Value::Null);
    assert!(c.sessions.is_empty());
}

#[test]
fn completion_batch_and_opaque_fallback_preserve_lsp_defaults_and_overrides() {
    let text = Text::new("abc", ColumnUnit::Utf16);
    let item = json!({"label":"candidate","insertText":"insert","textEditText":"replace","insertTextMode":1});
    let defaults = json!({"data":{"ticket":7},"insertTextFormat":2,"insertTextMode":2,"commitCharacters":[";"],"editRange":{"insert":span(),"replace":span()}});
    let batch = results::completion(&item, &defaults, &text, "session", false).unwrap();
    let fallback = results::completion(&item, &defaults, &text, "session", true).unwrap();
    for value in [&batch, &fallback] {
        assert_eq!(value.insert_text_mode, Some(1));
        assert_eq!(value.insert_text_format, Some(2));
        assert_eq!(value.commit_characters, vec![";"]);
        assert_eq!(value.text_edit.as_ref().unwrap().text, "replace");
        assert_eq!(value.text_edit.as_ref().unwrap().range.start, 0);
    }
    assert!(batch.data.is_none());
    let original = results::original(fallback.data.as_deref(), "session").unwrap();
    assert_eq!(original["data"]["ticket"], 7);
    assert_eq!(original["textEdit"]["newText"], "replace");
    assert_eq!(original["insertTextMode"], 1);
    assert!(item.get("textEdit").is_none());
    assert!(item.get("data").is_none());
}

#[test]
fn readiness_is_published_after_negotiated_provider_availability() {
    let (_client, _) = setup();
    assert!(state(
        |s| !s.ready_hover.is_empty() && s.ready_hover.iter().all(|enabled| *enabled)
    ));
}

#[test]
fn cached_push_diagnostics_follow_unicode_edits_until_the_next_publication() {
    for unit in [ColumnUnit::Utf8, ColumnUnit::Utf16] {
        let before = Text::new("alpha beta", unit);
        let after = Text::new("中😀alpha beta", unit);
        let mut values = vec![
            json!({"range":{"start":before.position(0).unwrap(),"end":before.position(5).unwrap()},"message":"finding","code":7}),
        ];
        results::rebase_diagnostics(
            &mut values,
            &before,
            &after,
            &before.diff(&after.value).unwrap(),
        )
        .unwrap();
        let converted = results::diagnostics(&values, &after);
        assert_eq!(converted[0].range.start, 7);
        assert_eq!(converted[0].range.end, 12);
        assert_eq!(values[0]["code"], 7);
        let replaced = Text::new("中😀X beta", unit);
        results::rebase_diagnostics(
            &mut values,
            &after,
            &replaced,
            &after.diff(&replaced.value).unwrap(),
        )
        .unwrap();
        assert!(
            values.is_empty(),
            "a finding wholly replaced by typing expires"
        );
    }
}

fn publish_for(c: &mut Client, process: u64, id: u64, uri: &str, line: u64) {
    server(
        c,
        process,
        id,
        "textDocument/publishDiagnostics",
        json!({"uri":uri,"diagnostics":[
            {"range":{"start":{"line":line,"character":3},"end":{"line":line,"character":9}},
             "severity":2,"message":"unused","source":"rustc","code":"E0599"}
        ]}),
    );
}

#[test]
fn a_file_nobody_has_open_is_told_to_loom_as_a_finding_in_the_servers_units() {
    use crate::loom::service::findings::Encoding;
    let (mut c, p) = setup();
    publish_for(&mut c, p, 1, "file:///C:/project/other.go", 4);
    let key = c.sessions[&p].key.clone();
    state(|s| {
        assert!(s.diagnostics.is_empty(), "nothing goes the document way");
        let (encoding, items) = &s.findings[&(key.clone(), "C:/project/other.go".to_owned())];
        assert_eq!(*encoding, Encoding::Utf16);
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].start.line, items[0].start.character), (4, 3));
        assert_eq!((items[0].end.line, items[0].end.character), (4, 9));
        assert_eq!(items[0].severity, 2);
        assert_eq!(items[0].code.as_deref(), Some("E0599"));
    });
    // The server clearing the file takes it out.
    server(
        &mut c,
        p,
        2,
        "textDocument/publishDiagnostics",
        json!({"uri":"file:///C:/project/other.go","diagnostics":[]}),
    );
    state(|s| assert!(s.findings.is_empty()));
}

#[test]
fn an_open_file_keeps_its_findings_with_its_document() {
    let (mut c, p) = setup();
    let version = c.documents[&1].server_version.unwrap();
    server(
        &mut c,
        p,
        1,
        "textDocument/publishDiagnostics",
        json!({"uri":"file:///C:/project/main.go","version":version,"diagnostics":[{"range":span(),"message":"mine"}]}),
    );
    state(|s| {
        assert_eq!(s.diagnostics.len(), 1);
        assert!(s.findings.is_empty());
    });
    // A publication for an older state of the open file is dropped, not listed.
    server(
        &mut c,
        p,
        2,
        "textDocument/publishDiagnostics",
        json!({"uri":"file:///C:/project/main.go","version":version+9,"diagnostics":[{"range":span(),"message":"stale"}]}),
    );
    state(|s| assert!(s.findings.is_empty()));
}

#[test]
fn closing_a_file_keeps_its_problems_and_stopping_the_server_takes_them_back() {
    let (mut c, p) = setup();
    let version = c.documents[&1].server_version.unwrap();
    server(
        &mut c,
        p,
        1,
        "textDocument/publishDiagnostics",
        json!({"uri":"file:///C:/project/main.go","version":version,"diagnostics":[{"range":span(),"message":"kept"}]}),
    );
    c.close(1).unwrap();
    let key = c.sessions[&p].key.clone();
    state(|s| {
        let (_, items) = &s.findings[&(key.clone(), "C:/project/main.go".to_owned())];
        assert_eq!(items[0].message, "kept");
    });
    c.message(Inbound::RootsChanged(vec![])).unwrap();
    state(|s| {
        assert!(s.findings.is_empty());
        assert!(s.finding_withdrawals.contains(&key));
    });
}

#[test]
fn findings_respect_loom_bounds_and_skip_what_cannot_be_placed() {
    let long = "é".repeat(2000);
    let mut values = vec![
        json!({"range":span(),"message":long,"code":{"x":1},"severity":9}),
        json!({"message":"no range"}),
        json!({"range":{"start":{"line":-1,"character":0},"end":{"line":0,"character":0}},"message":"negative"}),
    ];
    let found = crate::findings::convert(&values);
    assert_eq!(found.len(), 1, "an unplaceable finding is left out");
    assert!(found[0].message.len() <= 2000 && found[0].message.chars().all(|c| c == 'é'));
    assert_eq!(found[0].severity, 1, "a severity outside 1 to 4 is an error");
    assert_eq!(found[0].code.as_deref(), Some("{\"x\":1}"));
    assert_eq!(found[0].source.as_deref(), Some(crate::NAME));
    values = (0..600)
        .map(|n| json!({"range":span(),"message":format!("m{n}")}))
        .collect();
    assert_eq!(crate::findings::convert(&values).len(), 500);
}

#[test]
fn a_session_reports_its_state_and_one_line_and_is_withdrawn_with_the_client() {
    use crate::loom::service::status::State;
    state(|s| *s = Default::default());
    let mut widgets = crate::status::Widgets::default();
    let update = |widgets: &mut crate::status::Widgets, rank, detail: &str| {
        widgets.update(std::collections::BTreeMap::from([(
            "1:".to_owned(),
            (
                Status {
                    rank,
                    detail: detail.into(),
                },
                vec!["C:/project/main.go".to_owned()],
            ),
        )]))
    };
    for (rank, detail, expected, line) in [
        (2, "Starting", State::Starting, Some("Starting")),
        (2, "Indexing 50%", State::Working, Some("Indexing 50%")),
        (1, "", State::Ready, None),
        (3, "failed", State::Error, Some("failed")),
        (2, "Restarting", State::Starting, Some("Restarting")),
        (0, "", State::Stopped, None),
    ] {
        update(&mut widgets, rank, detail);
        state(|s| {
            assert_eq!(s.reports.len(), 1, "one session is one report");
            let report = s.reports.values().next().unwrap();
            assert_eq!(report.state, expected, "rank {rank}, {detail:?}");
            assert_eq!(report.detail.as_deref(), line);
            assert_eq!(
                report.contexts.as_ref().unwrap(),
                &["C:/project/main.go"],
                "the report is about the document the session serves"
            );
        });
    }
    // A long, spread-out line is one short line.
    update(&mut widgets, 2, &format!("a   b\n{}", "x".repeat(200)));
    state(|s| {
        let detail = s.reports.values().next().unwrap().detail.clone().unwrap();
        assert!(detail.chars().count() <= 80 && !detail.contains('\n'));
    });
    widgets.dispose();
    assert!(state(|s| s.reports.is_empty()));
}

#[test]
fn status_keeps_roots_independent_and_releases_only_closed_documents() {
    use crate::loom::service::status::State;
    let (mut client, first) = setup();
    state(|s| s.roots.push(root(2, "C:/other")));
    let reference = doc(2, "C:/other/other.go", "package other\n");
    client.message(Inbound::DocumentOpened(reference)).unwrap();
    state(|s| {
        assert_eq!(s.reports.len(), 2);
        assert!(s.reports.values().any(|report| {
            report.contexts.as_ref().unwrap() == &["C:/project/main.go"]
                && report.state == State::Ready
        }));
        assert!(s.reports.values().any(|report| {
            report.contexts.as_ref().unwrap() == &["C:/other/other.go"]
                && matches!(report.state, State::Starting | State::Working)
        }));
    });
    client
        .message(Inbound::ProcessExited(ProcessExit {
            process: first,
            code: Some(1),
            killed: false,
            error: None,
        }))
        .unwrap();
    state(|s| {
        assert!(s.reports.values().any(|report| {
            report.contexts.as_ref().unwrap() == &["C:/project/main.go"]
                && report.state == State::Error
        }));
        assert!(s.reports.values().any(|report| {
            report.contexts.as_ref().unwrap() == &["C:/other/other.go"]
                && matches!(report.state, State::Starting | State::Working)
        }));
    });
    client.close(1).unwrap();
    state(|s| {
        assert_eq!(s.reports.len(), 1);
        assert_eq!(
            s.reports.values().next().unwrap().contexts.as_ref().unwrap(),
            &["C:/other/other.go"]
        );
    });
    client.message(Inbound::Shutdown).unwrap();
    assert!(state(|s| s.reports.is_empty()));
}

//! Bounded RPC, server requests, watches and recovery timers.
use super::*;

impl Client {
    pub(super) fn input(&mut self, s: &mut Session, text: &str) -> Result<()> {
        if text.len() > MAX_MESSAGE {
            return Err("language server message budget exceeded".into());
        }
        let start = host::monotonic_nanoseconds();
        let mut message: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        self.record("json-decode", start);
        if !message.is_object() || message["jsonrpc"] != "2.0" {
            return Err("invalid JSON-RPC message".into());
        }
        if let Some(method) = message["method"].as_str() {
            // File watches are acknowledged asynchronously. Preserve the order of
            // subsequent capability changes, including ones sent during startup.
            if s.registration.is_some()
                && matches!(
                    method,
                    "client/registerCapability" | "client/unregisterCapability"
                )
            {
                if s.registration_queue.len() >= 16 || text.len() > 64 * 1024 {
                    return s.answer(
                        message["id"].take(),
                        Err("capability registration queue limit exceeded".into()),
                    );
                }
                s.registration_queue.push_back(text.to_owned());
                return Ok(());
            }
            let method = method.to_owned();
            let id = message["id"].take();
            let params = message["params"].take();
            let result = self.receive(s, &method, params, &id);
            match result {
                Ok(Some(result)) => s.answer(id, Ok(result))?,
                Ok(None) => {}
                Err(error) => s.answer(id, Err(error))?,
            }
            return Ok(());
        }
        let id = message["id"].as_u64().ok_or("invalid RPC response id")?;
        let Some(mut pending) = s.pending.remove(&id) else {
            return Ok(());
        };
        self.cancel_timer(Some(pending.timer));
        self.record(&format!("rpc:{}", pending.method), pending.started);
        if !message["error"].is_null() {
            let code = message["error"]["code"].as_i64().unwrap_or(0);
            let error = format!("{code}: {}", string(&message["error"]["message"]));
            let valid = match &pending.purpose {
                Purpose::Provider(call) => {
                    self.documents.get(&call.request.document).is_some_and(|d| {
                        d.reference.lifecycle == call.reference.lifecycle
                            && d.reference.version == call.reference.version
                    })
                }
                _ => false,
            };
            if matches!(code, -32801 | -32802) && !pending.retried && valid {
                s.next += 1;
                let next = s.next;
                pending.retried = true;
                pending.timer = self.timer(5000, Timer::Rpc(s.process, next))?;
                let value = json!({"jsonrpc":"2.0","id":next,"method":pending.method,"params":pending.params});
                s.pending.insert(next, pending);
                return s.send(value, Some(next));
            }
            if matches!(pending.purpose, Purpose::Initialize) {
                self.failed(s, error);
                return Ok(());
            }
            host::log(1, &error);
            if !valid
                || !matches!(&pending.purpose, Purpose::Provider(call) if call.diagnostic_pull.is_some())
            {
                self.discard(pending.purpose);
                return Ok(());
            }
        }
        let mut answer = message["result"].take();
        match pending.purpose {
            Purpose::Initialize => {
                s.capabilities = answer
                    .get("capabilities")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match s.capabilities["positionEncoding"].as_str() {
                    Some("utf-8") => s.unit = ColumnUnit::Utf8,
                    Some("utf-16") | None => s.unit = ColumnUnit::Utf16,
                    Some(other) => {
                        self.failed(s, format!("unsupported position encoding {other}"));
                        return Ok(());
                    }
                }
                s.notify("initialized", json!({}))?;
                s.notify(
                    "workspace/didChangeConfiguration",
                    json!({"settings":self.setting("configuration",json!({}))}),
                )?;
                s.ready = true;
                // Publish negotiated provider availability before advertising readiness.
                self.configure(Some(s));
                self.configuration_dirty = false;
                s.stable = Some(self.timer(60000, Timer::Stable(s.process))?);
                self.status(s);
                let documents = self
                    .documents
                    .iter()
                    .filter(|(_, d)| d.process == s.process)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for id in documents {
                    self.synchronize(s, id, None)?;
                }
                let requests = std::mem::take(&mut s.waiting);
                for (owner, query) in std::mem::take(&mut s.waiting_symbols) {
                    if yes(&s.effective(None)["workspaceSymbolProvider"]) {
                        self.rpc(
                            s,
                            "workspace/symbol",
                            json!({"query":query}),
                            Purpose::Symbols(owner),
                            5000,
                        )?;
                    } else {
                        self.discard(Purpose::Symbols(owner));
                    }
                }
                for request in requests {
                    if let Err(error) = self.dispatch_provider(s, request) {
                        host::log(1, &format!("Provider: {error}"));
                    }
                }
            }
            Purpose::Shutdown => {
                let _ = s.notify("exit", Value::Null);
                self.stop_session(s);
                self.statuses.remove(&s.key);
                self.refresh_status();
            }
            Purpose::Provider(mut call) => {
                if let Some(pull) = &mut call.diagnostic_pull {
                    if answer["kind"] == "full" && pull.bytes + text.len() <= MAX_MESSAGE {
                        pull.bytes += text.len();
                        if let Value::Array(items) = answer["items"].take() {
                            pull.items.extend(items);
                        }
                    }
                    if let Some(identifier) = pull.remaining.pop_front() {
                        let mut params = pending.params;
                        params["identifier"] = json!(identifier);
                        let snapshot = call.request.snapshot;
                        let result =
                            self.rpc(s, &pending.method, params, Purpose::Provider(call), 5000);
                        if result.is_err() {
                            host::release_snapshot(snapshot);
                        }
                        return result;
                    }
                    if pull.bytes > 0 {
                        answer = json!({"kind":"full","items":std::mem::take(&mut pull.items)});
                    }
                }
                let started = host::monotonic_nanoseconds();
                let result = self.answer_provider(s, &call, answer, text.len());
                self.record(
                    &format!("convert:{}", kind_name(call.request.kind)),
                    started,
                );
                host::release_snapshot(call.request.snapshot);
                if let Err(error) = result {
                    host::log(1, &format!("Provider: {error}"));
                }
            }
            Purpose::Command(request) => {
                let request = request.map(|request| *request);
                if s.edits.is_empty() {
                    self.complete_command(request)?;
                } else {
                    s.completed.push(request);
                }
            }
            Purpose::Symbols(request) => {
                let coordinates = Text::new("", s.unit);
                let values = array(&answer)
                    .iter()
                    .filter(|v| v["location"]["uri"].is_string())
                    .map(|v| {
                        Ok(WorkspaceSymbol {
                            name: string(&v["name"]),
                            kind: number(&v["kind"]),
                            container: optional(&v["containerName"]),
                            location: results::location(&v["location"], &coordinates)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                if let Some(group) = self.symbols.get_mut(&request) {
                    group.values.extend(values);
                }
                self.discard(Purpose::Symbols(request));
            }
        }
        Ok(())
    }
    pub(super) fn complete_command(&self, request: Option<ProviderRequest>) -> Result<()> {
        if let Some(request) = request {
            host::release_snapshot(request.snapshot);
            host::provide_language(request.request, &LanguageAnswer::Completed)
                .map_err(|e| e.message)?;
        }
        Ok(())
    }
    pub(super) fn receive(
        &mut self,
        s: &mut Session,
        method: &str,
        params: Value,
        id: &Value,
    ) -> Result<Option<Value>> {
        let coordinates = Text::new("", s.unit);
        match method {
            "textDocument/publishDiagnostics" => {
                if let Ok(path) = path(&string(&params["uri"])) {
                    if let Some(doc) = self.documents.values_mut().find(|d| {
                        key(&d.reference.path) == key(&path)
                            && d.process == s.process
                            && d.server_version.is_some()
                            && (params["version"].is_null()
                                || params["version"].as_u64() == d.server_version)
                    }) {
                        doc.diagnostics = array(&params["diagnostics"]).to_vec();
                        host::publish_diagnostics(
                            &format!("{ID}.diagnostics"),
                            &doc.reference.path,
                            doc.reference.version,
                            &results::diagnostics(&doc.diagnostics, &doc.text),
                        )
                        .map_err(|e| e.message)?;
                    } else if !self
                        .documents
                        .values()
                        .any(|d| key(&d.reference.path) == key(&path))
                    {
                        // A file nobody has open: the server knows it from the workspace,
                        // such as the output of a build. Loog lists it with the problems.
                        crate::findings::publish(&s.key, &path, s.unit, array(&params["diagnostics"]));
                    }
                }
            }
            "workspace/configuration" => {
                let configuration = self.setting("configuration", json!({}));
                let values = array(&params["items"])
                    .iter()
                    .map(|v| {
                        if let Some(section) = v["section"].as_str().filter(|s| !s.is_empty()) {
                            section
                                .split('.')
                                .fold(&configuration, |value, key| &value[key])
                                .clone()
                        } else {
                            configuration.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                return Ok(Some(json!(values)));
            }
            "workspace/workspaceFolders" => {
                return Ok(Some(json!([{"uri":uri(&s.path),"name":s.root.name}])));
            }
            "workspace/applyEdit" | "window/showDocument" => {
                if self.operations.len() >= 64 {
                    return Err("workspace operation budget exceeded".into());
                }
                let show = method == "window/showDocument";
                let operation = if show {
                    WorkspaceOperation::ShowDocument(FileLocation{path:path(&string(&params["uri"]))?,range:coordinates.file_range(params.get("selection").unwrap_or(&json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}})))})
                } else {
                    WorkspaceOperation::Edit(WorkspaceEditRequest {
                        title: params["label"].as_str().unwrap_or(NAME).into(),
                        edit: results::workspace_edit(&params["edit"], &coordinates, &|url, v| {
                            self.protocol_version(url, v)
                        })?,
                    })
                };
                let operation =
                    host::perform_workspace_operation(&operation).map_err(|e| e.message)?;
                self.operations.insert(
                    operation,
                    Operation {
                        process: s.process,
                        id: id.clone(),
                        show,
                    },
                );
                s.edits.insert(operation);
                return Ok(None);
            }
            "window/workDoneProgress/create" => {}
            "experimental/serverStatus" => host::log(
                if params["health"] == "error" { 2 } else { 0 },
                &format!("serverStatus {params}"),
            ),
            "$/progress" => {
                let token = params["token"].to_string();
                if params["value"]["kind"] == "end" {
                    s.progress.retain(|(key, _)| key != &token);
                } else {
                    let previous = s.progress.iter().position(|(key, _)| key == &token);
                    if previous.is_none() && s.progress.len() >= 64 {
                        return Err("progress token budget exceeded".into());
                    }
                    let mut value = previous
                        .map(|i| s.progress[i].1.clone())
                        .unwrap_or_else(|| json!({}));
                    if let Some(update) = params["value"].as_object() {
                        for (key, v) in update {
                            value[key] = v.clone();
                        }
                    }
                    for field in ["title", "message"] {
                        if let Some(text) = value[field].as_str() {
                            value[field] = json!(text.chars().take(2000).collect::<String>());
                        }
                    }
                    if let Some(i) = previous {
                        s.progress[i].1 = value;
                    } else {
                        s.progress.push((token, value));
                    }
                }
                self.status(s);
            }
            "window/logMessage" | "window/showMessage" | "window/showMessageRequest" => {
                let text = string(&params["message"]);
                host::log(
                    match params["type"].as_u64() {
                        Some(1) => 2,
                        Some(2) => 1,
                        _ => 0,
                    },
                    &text,
                );
                if params["type"] == 1 && method != "window/logMessage" {
                    s.server_error = Some(text.chars().take(4000).collect());
                    self.status(s);
                }
            }
            "client/registerCapability" => return self.register(s, &params, id),
            "client/unregisterCapability" => {
                // COMPAT(lsp-unregistration-spelling): Accept the protocol's historical
                // `unregisterations` spelling and servers using `unregistrations`.
                // Existing server versions require both. New senders should follow
                // the negotiated LSP schema. Remove an alias when supported servers
                // no longer send it; no removal version has been scheduled.
                let registrations = params
                    .get("unregisterations")
                    .or(params.get("unregistrations"))
                    .unwrap_or(&Value::Null);
                let values = registrations
                    .as_array()
                    .ok_or("invalid unregistration batch")?;
                if values.len() > 64 || values.iter().any(|v| !v["id"].is_string()) {
                    return Err("invalid unregistration batch".into());
                }
                let mut changed = BTreeSet::new();
                for registration in values {
                    let id = string(&registration["id"]);
                    if let Some(old) = s.dynamic.remove(&id) {
                        changed.insert(old.field);
                    }
                    if let Some(watches) = s.watches.remove(&id) {
                        for watch in watches {
                            host::unwatch_files(watch.handle);
                        }
                    }
                }
                if changed.contains("semanticTokensProvider") {
                    s.semantic.clear();
                }
                self.changed(&changed);
            }
            "workspace/semanticTokens/refresh"
            | "workspace/diagnostic/refresh"
            | "workspace/inlayHint/refresh"
            | "workspace/codeLens/refresh" => {
                let kind = match method {
                    "workspace/semanticTokens/refresh" => {
                        s.semantic.clear();
                        "highlights"
                    }
                    "workspace/diagnostic/refresh" => "diagnostics",
                    "workspace/inlayHint/refresh" => "inlay-hints",
                    _ => "code-lens",
                };
                *self.revisions.entry(kind.into()).or_default() += 1;
                self.configuration_dirty = true;
            }
            _ if method.starts_with("$/") => {}
            _ => return Err(format!("unsupported server request: {method}")),
        }
        Ok(Some(Value::Null))
    }
    pub(super) fn operation(&mut self, id: u64, result: Completion) -> Result<()> {
        let Some(operation) = self.operations.remove(&id) else {
            return Ok(());
        };
        self.with_session(operation.process, |c, s| {
            s.edits.remove(&id);
            let (committed, error) = match result {
                Completion::Committed(_) => (true, None),
                Completion::Failed(error) => (false, Some(error.message)),
            };
            let answer = if operation.show {
                json!({"success":committed})
            } else {
                json!({"applied":committed,"failureReason":error})
            };
            s.answer(operation.id, Ok(answer))?;
            if s.edits.is_empty() {
                for command in std::mem::take(&mut s.completed) {
                    c.complete_command(command)?;
                }
            }
            Ok(())
        })?;
        Ok(())
    }
    pub(super) fn register(
        &mut self,
        s: &mut Session,
        params: &Value,
        id: &Value,
    ) -> Result<Option<Value>> {
        let values = params["registrations"]
            .as_array()
            .ok_or("invalid registration batch")?;
        if values.len() > 64 {
            return Err("invalid registration batch".into());
        }
        let mut registration = Registration {
            id: id.clone(),
            previous_dynamic: s.dynamic.clone(),
            previous_watches: s.watches.clone(),
            created: BTreeSet::new(),
            waiting: BTreeSet::new(),
            changed: BTreeSet::new(),
        };
        let mut ids = BTreeSet::new();
        let result = (|| -> Result<()> {
            for value in values {
                let id = value["id"]
                    .as_str()
                    .filter(|v| !v.is_empty() && v.len() <= 256)
                    .ok_or("invalid dynamic registration")?;
                let method = value["method"]
                    .as_str()
                    .ok_or("invalid dynamic registration")?;
                if !ids.insert(id.to_owned()) {
                    return Err("duplicate dynamic registration in one batch".into());
                }
                if let Some(old) = s.dynamic.remove(id) {
                    registration.changed.insert(old.field);
                }
                s.watches.remove(id);
                if method == "workspace/didChangeWatchedFiles" {
                    let watchers = array(&value["registerOptions"]["watchers"]);
                    if s.watches.len() >= 16 || watchers.len() > 16 {
                        return Err("watch registration budget exceeded".into());
                    }
                    let mut watches = vec![];
                    for watcher in watchers {
                        let kinds = watcher["kind"].as_u64().unwrap_or(7) as u32;
                        if !(1..=7).contains(&kinds)
                            || (!watcher["kind"].is_null() && watcher["kind"].as_u64().is_none())
                        {
                            return Err("invalid watch kinds".into());
                        }
                        let pattern = &watcher["globPattern"];
                        let relative = if pattern.is_string() {
                            pattern.as_str()
                        } else {
                            pattern["pattern"].as_str()
                        }
                        .ok_or("invalid watch glob")?
                        .replace('\\', "/");
                        let base = pattern["baseUri"]["uri"]
                            .as_str()
                            .or(pattern["baseUri"].as_str())
                            .map(path)
                            .transpose()?
                            .unwrap_or_else(|| s.path.clone());
                        let base = normalize(&base);
                        let root = normalize(&s.root.path);
                        // COMPAT(mixed-workspace-watches): JDT LS 1.63.0 requests workspace
                        // and external toolchain watches in the same registration. Retain
                        // the workspace subset until supported servers send scoped
                        // registrations themselves. No removal version is scheduled.
                        if key(&base) != key(&root) && !inside(&base, &root) {
                            continue;
                        }
                        let pattern = if relative.starts_with('/')
                            || relative.as_bytes().get(1) == Some(&b':')
                        {
                            if !inside(&relative, &root) {
                                continue;
                            }
                            relative[root.len()..].trim_start_matches('/').to_owned()
                        } else {
                            let prefix = base[root.len()..].trim_start_matches('/');
                            if prefix.is_empty() {
                                relative
                            } else {
                                format!("{prefix}/{relative}")
                            }
                        };
                        let handle = host::watch_files(&WatchRequest {
                            root: s.root.id,
                            patterns: vec![pattern],
                        })
                        .map_err(|e| e.message)?;
                        registration.created.insert(handle);
                        registration.waiting.insert(handle);
                        watches.push(Watch { handle, kinds });
                    }
                    s.watches.insert(id.into(), watches);
                } else {
                    let field = if let Some(name) = method.strip_prefix("textDocument/") {
                        Some(format!(
                            "{}Provider",
                            match name {
                                "formatting" => "documentFormatting",
                                "rangeFormatting" => "documentRangeFormatting",
                                "onTypeFormatting" => "documentOnTypeFormatting",
                                other => other,
                            }
                        ))
                    } else if method == "workspace/executeCommand" {
                        Some("executeCommandProvider".into())
                    } else {
                        None
                    };
                    if s.dynamic.len() >= 64 {
                        return Err("dynamic registration budget exceeded".into());
                    }
                    if let Some(field) = field {
                        registration.changed.insert(field.clone());
                        s.dynamic.insert(
                            id.into(),
                            Dynamic {
                                field,
                                options: value
                                    .get("registerOptions")
                                    .cloned()
                                    .unwrap_or(json!(true)),
                            },
                        );
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            for handle in &registration.created {
                host::unwatch_files(*handle);
            }
            s.dynamic = registration.previous_dynamic;
            s.watches = registration.previous_watches;
            return Err(error);
        }
        if registration.waiting.is_empty() {
            self.commit_registration(s, registration);
            Ok(Some(Value::Null))
        } else {
            s.registration = Some(registration);
            Ok(None)
        }
    }
    pub(super) fn commit_registration(&mut self, s: &mut Session, registration: Registration) {
        let active = s
            .watches
            .values()
            .flatten()
            .map(|w| w.handle)
            .collect::<BTreeSet<_>>();
        for watches in registration.previous_watches.values() {
            for watch in watches {
                if !active.contains(&watch.handle) {
                    host::unwatch_files(watch.handle);
                }
            }
        }
        if registration.changed.contains("semanticTokensProvider") {
            s.semantic.clear();
        }
        self.changed(&registration.changed);
    }
    pub(super) fn watch_ready(&mut self, id: u64, error: Option<String>) -> Result<()> {
        let process = self
            .sessions
            .values()
            .find(|s| {
                s.registration
                    .as_ref()
                    .is_some_and(|r| r.waiting.contains(&id))
                    || s.watches.values().flatten().any(|w| w.handle == id)
            })
            .map(|s| s.process);
        if let Some(process) = process {
            self.with_session(process, |c, s| {
                if let Some(mut registration) = s.registration.take() {
                    if registration.waiting.contains(&id) {
                        if let Some(error) = error {
                            for handle in &registration.created {
                                host::unwatch_files(*handle);
                            }
                            s.dynamic = registration.previous_dynamic;
                            s.watches = registration.previous_watches;
                            s.answer(registration.id, Err(error))?;
                        } else {
                            registration.waiting.remove(&id);
                            if registration.waiting.is_empty() {
                                let id = registration.id.clone();
                                c.commit_registration(s, registration);
                                s.answer(id, Ok(Value::Null))?;
                            } else {
                                s.registration = Some(registration);
                            }
                        }
                    } else {
                        s.registration = Some(registration);
                    }
                } else if let Some(error) = error {
                    host::log(1, &format!("Watch: {error}"));
                    c.restart(s)?;
                }
                while s.registration.is_none() {
                    let Some(message) = s.registration_queue.pop_front() else {
                        break;
                    };
                    c.input(s, &message)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    pub(super) fn file_changed(&mut self, event: FileChanged) -> Result<()> {
        let process = self
            .sessions
            .values()
            .find(|s| {
                s.watches
                    .values()
                    .flatten()
                    .any(|w| w.handle == event.watch)
            })
            .map(|s| s.process);
        if let Some(process) = process {
            self.with_session(process,|c,s|{if event.changes.iter().any(|v|v.kind==FileChangeKind::Rescan){return c.restart(s);}let kinds=s.watches.values().flatten().find(|w|w.handle==event.watch).map_or(7,|w|w.kinds);let changes=event.changes.iter().filter_map(|v|{let kind=match v.kind{FileChangeKind::Create=>1,FileChangeKind::Delete=>3,_=>2};(kinds&(1<<(kind-1))!=0).then(||json!({"uri":uri(&format!("{}/{}",s.root.path.trim_end_matches(['\\','/']),v.path)),"type":kind}))}).collect::<Vec<_>>();if !changes.is_empty(){s.notify("workspace/didChangeWatchedFiles",json!({"changes":changes}))?;}Ok(())})?;
        }
        Ok(())
    }
    pub(super) fn cancel(&mut self, request: u64) {
        self.symbols.remove(&request);
        let processes = self.sessions.keys().copied().collect::<Vec<_>>();
        for process in processes {
            let _ = self.with_session(process, |c, s| {
                s.waiting_symbols.retain(|(owner, _)| *owner != request);
                let ids = s
                    .pending
                    .iter()
                    .filter_map(|(id, p)| match &p.purpose {
                        Purpose::Provider(call) if call.request.request == request => Some(*id),
                        Purpose::Symbols(owner) if *owner == request => Some(*id),
                        Purpose::Command(Some(call)) if call.request == request => Some(*id),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for id in ids {
                    if let Some(p) = s.pending.remove(&id) {
                        c.cancel_timer(Some(p.timer));
                        c.discard(p.purpose);
                        s.outbox.retain(|f| f.request != Some(id));
                        let _ = s.notify("$/cancelRequest", json!({"id":id}));
                    }
                }
                let queued = s.waiting.iter().position(|p| p.request == request);
                if let Some(i) = queued {
                    let p = s.waiting.remove(i).unwrap();
                    host::release_snapshot(p.snapshot);
                }
                Ok(())
            });
        }
    }
    pub(super) fn elapsed(&mut self, id: u64) -> Result<()> {
        let Some(timer) = self.timers.remove(&id) else {
            return Ok(());
        };
        match timer {
            Timer::Rpc(process, rpc) => {
                self.with_session(process, |c, s| {
                    if let Some(p) = s.pending.remove(&rpc) {
                        let message = format!("{} timed out", p.method);
                        if matches!(p.purpose, Purpose::Initialize) {
                            c.failed(s, message);
                        } else if matches!(p.purpose, Purpose::Shutdown) {
                            c.stop_session(s);
                            c.statuses.remove(&s.key);
                            c.refresh_status();
                        } else {
                            host::log(1, &message);
                            c.discard(p.purpose);
                            s.outbox.retain(|f| f.request != Some(rpc));
                            let _ = s.notify("$/cancelRequest", json!({"id":rpc}));
                        }
                    }
                    Ok(())
                })?;
            }
            Timer::Restart {
                previous_process,
                key,
                root,
                cwd,
                path,
                attempt,
            } => {
                if self.shutting_down || !host::list_roots().iter().any(|r| r.id == root.id) {
                    return Ok(());
                }
                let documents = self
                    .documents
                    .iter()
                    .filter(|(_, d)| d.process == previous_process)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                if documents.is_empty() {
                    return Ok(());
                }
                match self.start(root, cwd, path, attempt) {
                    Ok(process) => {
                        let root = self.sessions[&process].root.id;
                        for id in documents {
                            if let Some(doc) = self.documents.get_mut(&id)
                                && key.starts_with(&format!("{root}:"))
                                && inside(&doc.reference.path, &self.sessions[&process].path)
                            {
                                doc.process = process;
                                doc.server_version = None;
                                doc.diagnostics.clear();
                                doc.dirty = true;
                            }
                        }
                        self.owners.remove(&previous_process);
                    }
                    Err(error) => {
                        self.statuses.insert(
                            key,
                            Status {
                                rank: 3,
                                detail: error,
                            },
                        );
                        self.refresh_status();
                    }
                }
            }
            Timer::Idle(process) => {
                self.with_session(process, |c, s| {
                    s.idle = None;
                    if !c.documents.values().any(|d| d.process == process) {
                        c.rpc(s, "shutdown", Value::Null, Purpose::Shutdown, 1000)?;
                    }
                    Ok(())
                })?;
            }
            Timer::Stable(process) => {
                if let Some(s) = self.sessions.get_mut(&process) {
                    s.stable = None;
                    s.attempt = 0;
                }
            }
        }
        Ok(())
    }
}

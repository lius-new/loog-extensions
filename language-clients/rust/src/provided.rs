//! Resource-interface adapter compiled inside each opted-in language component.
//! Server policy, framing and document synchronization remain plugin-owned.
use crate::{
    CONFIG, ID, host,
    loog::{extension::types::*, workspace::session},
    protocol::{Text, array},
};
use loog_language_workspace::{
    DocumentEvent, Documents, Frames, LocationResult, LocationUris, Process, ProcessEvent,
    Processes, ReadyDocument,
    api::{
        self,
        session::{Command, Event, ResourceDiagnostic},
    },
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[path = "provided_actions.rs"]
mod actions;
#[path = "provided_capabilities.rs"]
mod capabilities;
#[path = "provided_completion.rs"]
mod completion;
#[path = "provided_diagnostics.rs"]
mod diagnostics;
#[path = "provided_format.rs"]
mod formatting;
#[path = "provided_locations.rs"]
mod locations;
#[path = "provided_program.rs"]
mod program;
#[path = "provided_rename.rs"]
mod rename;
#[path = "provided_edits.rs"]
mod server_edits;
#[path = "provided_status.rs"]
mod status;
#[path = "provided_tools.rs"]
mod tools;

const MESSAGE_LIMIT: usize = 8 * 1024 * 1024;
const TOTAL_TEXT: usize = 16 * 1024 * 1024;
fn now() -> u64 {
    host::monotonic_nanoseconds() / 1_000_000
}
fn submit(command: Command) -> Result<u64, String> {
    session::submit(&serde_json::to_string(&command).map_err(|error| error.to_string())?)
}
fn setting(settings: &loog_language_workspace::Settings, name: &str) -> Value {
    let value = settings
        .get(&format!("{ID}.{name}"))
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null);
    value
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(value)
}
fn error(message: impl AsRef<str>) {
    host::log(
        2,
        &format!(
            "{ID} environment: {}",
            message.as_ref().chars().take(1000).collect::<String>()
        ),
    );
}
struct Document {
    descriptor: api::session::DocumentResource,
    uri: String,
    text: Text,
    version: u64,
    sent: bool,
    diagnostics: Vec<Value>,
}
struct Session {
    settings: loog_language_workspace::Settings,
    scope: u64,
    project: api::ProviderPath,
    root_uri: String,
    frames: Frames,
    outgoing: VecDeque<Vec<u8>>,
    offset: usize,
    documents: BTreeMap<u64, Document>,
    ready: bool,
    unit: ColumnUnit,
    version: u64,
    deadline: Option<u64>,
    stopping: bool,
    eof: bool,
    next_request: u64,
    pending: BTreeMap<u64, (ProviderRequest, api::session::DocumentVersion)>,
    completed: Vec<(
        ProviderRequest,
        api::session::DocumentVersion,
        Option<String>,
    )>,
    capabilities: Value,
    pulls: diagnostics::Pulls,
    managed: bool,
    tool_error: Vec<u8>,
}
impl Session {
    fn send(&mut self, value: Value) -> Result<(), String> {
        let body = value.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes();
        if body.len() > MESSAGE_LIMIT
            || self.outgoing.len() >= 64
            || self.outgoing.iter().map(Vec::len).sum::<usize>() + frame.len()
                > MESSAGE_LIMIT + 64 * 1024
        {
            return Err("Language output queue exceeds its budget".into());
        }
        self.outgoing.push_back(frame);
        Ok(())
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    fn sync(&mut self, id: u64) -> Result<(), String> {
        if !self.ready || self.stopping {
            return Ok(());
        }
        let doc = self.documents.get_mut(&id).unwrap();
        doc.text.unit = self.unit;
        let value = if doc.sent {
            json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
                "textDocument":{"uri":doc.uri,"version":doc.version},"contentChanges":[{"text":doc.text.value.as_ref()}]}})
        } else {
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
                "uri":doc.uri,"languageId":program::language(&doc.descriptor.language, &doc.descriptor.variant, &doc.uri),"version":doc.version,"text":doc.text.value.as_ref()}}})
        };
        doc.sent = true;
        if self.capabilities["diagnosticProvider"].is_object() {
            self.pulls.queue(doc.descriptor.stamp);
        }
        self.send(value)?;
        self.pull_diagnostics()
    }
    fn pull_diagnostics(&mut self) -> Result<(), String> {
        if !self.ready || self.stopping {
            return Ok(());
        }
        while let Some((id, stamp)) = self.pulls.next(&mut self.next_request, now()) {
            let Some(doc) = self
                .documents
                .get(&stamp.document)
                .filter(|doc| doc.sent && doc.descriptor.stamp == stamp)
            else {
                self.pulls.finish(id);
                continue;
            };
            return self.send(json!({"jsonrpc":"2.0","id":id,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":doc.uri}}}));
        }
        Ok(())
    }
    fn shutdown(&mut self) -> Result<(), String> {
        if !self.stopping {
            self.stopping = true;
            self.deadline = Some(now() + 5000);
            if self.ready {
                self.send(json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}))?;
            }
        }
        Ok(())
    }
}
struct PendingEdit {
    process: Process,
    documents: Vec<rename::Document>,
    action: Option<api::session::ResourceCodeAction>,
}
pub(super) struct Client {
    documents: Documents,
    processes: Processes,
    locations: LocationUris,
    edits: BTreeMap<u64, PendingEdit>,
    server_edits: server_edits::Edits,
    configurations: capabilities::Configurations,
    reports: status::Reports,
    failures: BTreeMap<Process, api::session::ResourceServiceReport>,
    sessions: BTreeMap<Process, Session>,
    restarting: BTreeMap<u64, ReadyDocument>,
    failed: BTreeSet<(u64, api::ProviderPath)>,
    publications: BTreeSet<u64>,
    timer: Option<u64>,
    stopped: bool,
    bound: bool,
    binding: Option<u64>,
    contexts: BTreeMap<u64, api::session::DocumentResource>,
    waiting: BTreeMap<u64, (ProviderRequest, api::session::DocumentResource)>,
}
impl Default for Client {
    fn default() -> Self {
        Self {
            documents: Documents::new(CONFIG.languages.iter().map(|s| (*s).into()).collect())
                .with_projects(
                    CONFIG.markers.iter().map(|s| (*s).into()).collect(),
                    CONFIG.outermost,
                    CONFIG.file_fallback,
                )
                .with_settings(),
            processes: Processes::default(),
            locations: LocationUris::default(),
            edits: BTreeMap::new(),
            server_edits: server_edits::Edits::default(),
            configurations: capabilities::Configurations::default(),
            reports: status::Reports::default(),
            failures: BTreeMap::new(),
            sessions: BTreeMap::new(),
            restarting: BTreeMap::new(),
            failed: BTreeSet::new(),
            publications: BTreeSet::new(),
            timer: None,
            stopped: false,
            bound: false,
            binding: None,
            contexts: BTreeMap::new(),
            waiting: BTreeMap::new(),
        }
    }
}
impl Client {
    pub fn dispatch(&mut self, batch: &[Inbound]) -> BTreeSet<u64> {
        let mut handled = BTreeSet::new();
        let cancelled: BTreeSet<_> = batch
            .iter()
            .filter_map(|message| match message {
                Inbound::ProviderCancelled(request) => Some(*request),
                _ => None,
            })
            .collect();
        for message in batch {
            match message {
                Inbound::TimerElapsed(id) if Some(*id) == self.timer => self.timer = None,
                Inbound::Shutdown => {
                    self.restarting.clear();
                    self.locations.cancel_all();
                    self.edits.clear();
                    self.server_edits.cancel_all();
                    self.stopped = true;
                    self.documents.stop();
                    for session in self.sessions.values_mut() {
                        session.completed.clear();
                        if let Err(e) = session.shutdown() {
                            error(e);
                        }
                    }
                }
                Inbound::ProviderCancelled(request) => {
                    self.locations.cancel(*request);
                    self.edits.remove(request);
                    self.contexts.remove(request);
                    self.waiting.remove(request);
                    for session in self.sessions.values_mut() {
                        session
                            .completed
                            .retain(|(call, _, _)| call.request != *request);
                        let ids: Vec<_> = session
                            .pending
                            .iter()
                            .filter(|(_, (call, _))| call.request == *request)
                            .map(|(id, _)| *id)
                            .collect();
                        for id in ids {
                            session.pending.remove(&id);
                            if let Err(e) = session.notify("$/cancelRequest", json!({"id":id})) {
                                error(e);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for value in session::poll() {
            let event = match serde_json::from_str::<Event>(&value) {
                Ok(value) => value,
                Err(e) => {
                    error(e.to_string());
                    continue;
                }
            };
            if matches!(event, Event::SettingsChanged) {
                self.restarting.clear();
                // Base SettingsChanged reflects native project overrides. Only
                // this resource event invalidates environment-specific values.
                self.locations.cancel_all();
                for process in self.sessions.keys().copied().collect::<Vec<_>>() {
                    self.kill(process, None);
                }
                self.failed.clear();
                self.failures.clear();
            }
            if self.reports.receive(&event) || self.configurations.receive(&event) {
                continue;
            }
            let responses = self.server_edits.receive(&event, &mut submit);
            self.edit_responses(responses);
            if let Event::Reply { ticket, result } = &event {
                if self.binding == Some(*ticket) {
                    self.binding = None;
                    match result {
                        Ok(_) => self.bound = true,
                        Err(e) => {
                            error(&e.message);
                            self.stopped = true;
                            self.documents.stop();
                        }
                    }
                    continue;
                }
                if self.publications.remove(ticket) {
                    if let Err(e) = result {
                        if e.kind != api::FailureKind::Conflict {
                            error(&e.message);
                        }
                    }
                    continue;
                }
            }
            if let Event::ProviderDocument { request, document } = &event {
                if cancelled.contains(request) {
                    handled.insert(*request);
                    continue;
                }
                if self.contexts.len() < 32 {
                    self.contexts.insert(*request, document.clone());
                } else {
                    error("Resource provider context budget exceeded");
                }
                continue;
            }
            let changes = self.documents.receive(&event);
            self.document_events(changes);
            let changes = self.locations.receive(&event);
            self.location_results(changes);
            let changes = self.processes.receive(event, &mut submit);
            self.process_events(changes);
        }
        for message in batch {
            if let Inbound::ProviderRequest(request) = message {
                if let Some(document) = self.contexts.remove(&request.request) {
                    handled.insert(request.request);
                    if self.waiting.len() < 32 {
                        self.waiting
                            .insert(request.request, (request.clone(), document));
                    } else {
                        empty(request);
                    }
                }
            }
        }
        if !self.bound && self.binding.is_none() && !self.stopped {
            match submit(Command::BindLanguageProviders {
                providers: capabilities::KINDS
                    .iter()
                    .map(|kind| format!("{ID}.{kind}"))
                    .collect(),
            }) {
                Ok(ticket) => self.binding = Some(ticket),
                Err(e) => error(e),
            }
        }
        let changes = self.documents.drive(&mut submit);
        self.document_events(changes);
        let changes = self.processes.drive(&mut submit);
        self.process_events(changes);
        if self.processes.can_start() {
            for doc in std::mem::take(&mut self.restarting).into_values() {
                if self.documents.current(doc.descriptor.stamp)
                    && let Err(e) = self.open(doc)
                {
                    error(e);
                }
            }
        }
        self.request_waiting();
        let changes = self.locations.drive(&mut submit);
        self.location_results(changes);
        let responses = self.server_edits.drive(&mut submit);
        self.edit_responses(responses);
        let completed = self
            .sessions
            .iter_mut()
            .filter(|(process, _)| !self.server_edits.pending(**process))
            .flat_map(|(_, session)| std::mem::take(&mut session.completed))
            .collect::<Vec<_>>();
        for (request, stamp, reason) in completed {
            self.finish(request.request, stamp, reason);
        }
        for process in self.sessions.keys().copied().collect::<Vec<_>>() {
            if self.sessions[&process]
                .deadline
                .is_some_and(|deadline| now() >= deadline)
            {
                self.kill(
                    process,
                    Some("Language server initialization or shutdown timed out".into()),
                );
                continue;
            }
            if self.sessions[&process].pulls.expired(now()) {
                self.kill(
                    process,
                    Some("Language diagnostics request timed out".into()),
                );
                continue;
            }
            let result = self.flush(process);
            if let Err(e) = result {
                self.kill(process, Some(e));
            }
        }
        if self.bound {
            let mut configurations = vec![];
            for (process, session) in &self.sessions {
                let contexts = session
                    .documents
                    .values()
                    .map(|doc| doc.descriptor.resource.clone())
                    .collect::<Vec<_>>();
                if contexts.is_empty() {
                    continue;
                }
                for kind in capabilities::KINDS {
                    configurations.push(capabilities::describe(
                        kind,
                        &session.capabilities,
                        session.ready && !session.stopping,
                        process.0,
                        contexts.clone(),
                    ));
                }
            }
            self.configurations.update(configurations);
            self.configurations.drive(&mut submit);
        }
        self.report_status();
        self.reports.drive(&mut submit);
        // A bounded maintenance timer covers deadlines and a mailbox that
        // rejected submission before accepting it. No busy guest polling.
        let maintenance = self.documents.needs_drive()
            || (self.bound && self.configurations.needs_drive())
            || self.server_edits.needs_drive()
            || self.reports.needs_drive()
            || self.locations.needs_drive()
            || (!self.bound && self.binding.is_none() && !self.stopped)
            || self.processes.needs_drive()
            || self.sessions.values().any(|s| {
                s.deadline.is_some() || s.pulls.pending() || !s.outgoing.is_empty() || s.eof
            });
        if !maintenance && let Some(timer) = self.timer.take() {
            host::timer_cancel(timer);
        }
        if self.timer.is_none() && maintenance {
            match host::timer_start(1000) {
                Ok(timer) => self.timer = Some(timer),
                Err(e) => error(e.message),
            }
        }
        handled
    }
    fn request_waiting(&mut self) {
        for id in self.waiting.keys().copied().collect::<Vec<_>>() {
            let (_, descriptor) = &self.waiting[&id];
            let found = self.sessions.iter().find(|(_, session)| {
                session.ready
                    && !session.stopping
                    && session
                        .documents
                        .get(&descriptor.stamp.document)
                        .is_some_and(|d| d.descriptor.stamp == descriptor.stamp && d.sent)
            });
            let Some((&process, _)) = found else { continue };
            let (request, descriptor) = self.waiting.remove(&id).unwrap();
            let session = self.sessions.get_mut(&process).unwrap();
            let doc = &session.documents[&descriptor.stamp.document];
            let Ok(position) = doc.text.position(request.cursor.unwrap_or(0)) else {
                empty(&request);
                continue;
            };
            let mut point = json!({"textDocument":{"uri":doc.uri},"position":position});
            let (method, params) = match request.kind {
                ProviderKind::Hover => ("textDocument/hover", point),
                ProviderKind::CodeActions => match actions::params(&request, doc) {
                    Ok(params) => ("textDocument/codeAction", params),
                    Err(reason) => {
                        self.finish(request.request, descriptor.stamp, Some(reason));
                        continue;
                    }
                },
                ProviderKind::CodeActionResolve => {
                    let raw = match actions::original(
                        &request,
                        &completion::key(session.scope, process, descriptor.stamp),
                    ) {
                        Ok(raw) => raw,
                        Err(reason) => {
                            self.finish(request.request, descriptor.stamp, Some(reason));
                            continue;
                        }
                    };
                    if raw.get("edit").is_some_and(|v| !v.is_null())
                        || raw.get("command").is_some_and(|v| !v.is_null())
                        || session.capabilities["codeActionProvider"]["resolveProvider"] != true
                    {
                        self.resolve_action(process, request.request, descriptor.stamp, raw);
                        continue;
                    }
                    ("codeAction/resolve", raw)
                }
                ProviderKind::ExecuteCommand => match actions::command(
                    &request,
                    &actions::command_key(session.scope, process, descriptor.stamp),
                    &session.capabilities,
                ) {
                    Ok(params) => ("workspace/executeCommand", params),
                    Err(reason) => {
                        self.finish(request.request, descriptor.stamp, Some(reason));
                        continue;
                    }
                },
                ProviderKind::PrepareRename
                    if session.capabilities["renameProvider"]["prepareProvider"] == true =>
                {
                    ("textDocument/prepareRename", point)
                }
                ProviderKind::PrepareRename => {
                    let available = !session.capabilities["renameProvider"].is_null()
                        && session.capabilities["renameProvider"] != false;
                    let answer = LanguageAnswer::RenameTarget(available.then_some(RenameTarget {
                        range: None,
                        placeholder: None,
                        default_behavior: true,
                    }));
                    if let Err(reason) = host::provide_language(request.request, &answer) {
                        error(reason.message);
                    }
                    continue;
                }
                ProviderKind::Rename => {
                    let name = request.context.as_ref().and_then(|c| c.name.as_ref());
                    if session.capabilities["renameProvider"].is_null()
                        || session.capabilities["renameProvider"] == false
                        || name.is_none_or(|name| name.is_empty() || name.len() > 16 * 1024)
                    {
                        self.publish_edit(request.request, descriptor.stamp, vec![]);
                        continue;
                    }
                    point["newName"] = json!(name.unwrap());
                    ("textDocument/rename", point)
                }
                ProviderKind::Formatting | ProviderKind::FormatOnType => {
                    match formatting::request(&request, &doc.text, &doc.uri, &session.capabilities)
                    {
                        Ok(Some(value)) => value,
                        result => {
                            if let Err(reason) = result {
                                error(reason);
                            }
                            empty(&request);
                            continue;
                        }
                    }
                }
                ProviderKind::Definition => {
                    let (method, capability) = match request.context.as_ref().and_then(|c| c.mode) {
                        Some(ProviderMode::Declaration) => {
                            ("textDocument/declaration", "declarationProvider")
                        }
                        Some(ProviderMode::TypeDefinition) => {
                            ("textDocument/typeDefinition", "typeDefinitionProvider")
                        }
                        Some(ProviderMode::Implementation) => {
                            ("textDocument/implementation", "implementationProvider")
                        }
                        _ => ("textDocument/definition", "definitionProvider"),
                    };
                    if session.capabilities[capability].is_null()
                        || session.capabilities[capability] == false
                    {
                        empty(&request);
                        continue;
                    }
                    (method, point)
                }
                ProviderKind::References
                    if !session.capabilities["referencesProvider"].is_null()
                        && session.capabilities["referencesProvider"] != false =>
                {
                    point["context"] = json!({"includeDeclaration":request.context.as_ref().is_some_and(|c| c.include_declaration)});
                    ("textDocument/references", point)
                }
                ProviderKind::Completion
                    if !session.capabilities["completionProvider"].is_null()
                        && session.capabilities["completionProvider"] != false =>
                {
                    let trigger = request.context.as_ref().map_or(0, |c| c.trigger_kind);
                    point["context"] = json!({"triggerKind":if trigger == 0 {
                        if request.trigger.is_some() { 2 } else { 1 }
                    } else { trigger }});
                    if let Some(trigger) = &request.trigger {
                        point["context"]["triggerCharacter"] = json!(trigger);
                    }
                    ("textDocument/completion", point)
                }
                ProviderKind::CompletionResolve
                    if session.capabilities["completionProvider"]["resolveProvider"] == true =>
                {
                    match completion::original(
                        &request,
                        &completion::key(session.scope, process, descriptor.stamp),
                    ) {
                        Ok(raw) => ("completionItem/resolve", raw),
                        Err(_) => {
                            empty(&request);
                            continue;
                        }
                    }
                }
                _ => {
                    empty(&request);
                    continue;
                }
            };
            session.next_request += 1;
            let rpc = session.next_request;
            let message = json!({"jsonrpc":"2.0","id":rpc,"method":method,"params":params});
            session.pending.insert(rpc, (request, descriptor.stamp));
            if let Err(e) = session.send(message) {
                self.kill(process, Some(e));
            }
        }
    }
    fn flush(&mut self, process: Process) -> Result<(), String> {
        let session = self.sessions.get_mut(&process).unwrap();
        while let Some(frame) = session.outgoing.front() {
            let count = self
                .processes
                .write(process, &frame[session.offset..], &mut submit)?;
            session.offset += count;
            if session.offset != frame.len() {
                break;
            }
            session.outgoing.pop_front();
            session.offset = 0;
        }
        if session.eof && session.outgoing.is_empty() {
            self.processes.close_input(process, &mut submit)?;
            session.eof = false;
        }
        Ok(())
    }
    fn location_results(&mut self, results: Vec<LocationResult>) {
        for result in results {
            let edit = self.edits.remove(&result.request);
            if !self.documents.current(result.stamp) {
                continue;
            }
            if let Some(edit) = edit {
                let documents = result
                    .locations
                    .and_then(|locations| rename::resolved(edit.documents, locations));
                if let Some(mut action) = edit.action {
                    match documents {
                        Ok(documents) => {
                            action.documents = Some(documents);
                            self.publish_actions(result.request, result.stamp, vec![action]);
                        }
                        Err(reason) => self.finish(result.request, result.stamp, Some(reason)),
                    }
                    continue;
                }
                let documents = documents.unwrap_or_else(|reason| {
                    error(reason);
                    vec![]
                });
                self.publish_edit(result.request, result.stamp, documents);
                continue;
            }
            let locations = result.locations.unwrap_or_else(|reason| {
                error(reason);
                vec![]
            });
            if self.publications.len() >= 128 {
                error("Resource result mailbox budget exceeded");
                continue;
            }
            match submit(Command::ProvideLocations {
                request: result.request,
                stamp: result.stamp,
                locations,
            }) {
                Ok(ticket) => {
                    self.publications.insert(ticket);
                }
                Err(reason) => error(reason),
            }
        }
    }
    fn publication(&mut self, command: Command) {
        if self.publications.len() >= 128 {
            error("Resource result mailbox budget exceeded");
            return;
        }
        match submit(command) {
            Ok(ticket) => {
                self.publications.insert(ticket);
            }
            Err(reason) => error(reason),
        }
    }
    fn publish_actions(
        &mut self,
        request: u64,
        stamp: api::session::DocumentVersion,
        actions: Vec<api::session::ResourceCodeAction>,
    ) {
        self.publication(Command::ProvideCodeActions {
            request,
            stamp,
            actions,
        });
    }
    fn finish(
        &mut self,
        request: u64,
        stamp: api::session::DocumentVersion,
        reason: Option<String>,
    ) {
        self.publication(Command::FinishLanguageRequest {
            request,
            stamp,
            error: reason.map(|s| {
                let message: String = s.chars().filter(|c| *c != '\0').take(250).collect();
                if message.is_empty() {
                    "Language provider request failed".into()
                } else {
                    message
                }
            }),
        });
    }
    fn resolve_action(
        &mut self,
        process: Process,
        request: u64,
        stamp: api::session::DocumentVersion,
        raw: Value,
    ) {
        let result = (|| {
            let session = self.sessions.get(&process).ok_or("Action server stopped")?;
            let doc = session
                .documents
                .get(&stamp.document)
                .filter(|doc| doc.descriptor.stamp == stamp && self.documents.current(stamp))
                .ok_or("Action source document changed")?;
            let action = actions::resolved(
                &raw,
                &actions::command_key(session.scope, process, stamp),
                &session.capabilities,
            )?;
            if action.disabled.is_some() || raw.get("edit").is_none_or(Value::is_null) {
                return Ok(Some(action));
            }
            if self.edits.len() >= 4 {
                return Err("Too many proposals awaiting URI mapping".into());
            }
            let documents = rename::parse(&raw["edit"], session.unit, |uri, version| {
                let open = session.documents.values().find(|d| d.uri == uri && d.sent);
                match (open, version) {
                    (Some(doc), version)
                        if version.is_none_or(|v| u64::try_from(v).ok() == Some(doc.version))
                            && self.documents.current(doc.descriptor.stamp) =>
                    {
                        Ok(Some(doc.descriptor.stamp))
                    }
                    (None, None) => Ok(None),
                    _ => Err("An edited server document version is no longer current".into()),
                }
            })?;
            let environment = doc.descriptor.resource.environment().unwrap().clone();
            self.locations.start_with_scopes(
                request,
                stamp,
                self.documents.read_scopes(&environment, session.scope),
                environment,
                rename::uris(&documents),
            )?;
            self.edits.insert(
                request,
                PendingEdit {
                    process,
                    documents,
                    action: Some(action),
                },
            );
            Ok::<_, String>(None)
        })();
        match result {
            Ok(Some(action)) => self.publish_actions(request, stamp, vec![action]),
            Ok(None) => {}
            Err(reason) => self.finish(request, stamp, Some(reason)),
        }
    }
    fn publish_edit(
        &mut self,
        request: u64,
        stamp: api::session::DocumentVersion,
        documents: Vec<api::session::ResourceDocumentEdit>,
    ) {
        if self.publications.len() >= 128 {
            error("Resource result mailbox budget exceeded");
            return;
        }
        match submit(Command::ProvideWorkspaceEdit {
            request,
            stamp,
            documents,
        }) {
            Ok(ticket) => {
                self.publications.insert(ticket);
            }
            Err(reason) => error(reason),
        }
    }
    fn edit_responses(&mut self, responses: Vec<server_edits::Response>) {
        for response in responses {
            if let Some(session) = self.sessions.get_mut(&response.process) {
                if let Err(reason) =
                    session.send(json!({"jsonrpc":"2.0","id":response.id,"result":response.result}))
                {
                    self.kill(response.process, Some(reason));
                }
            }
        }
    }
    fn kill(&mut self, process: Process, reason: Option<String>) {
        self.server_edits.cancel_process(process);
        self.edits.retain(|request, edit| {
            if edit.process == process {
                self.locations.cancel(*request);
                false
            } else {
                true
            }
        });
        if let Some(session) = self.sessions.remove(&process) {
            if let Some(reason) = &reason {
                if self.failed.len() >= 128 {
                    self.failed.pop_first();
                }
                if self.failures.len() >= 4 {
                    self.failures.pop_first();
                }
                self.failures.insert(
                    process,
                    api::session::ResourceServiceReport {
                        key: format!("environment-{}", process.0),
                        state: api::session::ServiceState::Error,
                        detail: Some(status::detail(reason)),
                        contexts: Some(
                            session
                                .documents
                                .values()
                                .map(|doc| doc.descriptor.resource.clone())
                                .collect(),
                        ),
                    },
                );
                self.failed.insert((session.scope, session.project));
            }
            for doc in session.documents.into_values() {
                self.clear(&doc);
            }
            for (request, _) in session.pending.into_values() {
                empty(&request);
            }
            if let Err(e) = self.processes.kill(process, &mut submit) {
                error(e);
            }
            if let Some(reason) = reason {
                error(reason);
            }
        }
    }
    fn report_status(&mut self) {
        use api::session::{ResourceServiceReport, ServiceState};
        self.failures.retain(|_, report| {
            let contexts = report.contexts.as_mut().unwrap();
            contexts.retain(|resource| self.documents.is_open(resource));
            !contexts.is_empty()
        });
        let mut reports: Vec<_> = self.failures.values().cloned().collect();
        reports.extend(self.sessions.iter().map(|(process, session)| {
            let (state, detail) = if session.stopping {
                (ServiceState::Stopped, "Stopping")
            } else if !session.ready {
                (
                    ServiceState::Starting,
                    if session.managed {
                        "Preparing tools and starting"
                    } else {
                        "Starting"
                    },
                )
            } else if !session.pending.is_empty()
                || session.pulls.pending()
                || self.server_edits.pending(*process)
            {
                (ServiceState::Working, "Working")
            } else {
                (ServiceState::Ready, "Ready")
            };
            ResourceServiceReport {
                key: format!("environment-{}", process.0),
                state,
                detail: Some(detail.into()),
                contexts: Some(
                    session
                        .documents
                        .values()
                        .map(|doc| doc.descriptor.resource.clone())
                        .collect(),
                ),
            }
        }));
        self.reports.update(reports);
    }
    fn clear(&mut self, doc: &Document) {
        if self.documents.current(doc.descriptor.stamp) {
            self.publish(doc.descriptor.stamp, vec![]);
        }
    }
    fn publish(
        &mut self,
        stamp: api::session::DocumentVersion,
        diagnostics: Vec<ResourceDiagnostic>,
    ) {
        if self.publications.len() >= 128 {
            error("Diagnostic publication budget exceeded");
            return;
        }
        match submit(Command::PublishDiagnostics {
            provider: format!("{ID}.diagnostics"),
            stamp,
            diagnostics,
        }) {
            Ok(ticket) => {
                self.publications.insert(ticket);
            }
            Err(e) => error(e),
        }
    }
    fn document_events(&mut self, events: Vec<DocumentEvent>) {
        for event in events {
            match event {
                DocumentEvent::Error(e) => error(e),
                DocumentEvent::Invalidated(scope) => {
                    self.restarting.retain(|_, doc| doc.root.scope != scope);
                    let affected = self
                        .sessions
                        .iter()
                        .filter(|(_, session)| session.scope == scope)
                        .map(|(process, _)| *process)
                        .collect::<Vec<_>>();
                    for process in affected {
                        self.kill(process, None);
                    }
                    self.failed.retain(|(owner, _)| *owner != scope);
                }
                DocumentEvent::Roots(_roots) => {
                    self.restarting.clear();
                    self.locations.cancel_all();
                    self.edits.clear();
                    self.server_edits.cancel_all();
                    for process in self.sessions.keys().copied().collect::<Vec<_>>() {
                        // Root refresh rechecks project markers, trust and
                        // configuration before starting a replacement server.
                        self.kill(process, None);
                    }
                    self.failed.clear();
                    self.failures.clear();
                }
                DocumentEvent::Closed(id) => {
                    self.restarting.remove(&id);
                    for session in self.sessions.values_mut() {
                        session.pulls.close(id);
                        if let Some(doc) = session.documents.remove(&id) {
                            if doc.sent
                                && let Err(e) = session.notify(
                                    "textDocument/didClose",
                                    json!({"textDocument":{"uri":doc.uri}}),
                                )
                            {
                                error(e);
                            }
                            if session.documents.is_empty()
                                && let Err(e) = session.shutdown()
                            {
                                error(e);
                            }
                        }
                    }
                }
                DocumentEvent::Ready(doc) => {
                    if let Err(e) = self.open(doc) {
                        error(e);
                    }
                }
            }
        }
    }
    fn open(&mut self, doc: ReadyDocument) -> Result<(), String> {
        if self.stopped || self.failed.contains(&(doc.root.scope, doc.project.clone())) {
            return Ok(());
        }
        let id = doc.descriptor.stamp.document;
        let retained: usize = self
            .sessions
            .values()
            .flat_map(|s| s.documents.iter())
            .filter(|(key, _)| **key != id)
            .map(|(_, d)| d.text.value.len())
            .sum::<usize>()
            + self
                .restarting
                .iter()
                .filter(|(key, _)| **key != id)
                .map(|(_, doc)| doc.text.len())
                .sum::<usize>();
        if retained + doc.text.len() > TOTAL_TEXT {
            return Err("Language document text budget exceeded".into());
        }
        let process = if let Some((process, _)) = self
            .sessions
            .iter()
            .find(|(_, s)| s.scope == doc.root.scope && s.project == doc.project && !s.stopping)
        {
            *process
        } else {
            if self.processes.waiting_for_capacity() {
                if self.restarting.len() >= 128 && !self.restarting.contains_key(&id) {
                    return Err("Language restart document budget exceeded".into());
                }
                self.restarting.insert(id, doc);
                return Ok(());
            }
            let api::ResourceRef::Provided { .. } = &doc.root.resource else {
                return Err("Expected a provided root".into());
            };
            // The setting names a program in the target environment. Never
            // expand the local package's ${extension} path for this transport.
            let (program, args) = program::command(&doc.settings)?;
            let managed = program == "@bundled";
            let (program, args) = if managed {
                tools::command(&doc.settings, args)?
            } else {
                (program, args)
            };
            let process = self.processes.start(
                doc.root.scope,
                api::Operation::Spawn {
                    program,
                    args,
                    cwd: doc.project.clone(),
                    env: vec![],
                },
                &mut submit,
            )?;
            let mut session = Session {
                settings: doc.settings,
                scope: doc.root.scope,
                project: doc.project,
                root_uri: doc.root_uri,
                frames: Frames::new(MESSAGE_LIMIT)?,
                outgoing: VecDeque::new(),
                offset: 0,
                documents: BTreeMap::new(),
                ready: false,
                unit: ColumnUnit::Utf16,
                version: 0,
                deadline: Some(now() + if managed { 300_000 } else { 30_000 }),
                stopping: false,
                eof: false,
                next_request: 2,
                pending: BTreeMap::new(),
                completed: vec![],
                capabilities: Value::Null,
                pulls: diagnostics::Pulls::default(),
                managed,
                tool_error: vec![],
            };
            if let Err(error) = session.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "processId":null,"rootUri":session.root_uri,"workspaceFolders":[{"uri":session.root_uri,"name":doc.root.name}],
                "capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]},
                    "textDocument":{"publishDiagnostics":{"versionSupport":true},"diagnostic":{"dynamicRegistration":false},"rename":{"prepareSupport":true},
                        "codeAction":{"dataSupport":true,"isPreferredSupport":true,"disabledSupport":true,
                            "codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix","refactor","refactor.extract","refactor.inline","refactor.rewrite","source","source.organizeImports","source.fixAll"]}},
                            "resolveSupport":{"properties":["edit","command"]}},
                        "completion":{"completionItem":{"snippetSupport":true,"insertReplaceSupport":true,
                            "documentationFormat":["markdown","plaintext"],
                            "resolveSupport":{"properties":["documentation","detail","additionalTextEdits"]}},
                            "completionList":{"itemDefaults":["editRange","insertTextFormat","insertTextMode","data","commitCharacters"]}}},
                    "workspace":{"configuration":true,"diagnostics":{"refreshSupport":true},"applyEdit":true,"workspaceEdit":{"documentChanges":true}}},
                "initializationOptions":setting(&session.settings, "initialization")}})) {
                let _ = self.processes.kill(process, &mut submit);
                return Err(error);
            }
            self.sessions.insert(process, session);
            process
        };
        let session = self.sessions.get_mut(&process).unwrap();
        session.version += 1;
        let sent = session.documents.get(&id).is_some_and(|d| d.sent);
        session.documents.insert(
            id,
            Document {
                descriptor: doc.descriptor,
                uri: doc.uri,
                text: Text::new(doc.text, session.unit),
                version: session.version,
                sent,
                diagnostics: vec![],
            },
        );
        let result = session.sync(id);
        if let Err(e) = result {
            self.kill(process, Some(e));
        }
        Ok(())
    }
    fn process_events(&mut self, events: Vec<ProcessEvent>) {
        for event in events {
            match event {
                ProcessEvent::Output {
                    process,
                    stderr: false,
                    bytes,
                } => {
                    let Some(session) = self.sessions.get_mut(&process) else {
                        continue;
                    };
                    let messages = match session.frames.push(&bytes) {
                        Ok(messages) => messages,
                        Err(e) => {
                            self.kill(process, Some(e));
                            continue;
                        }
                    };
                    for message in messages {
                        let result = serde_json::from_str(&message)
                            .map_err(|e| e.to_string())
                            .and_then(|value| self.rpc(process, value));
                        if let Err(e) = result {
                            self.kill(process, Some(e));
                            break;
                        }
                    }
                }
                // Stderr remains plugin-private, not an unbounded host log.
                ProcessEvent::Output {
                    process,
                    stderr: true,
                    bytes,
                } => {
                    if let Some(session) = self
                        .sessions
                        .get_mut(&process)
                        .filter(|s| s.managed && !s.ready)
                    {
                        session.tool_error.extend(
                            bytes
                                .into_iter()
                                .take(1024usize.saturating_sub(session.tool_error.len())),
                        );
                    }
                }
                ProcessEvent::Writable { .. } => {}
                ProcessEvent::Exited {
                    process,
                    code,
                    error: reason,
                } => {
                    let expected = self.sessions.get(&process).is_some_and(|s| s.stopping)
                        && code == Some(0)
                        && reason.is_none();
                    let tool_error = self.sessions.get(&process).and_then(|session| {
                        let text = String::from_utf8_lossy(&session.tool_error);
                        text.lines().find_map(|line| {
                            line.strip_prefix("loog-tool-error: ").map(str::to_owned)
                        })
                    });
                    self.kill(
                        process,
                        (!expected).then(|| {
                            reason
                                .or(tool_error)
                                .unwrap_or_else(|| format!("Language process exited ({code:?})"))
                        }),
                    );
                }
            }
        }
    }
    fn rpc(&mut self, process: Process, value: Value) -> Result<(), String> {
        let Some(session) = self.sessions.get_mut(&process) else {
            return Ok(());
        };
        if let Some(method) = value["method"].as_str() {
            if value.get("id").is_some() {
                if method == "workspace/applyEdit" {
                    let result = (|| {
                        let origin = session
                            .documents
                            .values()
                            .find(|doc| doc.sent && self.documents.current(doc.descriptor.stamp))
                            .ok_or("No current resource document can anchor the server edit")?;
                        let environment = origin.descriptor.resource.environment().unwrap().clone();
                        let proposal = value["params"]
                            .get("edit")
                            .filter(|v| v.is_object())
                            .ok_or("Missing workspace edit object")?;
                        let documents = rename::parse(proposal, session.unit, |uri, version| {
                            let open = session.documents.values().find(|d| d.uri == uri && d.sent);
                            match (open, version) {
                                (Some(doc), version)
                                    if version.is_none_or(|v| {
                                        u64::try_from(v).ok() == Some(doc.version)
                                    }) && self.documents.current(doc.descriptor.stamp) =>
                                {
                                    Ok(Some(doc.descriptor.stamp))
                                }
                                (None, None) => Ok(None),
                                _ => {
                                    Err("An edited server document version is no longer current"
                                        .into())
                                }
                            }
                        })?;
                        let title = value["params"]
                            .get("label")
                            .map_or(Ok("Language edit"), |v| {
                                v.as_str().ok_or("Invalid edit label")
                            })?;
                        self.server_edits.start(
                            process,
                            value["id"].clone(),
                            origin.descriptor.stamp,
                            title.into(),
                            environment.clone(),
                            self.documents.read_scopes(&environment, session.scope),
                            documents,
                        )
                    })();
                    if let Err(reason) = result {
                        session.send(json!({"jsonrpc":"2.0","id":value["id"],"result":{"applied":false,"failureReason":reason}}))?;
                    }
                    return Ok(());
                }
                let result = match method {
                    "workspace/diagnostic/refresh" => {
                        if session.capabilities["diagnosticProvider"].is_object() {
                            for doc in session.documents.values().filter(|doc| doc.sent) {
                                session.pulls.queue(doc.descriptor.stamp);
                            }
                            session.pull_diagnostics()?;
                        }
                        Some(Value::Null)
                    }
                    "workspace/configuration" => {
                        let items = array(&value["params"]["items"]);
                        if items.len() > 128 {
                            return Err("Configuration request budget exceeded".into());
                        }
                        let mut bytes = 0;
                        let mut results = vec![];
                        for item in items {
                            let mut config = setting(&session.settings, "configuration");
                            if let Some(section) = item["section"].as_str() {
                                for part in section.split('.') {
                                    config = config[part].clone();
                                }
                            }
                            bytes += config.to_string().len();
                            if bytes > 512 * 1024 {
                                return Err("Configuration response budget exceeded".into());
                            }
                            results.push(config);
                        }
                        Some(Value::Array(results))
                    }
                    "workspace/workspaceFolders" => {
                        Some(json!([{"uri":session.root_uri,"name":"workspace"}]))
                    }
                    "window/workDoneProgress/create" => Some(Value::Null),
                    _ => None,
                };
                return session.send(match result {
                    Some(result) => json!({"jsonrpc":"2.0","id":value["id"],"result":result}),
                    None => json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Unsupported resource client operation"}}),
                });
            }
            if method == "textDocument/publishDiagnostics" {
                if session.capabilities["diagnosticProvider"].is_object() {
                    return Ok(());
                }
                let params = &value["params"];
                let Some(doc) = session.documents.values_mut().find(|d| {
                    d.sent
                        && params["uri"] == d.uri
                        && params["version"].as_u64() == Some(d.version)
                        && self.documents.current(d.descriptor.stamp)
                }) else {
                    return Ok(());
                };
                let stamp = doc.descriptor.stamp;
                let diagnostics = diagnostics::findings(doc, &params["diagnostics"]);
                self.publish(stamp, diagnostics);
            }
        } else if value["id"] == 1 {
            if !value["error"].is_null() {
                return Err(format!(
                    "Language initialization failed: {}",
                    value["error"]
                ));
            }
            session.unit = match value["result"]["capabilities"]["positionEncoding"].as_str() {
                Some("utf-8") => ColumnUnit::Utf8,
                None | Some("utf-16") => ColumnUnit::Utf16,
                _ => return Err("Unsupported server position encoding".into()),
            };
            session.ready = true;
            session.capabilities = value["result"]["capabilities"].clone();
            session.notify("initialized", json!({}))?;
            if session.stopping {
                return session
                    .send(json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
            }
            session.deadline = None;
            session.notify(
                "workspace/didChangeConfiguration",
                json!({"settings":setting(&session.settings, "configuration")}),
            )?;
            for id in session.documents.keys().copied().collect::<Vec<_>>() {
                session.sync(id)?;
            }
        } else if value["id"] == 2 && session.stopping {
            session.notify("exit", Value::Null)?;
            session.eof = true;
        } else if let Some(id) = value["id"].as_u64() {
            if let Some(stamp) = session.pulls.finish(id) {
                let found = session
                    .documents
                    .get_mut(&stamp.document)
                    .filter(|doc| {
                        doc.descriptor.stamp == stamp
                            && self.documents.current(stamp)
                            && value["error"].is_null()
                            && value["result"]["kind"] == "full"
                            && value["result"]["items"].is_array()
                    })
                    .map(|doc| diagnostics::findings(doc, &value["result"]["items"]));
                session.pull_diagnostics()?;
                if let Some(findings) = found {
                    self.publish(stamp, findings);
                }
                return Ok(());
            }
            if let Some((request, stamp)) = session.pending.remove(&id) {
                if request.kind == ProviderKind::ExecuteCommand {
                    let reason = (!value["error"].is_null()).then(|| {
                        value["error"]["message"]
                            .as_str()
                            .unwrap_or("Language server command failed")
                            .to_owned()
                    });
                    session.completed.push((request, stamp, reason));
                    return Ok(());
                }
                if matches!(
                    request.kind,
                    ProviderKind::CodeActions | ProviderKind::CodeActionResolve
                ) {
                    let result = session
                        .documents
                        .get(&stamp.document)
                        .filter(|doc| {
                            doc.descriptor.stamp == stamp && self.documents.current(stamp)
                        })
                        .ok_or_else(|| "Code action source document changed".to_owned())
                        .and_then(|_| {
                            if value["error"].is_null() {
                                Ok(())
                            } else {
                                Err(value["error"]["message"]
                                    .as_str()
                                    .unwrap_or("Code action request failed")
                                    .to_owned())
                            }
                        });
                    if let Err(reason) = result {
                        self.finish(request.request, stamp, Some(reason));
                        return Ok(());
                    }
                    if request.kind == ProviderKind::CodeActionResolve {
                        self.resolve_action(
                            process,
                            request.request,
                            stamp,
                            value["result"].clone(),
                        );
                    } else {
                        let result = actions::offers(
                            &value["result"],
                            &completion::key(session.scope, process, stamp),
                        );
                        match result {
                            Ok(actions) => self.publish_actions(request.request, stamp, actions),
                            Err(reason) => self.finish(request.request, stamp, Some(reason)),
                        }
                    }
                    return Ok(());
                }
                if request.kind == ProviderKind::PrepareRename {
                    let answer = session
                        .documents
                        .get(&stamp.document)
                        .filter(|doc| {
                            doc.descriptor.stamp == stamp
                                && self.documents.current(stamp)
                                && value["error"].is_null()
                        })
                        .ok_or_else(|| {
                            "Rename preparation failed or its document changed".to_owned()
                        })
                        .and_then(|doc| rename::prepare(&value["result"], &doc.text))
                        .unwrap_or_else(|reason| {
                            error(reason);
                            None
                        });
                    host::provide_language(request.request, &LanguageAnswer::RenameTarget(answer))
                        .map_err(|e| e.message)?;
                    return Ok(());
                }
                if request.kind == ProviderKind::Rename {
                    let result = (|| {
                        let doc = session
                            .documents
                            .get(&stamp.document)
                            .filter(|doc| {
                                doc.descriptor.stamp == stamp
                                    && self.documents.current(stamp)
                                    && value["error"].is_null()
                            })
                            .ok_or("Rename failed or its source document changed")?;
                        if self.edits.len() >= 4 {
                            return Err("Too many rename proposals awaiting URI mapping".into());
                        }
                        let documents =
                            rename::parse(&value["result"], session.unit, |uri, version| {
                                let open =
                                    session.documents.values().find(|d| d.uri == uri && d.sent);
                                match (open, version) {
                                    (Some(doc), version)
                                        if version.is_none_or(|v| {
                                            u64::try_from(v).ok() == Some(doc.version)
                                        }) && self.documents.current(doc.descriptor.stamp) =>
                                    {
                                        Ok(Some(doc.descriptor.stamp))
                                    }
                                    (None, None) => Ok(None),
                                    _ => Err(
                                        "An edited server document version is no longer current"
                                            .into(),
                                    ),
                                }
                            })?;
                        let environment = doc.descriptor.resource.environment().unwrap().clone();
                        self.locations.start_with_scopes(
                            request.request,
                            stamp,
                            self.documents.read_scopes(&environment, session.scope),
                            environment,
                            rename::uris(&documents),
                        )?;
                        self.edits.insert(
                            request.request,
                            PendingEdit {
                                process,
                                documents,
                                action: None,
                            },
                        );
                        Ok::<_, String>(())
                    })();
                    if let Err(reason) = result {
                        error(reason);
                        self.publish_edit(request.request, stamp, vec![]);
                    }
                    return Ok(());
                }
                if matches!(
                    request.kind,
                    ProviderKind::Formatting | ProviderKind::FormatOnType
                ) {
                    let edits = session
                        .documents
                        .get(&stamp.document)
                        .filter(|doc| {
                            doc.descriptor.stamp == stamp
                                && self.documents.current(stamp)
                                && value["error"].is_null()
                        })
                        .ok_or_else(|| "Formatting failed or its document changed".to_owned())
                        .and_then(|doc| formatting::edits(&value["result"], &doc.text));
                    match edits {
                        Ok(edits) => {
                            host::provide_edits(request.request, &edits).map_err(|e| e.message)?
                        }
                        Err(reason) => {
                            error(reason);
                            empty(&request);
                        }
                    }
                    return Ok(());
                }
                if matches!(
                    request.kind,
                    ProviderKind::Definition | ProviderKind::References
                ) {
                    let result = session
                        .documents
                        .get(&stamp.document)
                        .filter(|doc| {
                            doc.descriptor.stamp == stamp
                                && self.documents.current(stamp)
                                && value["error"].is_null()
                        })
                        .ok_or_else(|| "Location source revision is no longer current".to_owned())
                        .and_then(|doc| {
                            locations::parse(&value["result"], session.unit).and_then(|uris| {
                                let environment =
                                    doc.descriptor.resource.environment().unwrap().clone();
                                self.locations.start_with_scopes(
                                    request.request,
                                    stamp,
                                    self.documents.read_scopes(&environment, session.scope),
                                    environment,
                                    uris,
                                )
                            })
                        });
                    if let Err(reason) = result {
                        error(reason);
                        empty(&request);
                    }
                    return Ok(());
                }
                if matches!(
                    request.kind,
                    ProviderKind::Completion | ProviderKind::CompletionResolve
                ) {
                    let result = session
                        .documents
                        .get(&stamp.document)
                        .filter(|doc| {
                            doc.descriptor.stamp == stamp
                                && self.documents.current(stamp)
                                && value["error"].is_null()
                        })
                        .map(|doc| {
                            completion::answer(
                                &value["result"],
                                &request,
                                &doc.text,
                                &completion::key(session.scope, process, stamp),
                                session.capabilities["completionProvider"]["resolveProvider"]
                                    == true,
                            )
                        });
                    match result {
                        Some(Ok((items, incomplete))) => {
                            if let Err(e) =
                                host::provide_completions(request.request, &items, incomplete)
                            {
                                error(e.message);
                            }
                        }
                        _ => empty(&request),
                    }
                    return Ok(());
                }
                let answer = if value["error"].is_null() && self.documents.current(stamp) {
                    session
                        .documents
                        .get(&stamp.document)
                        .filter(|d| d.descriptor.stamp == stamp)
                        .and_then(|doc| {
                            let value = &value["result"];
                            if value.is_null() {
                                return None;
                            }
                            Some(HoverAnswer {
                                contents: crate::protocol::markdown(&value["contents"])?,
                                range: value
                                    .get("range")
                                    .map(|range| doc.text.range(range))
                                    .transpose()
                                    .ok()?,
                            })
                        })
                } else {
                    None
                };
                host::provide_hover(request.request, answer.as_ref()).map_err(|e| e.message)?;
            }
        }
        Ok(())
    }
}

fn empty(request: &ProviderRequest) {
    let result = match request.kind {
        ProviderKind::PrepareRename => {
            host::provide_language(request.request, &LanguageAnswer::RenameTarget(None))
        }
        ProviderKind::Formatting | ProviderKind::FormatOnType => {
            host::provide_edits(request.request, &[])
        }
        ProviderKind::Completion | ProviderKind::CompletionResolve => {
            host::provide_completions(request.request, &[], false)
        }
        ProviderKind::Definition | ProviderKind::References => {
            host::provide_language(request.request, &LanguageAnswer::Places(vec![]))
        }
        _ => host::provide_hover(request.request, None),
    };
    if let Err(e) = result {
        error(e.message);
    }
}

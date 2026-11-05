use crate::config::{Configuration, language_id};
use crate::{
    ID, NAME, host,
    loom::extension::types::*,
    protocol::*,
    results,
    status::{Status, Widgets},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
#[path = "providers.rs"]
mod providers;
#[path = "rpc.rs"]
mod rpc;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

const MAX_MESSAGE: usize = 32 * 1024 * 1024;
const KINDS: &[&str] = &[
    "diagnostics",
    "completion",
    "hover",
    "definition",
    "document-symbols",
    "signature-help",
    "formatting",
    "inlay-hints",
    "code-actions",
    "references",
    "workspace-symbols",
    "document-highlights",
    "rename",
    "folding-ranges",
    "selection-ranges",
    "call-hierarchy",
    "type-hierarchy",
    "code-lens",
    "format-on-type",
    "execute-command",
    "highlights",
];
fn fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "completion" => &["completionProvider"],
        "hover" => &["hoverProvider"],
        "definition" => &[
            "definitionProvider",
            "declarationProvider",
            "typeDefinitionProvider",
            "implementationProvider",
        ],
        "document-symbols" => &["documentSymbolProvider"],
        "signature-help" => &["signatureHelpProvider"],
        "formatting" => &[
            "documentFormattingProvider",
            "documentRangeFormattingProvider",
        ],
        "inlay-hints" => &["inlayHintProvider"],
        "code-actions" => &["codeActionProvider"],
        "references" => &["referencesProvider"],
        "workspace-symbols" => &["workspaceSymbolProvider"],
        "document-highlights" => &["documentHighlightProvider"],
        "rename" => &["renameProvider"],
        "folding-ranges" => &["foldingRangeProvider"],
        "selection-ranges" => &["selectionRangeProvider"],
        "call-hierarchy" => &["callHierarchyProvider"],
        "type-hierarchy" => &["typeHierarchyProvider"],
        "code-lens" => &["codeLensProvider"],
        "format-on-type" => &["documentOnTypeFormattingProvider"],
        "execute-command" => &["executeCommandProvider"],
        "highlights" => &["semanticTokensProvider"],
        _ => &[],
    }
}
fn kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Diagnostics => "diagnostics",
        ProviderKind::Formatting | ProviderKind::Save => "formatting",
        ProviderKind::Completion | ProviderKind::CompletionResolve => "completion",
        ProviderKind::Hover => "hover",
        ProviderKind::Definition => "definition",
        ProviderKind::DocumentSymbols => "document-symbols",
        ProviderKind::SignatureHelp => "signature-help",
        ProviderKind::InlayHints => "inlay-hints",
        ProviderKind::CodeActions | ProviderKind::CodeActionResolve => "code-actions",
        ProviderKind::References => "references",
        ProviderKind::WorkspaceSymbols => "workspace-symbols",
        ProviderKind::DocumentHighlights => "document-highlights",
        ProviderKind::PrepareRename | ProviderKind::Rename => "rename",
        ProviderKind::FoldingRanges => "folding-ranges",
        ProviderKind::SelectionRanges => "selection-ranges",
        ProviderKind::CallHierarchy => "call-hierarchy",
        ProviderKind::TypeHierarchy => "type-hierarchy",
        ProviderKind::CodeLens | ProviderKind::CodeLensResolve => "code-lens",
        ProviderKind::FormatOnType => "format-on-type",
        ProviderKind::ExecuteCommand => "execute-command",
        ProviderKind::Highlights => "highlights",
    }
}
#[derive(Clone)]
struct Dynamic {
    field: String,
    options: Value,
}
#[derive(Clone)]
struct Watch {
    handle: u64,
    kinds: u32,
}
struct Registration {
    id: Value,
    previous_dynamic: BTreeMap<String, Dynamic>,
    previous_watches: BTreeMap<String, Vec<Watch>>,
    created: BTreeSet<u64>,
    waiting: BTreeSet<u64>,
    changed: BTreeSet<String>,
}
struct Document {
    reference: DocumentRef,
    process: u64,
    text: Text,
    server_version: Option<u64>,
    diagnostics: Vec<Value>,
    signature: Value,
    dirty: bool,
}
struct Call {
    request: ProviderRequest,
    text: Text,
    reference: DocumentRef,
    raw: Value,
    semantic_delta: bool,
}
enum Purpose {
    Initialize,
    Shutdown,
    Provider(Box<Call>),
    Command(Option<Box<ProviderRequest>>),
    Symbols(u64),
}
struct Rpc {
    purpose: Purpose,
    method: String,
    params: Value,
    timer: u64,
    retried: bool,
    started: u64,
}
struct Frame {
    bytes: Vec<u8>,
    request: Option<u64>,
}
struct Session {
    process: u64,
    key: String,
    root: RootRef,
    cwd: String,
    path: String,
    ready: bool,
    stopped: bool,
    attempt: u8,
    capabilities: Value,
    dynamic: BTreeMap<String, Dynamic>,
    watches: BTreeMap<String, Vec<Watch>>,
    registration: Option<Registration>,
    pending: BTreeMap<u64, Rpc>,
    outbox: VecDeque<Frame>,
    waiting: VecDeque<ProviderRequest>,
    waiting_symbols: Vec<(u64, String)>,
    next: u64,
    version: u64,
    unit: ColumnUnit,
    semantic: BTreeMap<String, (Option<String>, Vec<u32>)>,
    progress: Vec<(String, Value)>,
    server_error: Option<String>,
    initialization: Value,
    idle: Option<u64>,
    stable: Option<u64>,
    edits: BTreeSet<u64>,
    completed: Vec<Option<ProviderRequest>>,
}
enum Timer {
    Rpc(u64, u64),
    Restart {
        previous_process: u64,
        key: String,
        root: RootRef,
        cwd: String,
        path: String,
        attempt: u8,
    },
    Idle(u64),
    Stable(u64),
}
struct Operation {
    process: u64,
    id: Value,
    show: bool,
}
struct SymbolGroup {
    remaining: usize,
    values: Vec<WorkspaceSymbol>,
}
struct CompletionSet {
    process: u64,
    document: u64,
    lifecycle: u64,
    items: Vec<Value>,
    defaults: Value,
    bytes: usize,
}
#[derive(Default)]
pub struct Client {
    config: Configuration,
    sessions: BTreeMap<u64, Session>,
    owners: BTreeMap<u64, (u64, String)>,
    documents: BTreeMap<u64, Document>,
    timers: BTreeMap<u64, Timer>,
    statuses: BTreeMap<String, Status>,
    revisions: BTreeMap<String, u64>,
    operations: BTreeMap<u64, Operation>,
    symbols: BTreeMap<u64, SymbolGroup>,
    measurements: BTreeMap<String, Vec<f64>>,
    widget: Widgets,
    shutting_down: bool,
    configuration_dirty: bool,
    completion_sets: BTreeMap<u64, CompletionSet>,
    next_completion: u64,
    captures: BTreeMap<(u64, u64, u64), Rc<str>>,
}
impl Session {
    fn effective(&self, document: Option<&DocumentRef>) -> Value {
        let mut result = self.capabilities.clone();
        if !result.is_object() {
            result = json!({});
        }
        for dynamic in self.dynamic.values() {
            if document.is_some_and(|doc| !selector(&dynamic.options["documentSelector"], doc)) {
                continue;
            }
            result[&dynamic.field] = merge_options(&result[&dynamic.field], &dynamic.options);
        }
        result
    }
    fn flush(&mut self) -> Result<()> {
        while let Some(frame) = self.outbox.front() {
            let count = host::process_write(self.process, &frame.bytes).map_err(|e| e.message)?;
            if count == 0 {
                break;
            }
            if count as usize != frame.bytes.len() {
                return Err("partial whole-message admission".into());
            }
            self.outbox.pop_front();
        }
        Ok(())
    }
    fn send(&mut self, value: Value, request: Option<u64>) -> Result<()> {
        if self.stopped {
            return Err("language server connection is closed".into());
        }
        if self.outbox.len() >= 64 {
            return Err("language server write budget exceeded".into());
        }
        let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MESSAGE {
            return Err("language server message budget exceeded".into());
        }
        self.outbox.push_back(Frame { bytes, request });
        self.flush()
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(
            json!({"jsonrpc":"2.0","method":method,"params":params}),
            None,
        )
    }
    fn answer(&mut self, id: Value, result: std::result::Result<Value, String>) -> Result<()> {
        if id.is_null() {
            return Ok(());
        }
        self.send(
            match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err(error) => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error}})
                }
            },
            None,
        )
    }
}
fn merge_options(left: &Value, right: &Value) -> Value {
    if !yes(left) || left == true {
        return if yes(right) {
            right.clone()
        } else {
            left.clone()
        };
    }
    if !yes(right) || right == true {
        return left.clone();
    }
    let mut result = left.clone();
    if let (Some(a), Some(b)) = (result.as_object_mut(), right.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    for key in [
        "triggerCharacters",
        "retriggerCharacters",
        "moreTriggerCharacter",
        "commands",
    ] {
        if !left[key].is_null() || !right[key].is_null() {
            let mut values = array(&left[key]).to_vec();
            for v in array(&right[key]) {
                if !values.contains(v) {
                    values.push(v.clone());
                }
            }
            result[key] = json!(values);
        }
    }
    for key in ["resolveProvider", "prepareProvider", "range", "full"] {
        if yes(&left[key]) || yes(&right[key]) {
            result[key] = if left[key].is_object() || right[key].is_object() {
                let mut object = serde_json::Map::new();
                for source in [&left[key], &right[key]] {
                    if let Some(source) = source.as_object() {
                        object.extend(source.clone());
                    }
                }
                Value::Object(object)
            } else {
                json!(true)
            };
        }
    }
    result
}
fn selector(value: &Value, doc: &DocumentRef) -> bool {
    if value.is_null() {
        return true;
    }
    array(value).iter().any(|filter| {
        if filter["language"]
            .as_str()
            .is_some_and(|v| v != language_id(&doc.language, doc.variant.as_deref()))
            || filter["scheme"].as_str().is_some_and(|v| v != "file")
        {
            return false;
        }
        let pattern = if filter["pattern"].is_string() {
            &filter["pattern"]
        } else {
            &filter["pattern"]["pattern"]
        };
        if let Some(pattern) = pattern.as_str() {
            if pattern.len() > 1024 {
                return false;
            }
            let Ok(glob) = globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
            else {
                return false;
            };
            let matcher = glob.compile_matcher();
            let path = normalize(&doc.path);
            if !matcher.is_match(&path)
                && !matcher.is_match(path.rsplit('/').next().unwrap_or_default())
            {
                return false;
            }
        }
        true
    })
}
fn context() -> ProviderContext {
    ProviderContext {
        mode: None,
        name: None,
        query: None,
        include_declaration: true,
        tab_size: 4,
        insert_spaces: true,
        action_kinds: Vec::new(),
        trigger_kind: 1,
        retrigger: false,
        active_signature: None,
        active_parameter: None,
        data: None,
    }
}
impl Client {
    fn setting(&self, name: &str, fallback: Value) -> Value {
        let Some(text) = host::get_setting(&format!("{ID}.{name}")) else {
            return fallback;
        };
        let result = serde_json::from_str::<Value>(&text).and_then(|value| {
            if let Some(text) = value.as_str() {
                serde_json::from_str(text)
            } else {
                Ok(value)
            }
        });
        match result {
            Ok(v) => v,
            Err(e) => {
                host::log(1, &format!("Invalid {name} JSON: {e}"));
                fallback
            }
        }
    }
    fn measuring(&self) -> bool {
        self.setting("performance", json!(false)) == true
    }
    fn record(&mut self, name: &str, start: u64) {
        if !self.measuring() {
            return;
        }
        let elapsed = host::monotonic_nanoseconds().saturating_sub(start) as f64 / 1_000_000.;
        let samples = self.measurements.entry(name.into()).or_default();
        samples.push(elapsed);
        if samples.len() == 100 {
            samples.sort_by(f64::total_cmp);
            host::log(
                0,
                &format!(
                    "language-metrics {}",
                    json!({"name":name,"count":100,"p50_ms":samples[49],"p95_ms":samples[94]})
                ),
            );
            samples.clear();
        }
    }
    fn timer(&mut self, milliseconds: u32, value: Timer) -> Result<u64> {
        let id = host::timer_start(milliseconds).map_err(|e| e.message)?;
        self.timers.insert(id, value);
        Ok(id)
    }
    fn cancel_timer(&mut self, id: Option<u64>) {
        if let Some(id) = id {
            host::timer_cancel(id);
            self.timers.remove(&id);
        }
    }
    fn status(&mut self, session: &Session) {
        let status = if session.stopped {
            Status {
                rank: 0,
                detail: String::new(),
            }
        } else if let Some(error) = &session.server_error {
            Status {
                rank: 3,
                detail: error.clone(),
            }
        } else if !session.ready || !session.progress.is_empty() {
            let detail = session
                .progress
                .last()
                .map(|(_, v)| {
                    [
                        optional(&v["title"]),
                        optional(&v["message"]),
                        v["percentage"]
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .map(|v| format!("{}%", v.clamp(0., 100.).round())),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
                })
                .unwrap_or_else(|| "Starting".into());
            Status { rank: 2, detail }
        } else {
            Status {
                rank: 1,
                detail: String::new(),
            }
        };
        self.statuses.insert(session.key.clone(), status);
        self.refresh_status();
    }
    fn refresh_status(&mut self) {
        if !self.shutting_down {
            let mut states = BTreeMap::<String, (Status, Vec<String>)>::new();
            for document in self.documents.values() {
                if let Some((_, key)) = self.owners.get(&document.process) {
                    let status = self.statuses.get(key).cloned().unwrap_or_default();
                    states
                        .entry(key.clone())
                        .or_insert_with(|| (status, Vec::new()))
                        .1
                        .push(document.reference.path.clone());
                }
            }
            self.widget.update(states);
        }
    }
    fn configure(&self, extra: Option<&Session>) {
        let capabilities = self
            .sessions
            .values()
            .chain(extra)
            .filter(|s| !s.stopped)
            .map(|s| s.effective(None))
            .collect::<Vec<_>>();
        for &kind in KINDS {
            let sources = capabilities
                .iter()
                .filter(|v| kind == "diagnostics" || fields(kind).iter().any(|f| yes(&v[*f])))
                .collect::<Vec<_>>();
            let options = sources
                .iter()
                .filter_map(|v| fields(kind).iter().map(|f| &v[*f]).find(|v| yes(v)))
                .collect::<Vec<_>>();
            let strings = |field: &str| {
                options
                    .iter()
                    .flat_map(|v| array(&v[field]))
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            };
            let mut triggers = strings("triggerCharacters");
            for option in &options {
                if let Some(trigger) = option["firstTriggerCharacter"].as_str() {
                    triggers.push(trigger.into());
                    triggers.extend(
                        array(&option["moreTriggerCharacter"])
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_owned)),
                    );
                }
            }
            triggers.sort();
            triggers.dedup();
            let modes = if kind == "definition" {
                Some(
                    [
                        (ProviderMode::Definition, "definitionProvider"),
                        (ProviderMode::Declaration, "declarationProvider"),
                        (ProviderMode::TypeDefinition, "typeDefinitionProvider"),
                        (ProviderMode::Implementation, "implementationProvider"),
                    ]
                    .into_iter()
                    .filter(|(_, field)| sources.iter().any(|s| yes(&s[*field])))
                    .map(|(mode, _)| mode)
                    .collect(),
                )
            } else if kind == "formatting" {
                Some(
                    [
                        (ProviderMode::Full, "documentFormattingProvider"),
                        (ProviderMode::Range, "documentRangeFormattingProvider"),
                    ]
                    .into_iter()
                    .filter(|(_, field)| sources.iter().any(|s| yes(&s[*field])))
                    .map(|(mode, _)| mode)
                    .collect(),
                )
            } else {
                None
            };
            let configuration = ProviderConfiguration {
                provider: format!("{ID}.{kind}"),
                enabled: !sources.is_empty(),
                modes,
                revision: Some(*self.revisions.get(kind).unwrap_or(&0)),
                resolve: options.iter().any(|v| yes(&v["resolveProvider"])),
                prepare: options.iter().any(|v| yes(&v["prepareProvider"])),
                triggers,
                retriggers: strings("retriggerCharacters"),
                commands: strings("commands"),
                aliases: if kind == "formatting" {
                    vec![
                        NAME.into(),
                        ID.into(),
                        ID.rsplit('.').next().unwrap_or(ID).into(),
                    ]
                } else {
                    Vec::new()
                },
            };
            if let Err(e) = host::configure_provider(&configuration) {
                host::log(1, &e.message);
            }
        }
    }
    fn changed(&mut self, changed: &BTreeSet<String>) {
        self.configuration_dirty = true;
        for kind in KINDS {
            if fields(kind).iter().any(|f| changed.contains(*f)) {
                *self.revisions.entry((*kind).into()).or_default() += 1;
            }
        }
    }
    fn rpc(
        &mut self,
        s: &mut Session,
        method: &str,
        params: Value,
        purpose: Purpose,
        milliseconds: u32,
    ) -> Result<()> {
        if s.pending.len() >= 32 {
            return Err("language server request budget exceeded".into());
        }
        s.next += 1;
        let id = s.next;
        let timer = self.timer(milliseconds, Timer::Rpc(s.process, id))?;
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        s.pending.insert(
            id,
            Rpc {
                purpose,
                method: method.into(),
                params,
                timer,
                retried: false,
                started: host::monotonic_nanoseconds(),
            },
        );
        if let Err(error) = s.send(message, Some(id)) {
            s.pending.remove(&id);
            self.cancel_timer(Some(timer));
            return Err(error);
        }
        Ok(())
    }
    fn root(&self, reference: &DocumentRef) -> Result<(RootRef, String, String)> {
        let path = normalize(&reference.path);
        let mut roots = host::list_roots()
            .into_iter()
            .filter(|root| inside(&path, &root.path))
            .collect::<Vec<_>>();
        roots.sort_by_key(|r| std::cmp::Reverse(r.path.len()));
        let root = roots
            .into_iter()
            .next()
            .ok_or("open a trusted workspace folder for this document")?;
        let root_path = normalize(&root.path);
        let relative = path[root_path.len()..].trim_start_matches('/');
        let file_directory = relative.rsplit_once('/').map_or("", |(p, _)| p);
        let mut directory = file_directory;
        let mut marked = None;
        loop {
            if self.config.markers.iter().any(|marker| {
                host::read_file(
                    root.id,
                    &if directory.is_empty() {
                        (*marker).to_owned()
                    } else {
                        format!("{directory}/{marker}")
                    },
                    1,
                )
                .is_ok()
            }) {
                marked = Some(directory.to_owned());
                if !self.config.outermost {
                    break;
                }
            }
            if directory.is_empty() {
                break;
            }
            directory = directory.rsplit_once('/').map_or("", |(p, _)| p);
        }
        let cwd = marked.unwrap_or_else(|| {
            if self.config.file_fallback {
                file_directory.to_owned()
            } else {
                String::new()
            }
        });
        let path = if cwd.is_empty() {
            root_path
        } else {
            format!("{}/{cwd}", root_path.trim_end_matches('/'))
        };
        Ok((root, cwd, path))
    }
    fn start(&mut self, root: RootRef, cwd: String, path: String, attempt: u8) -> Result<u64> {
        if self.sessions.values().filter(|s| !s.stopped).count() >= 4 {
            return Err("at most four language server processes may run per plugin".into());
        }
        let process = host::process_spawn(&ProcessRequest {
            root: Some(root.id),
            program: self.config.program.into(),
            args: self.config.args.iter().map(|s| (*s).to_owned()).collect(),
            cwd: (!cwd.is_empty()).then(|| cwd.clone()),
            env: vec![],
            framing: ProcessFraming::ContentLengthText,
        })
        .map_err(|e| e.message)?;
        let initialization = self.setting("initialization", json!({}));
        let mut s = Session {
            process,
            key: format!("{}:{cwd}", root.id),
            root,
            cwd,
            path,
            ready: false,
            stopped: false,
            attempt,
            capabilities: json!({}),
            dynamic: BTreeMap::new(),
            watches: BTreeMap::new(),
            registration: None,
            pending: BTreeMap::new(),
            outbox: VecDeque::new(),
            waiting: VecDeque::new(),
            waiting_symbols: Vec::new(),
            next: 0,
            version: 0,
            unit: ColumnUnit::Utf16,
            semantic: BTreeMap::new(),
            progress: Vec::new(),
            server_error: None,
            initialization: initialization.clone(),
            idle: None,
            stable: None,
            edits: BTreeSet::new(),
            completed: Vec::new(),
        };
        self.configuration_dirty = true;
        self.owners.insert(process, (s.root.id, s.key.clone()));
        self.status(&s);
        let params = json!({"processId":null,"clientInfo":{"name":"Loom","version":"0.1.8"},"rootUri":uri(&s.path),"workspaceFolders":[{"uri":uri(&s.path),"name":s.root.name}],"capabilities":serde_json::from_str::<Value>(include_str!("../capabilities.json")).unwrap(),"initializationOptions":initialization});
        let result = self.rpc(&mut s, "initialize", params, Purpose::Initialize, 30000);
        if let Err(error) = result {
            self.stop_session(&mut s);
            return Err(error);
        }
        self.sessions.insert(process, s);
        Ok(process)
    }
    fn session(&mut self, reference: &DocumentRef) -> Result<u64> {
        let (root, cwd, path) = self.root(reference)?;
        let key = format!("{}:{cwd}", root.id);
        if let Some(process) = self
            .sessions
            .values()
            .find(|s| s.key == key && !s.stopped)
            .map(|s| s.process)
        {
            let idle = self.sessions.get_mut(&process).unwrap().idle.take();
            self.cancel_timer(idle);
            return Ok(process);
        }
        self.start(root, cwd, path, 0)
    }
    fn with_session<T>(
        &mut self,
        process: u64,
        f: impl FnOnce(&mut Self, &mut Session) -> Result<T>,
    ) -> Result<Option<T>> {
        let Some(mut session) = self.sessions.remove(&process) else {
            return Ok(None);
        };
        let result = f(self, &mut session);
        if !session.stopped {
            self.sessions.insert(process, session);
        }
        if self.configuration_dirty {
            self.configure(None);
            self.configuration_dirty = false;
        }
        result.map(Some)
    }
    fn stop_session(&mut self, s: &mut Session) {
        if s.stopped {
            return;
        }
        s.stopped = true;
        if !self.documents.values().any(|d| d.process == s.process) {
            self.owners.remove(&s.process);
        }
        self.configuration_dirty = true;
        self.completion_sets
            .retain(|_, set| set.process != s.process);
        for request in s.completed.drain(..).flatten() {
            host::release_snapshot(request.snapshot);
        }
        host::process_kill(s.process);
        self.cancel_timer(s.idle.take());
        self.cancel_timer(s.stable.take());
        for call in std::mem::take(&mut s.pending).into_values() {
            self.cancel_timer(Some(call.timer));
            self.discard(call.purpose);
        }
        for request in s.waiting.drain(..) {
            host::release_snapshot(request.snapshot);
        }
        for (owner, _) in s.waiting_symbols.drain(..) {
            self.discard(Purpose::Symbols(owner));
        }
        for watches in s.watches.values() {
            for watch in watches {
                host::unwatch_files(watch.handle);
            }
        }
        s.watches.clear();
        if let Some(registration) = s.registration.take() {
            for watches in registration.previous_watches.into_values() {
                for watch in watches {
                    host::unwatch_files(watch.handle);
                }
            }
            for watch in registration.created {
                host::unwatch_files(watch);
            }
        }
        self.operations
            .retain(|_, operation| operation.process != s.process);
        s.outbox.clear();
        s.progress.clear();
        s.semantic.clear();
    }
    fn discard(&mut self, purpose: Purpose) {
        match purpose {
            Purpose::Provider(call) => host::release_snapshot(call.request.snapshot),
            Purpose::Command(Some(request)) => host::release_snapshot(request.snapshot),
            Purpose::Symbols(request) => {
                if let Some(group) = self.symbols.get_mut(&request) {
                    group.remaining = group.remaining.saturating_sub(1);
                    if group.remaining == 0 {
                        let group = self.symbols.remove(&request).unwrap();
                        let _ = host::provide_language(
                            request,
                            &LanguageAnswer::WorkspaceSymbols(group.values),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    fn failed(&mut self, s: &mut Session, error: String) {
        if s.stopped {
            return;
        }
        self.stop_session(s);
        host::log(2, &format!("{NAME}: {error}"));
        self.statuses.insert(
            s.key.clone(),
            Status {
                rank: 3,
                detail: error,
            },
        );
        self.refresh_status();
        if !self.shutting_down
            && s.attempt < 3
            && self.documents.values().any(|d| d.process == s.process)
        {
            let _ = self.timer(
                1000 << s.attempt,
                Timer::Restart {
                    previous_process: s.process,
                    key: s.key.clone(),
                    root: s.root.clone(),
                    cwd: s.cwd.clone(),
                    path: s.path.clone(),
                    attempt: s.attempt + 1,
                },
            );
        }
    }
    fn protocol_version(&self, url: &str, version: u64) -> Result<u64> {
        let path = path(url)?;
        let doc = self
            .documents
            .values()
            .find(|d| key(&d.reference.path) == key(&path) && d.server_version == Some(version))
            .ok_or("workspace edit refers to an obsolete document version")?;
        Ok(doc.reference.version)
    }
    fn capture(&mut self, document: u64) -> Result<(DocumentRef, Rc<str>)> {
        let captured = host::capture_document(document).map_err(|e| e.message)?;
        let key = (
            document,
            captured.document.lifecycle,
            captured.document.version,
        );
        if let Some(text) = self.captures.get(&key) {
            host::release_snapshot(captured.snapshot);
            return Ok((captured.document, text.clone()));
        }
        let text = host::read_range(
            captured.snapshot,
            ByteRange {
                start: 0,
                end: captured.length,
            },
        )
        .map_err(|e| e.message);
        host::release_snapshot(captured.snapshot);
        let text = Rc::<str>::from(text?);
        self.captures.retain(|(id, _, _), _| *id != document);
        if text.len() <= MAX_MESSAGE {
            while self.captures.values().map(|text| text.len()).sum::<usize>() + text.len()
                > MAX_MESSAGE
            {
                let Some(key) = self.captures.keys().next().copied() else {
                    break;
                };
                self.captures.remove(&key);
            }
            self.captures.insert(key, text.clone());
        }
        Ok((captured.document, text))
    }
    fn synchronize(
        &mut self,
        s: &mut Session,
        id: u64,
        supplied: Option<(DocumentRef, Rc<str>)>,
    ) -> Result<()> {
        if !s.ready {
            if let Some(doc) = self.documents.get_mut(&id) {
                doc.dirty = true;
            }
            return Ok(());
        }
        let (reference, text) = match supplied {
            Some(value) => value,
            None => self.capture(id)?,
        };
        let doc = self.documents.get_mut(&id).ok_or("document closed")?;
        if doc.reference.lifecycle == reference.lifecycle
            && doc.reference.version > reference.version
        {
            return Err("document revision is no longer current".into());
        }
        if doc.server_version.is_some()
            && doc.reference.lifecycle == reference.lifecycle
            && doc.reference.version == reference.version
            && (Rc::ptr_eq(&doc.text.value, &text) || doc.text.value.as_ref() == text.as_ref())
        {
            doc.dirty = false;
            return Ok(());
        }
        let next = Text::new(text.clone(), s.unit);
        s.version += 1;
        if doc.server_version.is_none() {
            s.notify("textDocument/didOpen",json!({"textDocument":{"uri":uri(&reference.path),"languageId":language_id(&reference.language, reference.variant.as_deref()),"version":s.version,"text":text.as_ref()}}))?;
        } else {
            let sync = s.effective(Some(&reference))["textDocumentSync"].clone();
            let incremental = sync.as_u64() == Some(2) || sync["change"] == 2;
            let change = if incremental {
                doc.text.diff(&text)?
            } else {
                json!({"text":text.as_ref()})
            };
            s.notify("textDocument/didChange",json!({"textDocument":{"uri":uri(&reference.path),"version":s.version},"contentChanges":[&change]}))?;
            if !doc.diagnostics.is_empty() {
                // Keep push findings aligned until the server publishes a replacement.
                // A cached diagnostics response must not erase the host's mapped findings.
                let change = if incremental {
                    change
                } else {
                    doc.text.diff(&text)?
                };
                results::rebase_diagnostics(&mut doc.diagnostics, &doc.text, &next, &change)?;
            }
        }
        doc.text = next;
        doc.reference = reference;
        doc.server_version = Some(s.version);
        doc.dirty = false;
        Ok(())
    }
    fn unused(&mut self, process: u64) {
        if self.documents.values().any(|d| d.process == process) {
            return;
        }
        if !self.sessions.contains_key(&process) {
            let timers = self
                .timers
                .iter()
                .filter_map(|(id, t)| {
                    matches!(t,Timer::Restart{previous_process,..} if *previous_process==process)
                        .then_some(*id)
                })
                .collect::<Vec<_>>();
            for id in timers {
                self.cancel_timer(Some(id));
            }
            if let Some((_, key)) = self.owners.remove(&process)
                && !self.sessions.values().any(|s| s.key == key)
            {
                self.statuses.remove(&key);
                self.refresh_status();
            }
            return;
        }
        if let Some(s) = self.sessions.get_mut(&process) {
            let old = s.idle.take();
            self.cancel_timer(old);
            if let Ok(id) = self.timer(60000, Timer::Idle(process))
                && let Some(s) = self.sessions.get_mut(&process)
            {
                s.idle = Some(id);
            }
        }
    }
    fn restart(&mut self, s: &mut Session) -> Result<()> {
        let old = s.process;
        self.stop_session(s);
        let process = self.start(s.root.clone(), s.cwd.clone(), s.path.clone(), 0)?;
        for doc in self.documents.values_mut().filter(|d| d.process == old) {
            doc.process = process;
            doc.server_version = None;
            doc.diagnostics.clear();
            doc.dirty = true;
        }
        self.unused(process);
        self.owners.remove(&old);
        self.refresh_status();
        Ok(())
    }
    pub fn message(&mut self, message: Inbound) -> Result<()> {
        match message {
            Inbound::DocumentOpened(reference) | Inbound::DocumentResync(reference) => {
                if !self.config.languages.contains(&reference.language.as_str()) {
                    return Ok(());
                }
                if self.documents.get(&reference.document).is_some_and(|d| {
                    d.reference.lifecycle != reference.lifecycle
                        || d.reference.path != reference.path
                        || d.reference.language != reference.language
                        || d.reference.variant != reference.variant
                }) {
                    self.close(reference.document)?;
                }
                let id = reference.document;
                if !self.documents.contains_key(&id) {
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
                self.with_session(process, |c, s| c.synchronize(s, id, None))?;
            }
            Inbound::DocumentChanged(change) => {
                if let Some(doc) = self
                    .documents
                    .get(&change.document)
                    .filter(|d| d.reference.lifecycle == change.lifecycle)
                {
                    let process = doc.process;
                    self.with_session(process, |c, s| c.synchronize(s, change.document, None))?;
                }
            }
            Inbound::DocumentClosed(reference) => {
                if self
                    .documents
                    .get(&reference.document)
                    .is_some_and(|d| d.reference.lifecycle == reference.lifecycle)
                {
                    self.close(reference.document)?;
                }
            }
            Inbound::DocumentSaved(saved) => {
                let result: Result<()> = (|| {
                    if let Some(doc) = self.documents.get(&saved.document.document) {
                        let process = doc.process;
                        self.with_session(process, |c, s| {
                            c.synchronize(s, saved.document.document, None)?;
                            let save =
                                s.effective(Some(&saved.document))["textDocumentSync"]["save"]
                                    .clone();
                            if yes(&save) {
                                let mut params =
                                    json!({"textDocument":{"uri":uri(&saved.document.path)}});
                                if save["includeText"] == true {
                                    params["text"] = json!(
                                        host::read_range(
                                            saved.snapshot,
                                            ByteRange {
                                                start: 0,
                                                end: saved.length
                                            }
                                        )
                                        .map_err(|e| e.message)?
                                    );
                                }
                                s.notify("textDocument/didSave", params)?;
                            }
                            Ok(())
                        })?;
                    }
                    Ok(())
                })();
                host::release_snapshot(saved.snapshot);
                result?;
            }
            Inbound::ProviderRequest(request) => self.provide(request)?,
            Inbound::ProviderCommand(call) => self.execute(call.command, None)?,
            Inbound::ProviderCancelled(request) => self.cancel(request),
            Inbound::ProcessOutput(output) => {
                self.with_session(output.process, |c, s| {
                    if output.channel == OutputChannel::Stdout {
                        let value = output
                            .text
                            .unwrap_or_else(|| String::from_utf8_lossy(&output.bytes).into_owned());
                        if let Err(error) = c.input(s, &value) {
                            c.failed(s, error);
                        }
                    } else {
                        host::log(
                            0,
                            &String::from_utf8_lossy(&output.bytes)
                                .chars()
                                .take(4000)
                                .collect::<String>(),
                        );
                    }
                    Ok(())
                })?;
            }
            Inbound::ProcessWritable(output) => {
                self.with_session(output.process, |c, s| {
                    if let Err(error) = s.flush() {
                        c.failed(s, error);
                    }
                    Ok(())
                })?;
            }
            Inbound::ProcessExited(exit) => {
                self.with_session(exit.process, |c, s| {
                    c.failed(
                        s,
                        exit.error
                            .map(|e| e.message)
                            .unwrap_or_else(|| format!("language server exited ({:?})", exit.code)),
                    );
                    Ok(())
                })?;
            }
            Inbound::TimerElapsed(id) => self.elapsed(id)?,
            Inbound::OperationComplete((id, result)) => self.operation(id, result)?,
            Inbound::RootsChanged(roots) => {
                let allowed = roots.iter().map(|r| r.id).collect::<BTreeSet<_>>();
                let processes = self
                    .sessions
                    .values()
                    .filter(|s| !allowed.contains(&s.root.id))
                    .map(|s| s.process)
                    .collect::<Vec<_>>();
                for process in processes {
                    self.with_session(process, |c, s| {
                        c.stop_session(s);
                        c.statuses.remove(&s.key);
                        Ok(())
                    })?;
                }
                let timers = self
                    .timers
                    .iter()
                    .filter_map(|(id, t)| match t {
                        Timer::Restart { root, .. } if !allowed.contains(&root.id) => Some(*id),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for id in timers {
                    self.cancel_timer(Some(id));
                }
                self.documents.retain(|_, d| {
                    self.owners
                        .get(&d.process)
                        .is_some_and(|(root, _)| allowed.contains(root))
                });
                self.owners.retain(|_, (root, _)| allowed.contains(root));
                self.captures
                    .retain(|(id, _, _), _| self.documents.contains_key(id));
                self.statuses.retain(|key, _| {
                    key.split(':')
                        .next()
                        .and_then(|v| v.parse().ok())
                        .is_some_and(|id| allowed.contains(&id))
                });
                self.refresh_status();
            }
            Inbound::SettingsChanged(_) => {
                let processes = self.sessions.keys().copied().collect::<Vec<_>>();
                for process in processes {
                    self.with_session(process, |c, s| {
                        let init = c.setting("initialization", json!({}));
                        if init != s.initialization {
                            c.restart(s)?;
                        } else if s.ready {
                            s.notify(
                                "workspace/didChangeConfiguration",
                                json!({"settings":c.setting("configuration",json!({}))}),
                            )?;
                        }
                        Ok(())
                    })?;
                }
            }
            Inbound::WorkspaceFilesChanged(changes) => {
                let processes = self.sessions.keys().copied().collect::<Vec<_>>();
                for process in processes {
                    self.with_session(process, |c, s| {
                        for change in &changes {
                            let (method, paths) = match change {
                                WorkspaceFileChange::Created(path) => {
                                    ("workspace/didCreateFiles", vec![path])
                                }
                                WorkspaceFileChange::Deleted(path) => {
                                    ("workspace/didDeleteFiles", vec![path])
                                }
                                WorkspaceFileChange::Renamed((old, new)) => {
                                    ("workspace/didRenameFiles", vec![old, new])
                                }
                                WorkspaceFileChange::Rescan => {
                                    c.restart(s)?;
                                    return Ok(());
                                }
                            };
                            if !paths.iter().any(|p| inside(p, &s.path)) {
                                continue;
                            }
                            let files = if paths.len() == 2 {
                                json!([{"oldUri":uri(paths[0]),"newUri":uri(paths[1])}])
                            } else {
                                json!([{"uri":uri(paths[0])}])
                            };
                            s.notify(method, json!({"files":files}))?;
                        }
                        Ok(())
                    })?;
                }
            }
            Inbound::FileChanged(event) => self.file_changed(event)?,
            Inbound::WatchReady(id) => self.watch_ready(id, None)?,
            Inbound::WatchFailed(error) => {
                self.watch_ready(error.watch, Some(error.error.message))?
            }
            Inbound::Shutdown => {
                self.shutting_down = true;
                let timers = self.timers.keys().copied().collect::<Vec<_>>();
                for id in timers {
                    self.cancel_timer(Some(id));
                }
                let processes = self.sessions.keys().copied().collect::<Vec<_>>();
                for process in processes {
                    self.with_session(process, |c, s| {
                        if s.ready {
                            c.rpc(s, "shutdown", Value::Null, Purpose::Shutdown, 1000)?;
                        } else {
                            c.stop_session(s);
                        }
                        Ok(())
                    })?;
                }
                self.widget.dispose();
                self.statuses.clear();
            }
            _ => {}
        }
        Ok(())
    }
    fn close(&mut self, id: u64) -> Result<()> {
        self.captures.retain(|(document, _, _), _| *document != id);
        self.completion_sets.retain(|_, set| set.document != id);
        if let Some(doc) = self.documents.remove(&id) {
            self.with_session(doc.process, |_, s| {
                if s.ready && doc.server_version.is_some() {
                    s.notify(
                        "textDocument/didClose",
                        json!({"textDocument":{"uri":uri(&doc.reference.path)}}),
                    )?;
                }
                Ok(())
            })?;
            self.unused(doc.process);
        }
        self.refresh_status();
        Ok(())
    }
}

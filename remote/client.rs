//! SSH policy, connection UI and private Agent protocol live in this component.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
mod protocol;
mod manager;
use exports::loog::extension::guest::Guest;
use loog::extension::{host, types::*};
use loog::workspace::{session, tools};
use loog_workspace_api::{self as api, session::{Command, Event, Reply}};
use protocol::{Profile, Frames};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::{BTreeMap, VecDeque}};

const ID: &str = "loog.remote.ssh";
enum Pending { Hello, Bundle(usize), BundleReady, Mount, Operation(api::Request), AbandonedStart(api::Capability), Cleanup }
enum Ticket { Register(u64), Mount(u64), Started { process: u64, capability: api::Capability, handle: u64 }, Other }
struct Outgoing { id: Option<u64>, bytes: Vec<u8> }
#[derive(Default)]
struct Client {
    index: usize,
    profile: Profile,
    connected: Option<Profile>,
    process: Option<u64>,
    frames: Frames,
    next: u64,
    pending: BTreeMap<u64, Pending>,
    outgoing: VecDeque<Outgoing>,
    tickets: BTreeMap<u64, Ticket>,
    environment: Option<u64>,
    scope: Option<u64>,
    agent_root: Option<u64>,
    canonical_root: String,
    ui: Option<u64>,
    controls: BTreeMap<u64, &'static str>,
    status: Option<u64>,
    status_entry: Option<u64>,
    status_text: String,
    error_text: String,
    heartbeat: Option<u64>,
}
thread_local! { static CLIENT: RefCell<manager::Connections> = RefCell::new(manager::Connections::default()); }
struct Extension;
impl Guest for Extension {
    fn dispatch(batch: Vec<Inbound>) -> Result<RoundStatus, HostError> {
        CLIENT.with(|client| {
            let mut client = client.borrow_mut();
            for message in batch {
                client.message(message);
            }
            for event in session::poll() {
                let result = serde_json::from_str::<Event>(&event)
                    .map_err(|_| "Invalid workspace event".to_owned())
                    .map(|event| client.workspace(event));
                if let Err(error) = result { host::log(2, &error); }
            }
            client.flush();
            Ok(RoundStatus::Idle)
        })
    }
}
export!(Extension);

impl Client {
    fn submit(&mut self, command: Command, action: Ticket) -> Result<(), String> {
        if self.tickets.len() >= 256 { return Err("Workspace reply budget exceeded".into()); }
        let ticket = session::submit(&serde_json::to_string(&command).unwrap())?;
        self.tickets.insert(ticket, action);
        Ok(())
    }
    fn send(&mut self, method: &str, params: Value, pending: Pending) -> Result<(), String> {
        if self.pending.len() >= 128 { return Err("Agent request budget exceeded".into()); }
        self.next = self.next.checked_add(1).ok_or("Request identity exhausted")?;
        let bytes = protocol::frame(self.next, method, params);
        let queued: usize = self.outgoing.iter().map(|item| item.bytes.len()).sum();
        if bytes.len() > protocol::MAX_FRAME || queued + bytes.len() > 2 * protocol::MAX_FRAME {
            return Err("Agent input budget exceeded".into());
        }
        self.pending.insert(self.next, pending);
        self.outgoing.push_back(Outgoing { id: Some(self.next), bytes });
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        let Some(process) = self.process else { return Ok(()) };
        for _ in 0..32 {
            let Some(item) = self.outgoing.front() else { break };
            let count = host::process_write(process, &item.bytes).map_err(|e| e.message)? as usize;
            if count == 0 { break; }
            if count != item.bytes.len() { return Err("Incomplete process write".into()); }
            self.outgoing.pop_front();
        }
        Ok(())
    }
    fn connect(&mut self) -> Result<(), String> {
        if self.process.is_some() || self.environment.is_some() {
            return Err("Disconnect the current connection before connecting again".into());
        }
        let args = self.profile.arguments()?;
        host::storage_set(StorageScope::User, "profile", &serde_json::to_string(&self.profile).unwrap())
            .map_err(|e| e.message)?;
        self.frames = Frames::default();
        self.error_text.clear();
        self.process = Some(tools::launch("ssh", &args)?);
        self.connected = Some(self.profile.clone());
        self.outgoing.push_back(Outgoing { id: None, bytes: protocol::AGENT.to_vec() });
        self.send("hello", json!({}), Pending::Hello)?;
        self.heartbeat = Some(host::timer_start(10_000).map_err(|error| error.message)?);
        self.label("Connecting")
    }
    fn disconnect(&mut self, forget: bool) -> Result<(), String> {
        if let Some(timer) = self.heartbeat.take() { host::timer_cancel(timer); }
        if forget && self.scope.is_some() && let Some(profile) = &self.connected {
            let resource = api::ResourceRef::provided(api::EnvironmentKey::new(ID, profile.identity())?, self.canonical_root.clone())?;
            self.submit(Command::Forget { resource }, Ticket::Other)?;
        }
        if let Some(process) = self.process.take() { host::process_kill(process); }
        let mut failure = None;
        for (_, pending) in std::mem::take(&mut self.pending) {
            if let Pending::Operation(request) = pending {
                let kind = if request.operation.changes_state() { api::FailureKind::UnknownOutcome }
                    else { api::FailureKind::Unavailable };
                if let Err(error) = self.submit(Command::Complete { request: request.id,
                    result: Err(api::Failure::new(kind, "Connection closed")) }, Ticket::Other) {
                    failure.get_or_insert(error);
                }
            }
        }
        self.outgoing.clear();
        self.scope = None;
        self.agent_root = None;
        self.connected = None;
        if let Some(environment) = self.environment.take() {
            if let Err(error) = self.submit(Command::Unregister { environment }, Ticket::Other) {
                self.environment = Some(environment);
                failure.get_or_insert(error);
            }
        }
        self.label("Disconnected")?;
        failure.map_or(Ok(()), Err)
    }
    fn fail(&mut self, message: &str) {
        let _ = self.disconnect(false);
        let short: String = message.chars().take(300).collect();
        let _ = self.label(&short);
        host::log(2, &format!("Remote SSH: {short}"));
    }
    fn message(&mut self, message: Inbound) -> Result<(), String> {
        match message {
            Inbound::TimerElapsed(timer) if self.heartbeat == Some(timer) => {
                self.heartbeat = None;
                if self.process.is_some() {
                    self.send("ping", json!({}), Pending::Cleanup)?;
                    self.heartbeat = Some(host::timer_start(10_000).map_err(|error| error.message)?);
                }
            }
            Inbound::UiEvent(event) => match (self.controls.get(&event.node).copied().unwrap_or(""), event.payload) {
                ("target", UiEventPayload::Input(text)) => self.profile.target = text,
                ("root", UiEventPayload::Input(text)) => self.profile.root = text,
                ("config", UiEventPayload::Input(text)) => self.profile.config = text,
                ("port", UiEventPayload::Input(text)) => self.profile.port = text.parse().unwrap_or(0),
                ("connect", UiEventPayload::Click) => self.connect()?,
                ("disconnect", UiEventPayload::Click) => self.disconnect(true)?,
                ("close", UiEventPayload::Click) => self.hide()?,
                ("show", UiEventPayload::Click) => self.show()?,
                _ => {}
            },
            Inbound::ProcessOutput(output) if Some(output.process) == self.process => {
                if matches!(output.channel, OutputChannel::Stderr) {
                    if self.error_text.len() < 4096 {
                        let bytes = &output.bytes[..output.bytes.len().min(4096 - self.error_text.len())];
                        self.error_text.push_str(&String::from_utf8_lossy(bytes));
                    }
                } else {
                    for value in self.frames.push(&output.bytes)? { self.agent(value)?; }
                }
            }
            Inbound::ProcessExited(exit) if Some(exit.process) == self.process => {
                self.process = None;
                return Err(exit.error.map(|e| e.message).unwrap_or_else(|| {
                    if self.error_text.is_empty() { "SSH connection ended".into() } else { self.error_text.clone() }
                }));
            }
            _ => {}
        }
        Ok(())
    }
    fn agent(&mut self, value: Value) -> Result<(), String> {
        if let Some(value) = value.get("event") {
            let event = protocol::notification(value.clone()).map_err(|error| error.message)?;
            if let Some(scope) = self.scope {
                self.submit(Command::Notify { scope, event }, Ticket::Other)?;
            }
            return Ok(());
        }
        let id = value["id"].as_u64().ok_or("Agent reply has no request id")?;
        let Some(pending) = self.pending.remove(&id) else { return Ok(()) };
        let result = protocol::response(&value);
        match pending {
            Pending::Hello => {
                let value = result.map_err(|e| e.message)?;
                if value["protocol"] != 1 || value["platform"] != "linux" {
                    return Err("This provider requires the bundled Agent on Linux".into());
                }
                if value["machine"] != "x86_64" {
                    return Err("This SSH package currently supports Linux x86_64 targets".into());
                }
                self.label("Preparing workspace search")?;
                self.send("search_bundle_begin", json!({"size":protocol::SEARCH_SIZE.parse::<usize>().map_err(|_| "Invalid bundled search size")?,
                    "sha256":protocol::SEARCH_SHA256}), Pending::Bundle(0))?;
            }
            Pending::Bundle(offset) => {
                use base64::Engine;
                result.map_err(|e| e.message)?;
                let end = (offset + 128 * 1024).min(protocol::SEARCH_BUNDLE.len());
                if end > offset {
                    self.send("search_bundle_append", json!({"offset":offset,
                        "data":base64::engine::general_purpose::STANDARD.encode(&protocol::SEARCH_BUNDLE[offset..end])}), Pending::Bundle(end))?;
                } else {
                    self.send("search_bundle_finish", json!({}), Pending::BundleReady)?;
                }
            }
            Pending::BundleReady => {
                result.map_err(|e| e.message)?;
                let profile = self.connected.as_ref().ok_or("Connection ended during handshake")?;
                self.send("mount", json!({"path":profile.root}), Pending::Mount)?;
            }
            Pending::Mount => {
                let value = result.map_err(|e| e.message)?;
                self.agent_root = Some(value["root"].as_u64().ok_or("Invalid root handle")?);
                self.canonical_root = value["path"].as_str().ok_or("Invalid root identity")?.into();
                let profile = self.connected.as_ref().ok_or("Connection ended during handshake")?;
                let process = self.process.ok_or("Connection ended during handshake")?;
                self.submit(Command::Register { descriptor: api::EnvironmentDescriptor {
                    key: api::EnvironmentKey::new(ID, profile.identity())?,
                    name: profile.target.clone(), platform: "linux".into(),
                    capabilities: vec![api::Capability::FilesRead, api::Capability::FilesWrite, api::Capability::Watch, api::Capability::Search, api::Capability::Process, api::Capability::Terminal],
                } }, Ticket::Register(process))?;
            }
            Pending::Operation(request) => {
                let result = result.and_then(|value| protocol::answer(&request.operation, value));
                let ticket = match &result {
                    Ok(api::Answer::Started { handle }) => Ticket::Started {
                        process: self.process.unwrap_or(0), capability: request.operation.capability(), handle: *handle },
                    _ => Ticket::Other,
                };
                self.submit(Command::Complete { request: request.id, result }, ticket)?;
            }
            Pending::AbandonedStart(capability) => {
                if let Ok(value) = result {
                    let field = match capability {
                        api::Capability::Watch => "watch",
                        api::Capability::Search => "search",
                        api::Capability::Process => "process",
                        api::Capability::Terminal => "terminal",
                        _ => "write",
                    };
                    if let Some(handle) = value[field].as_u64() {
                        self.release(capability, handle)?;
                    }
                }
            }
            Pending::Cleanup => {}
        }
        Ok(())
    }
    fn workspace(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Reply { ticket, result } => {
                let Some(action) = self.tickets.remove(&ticket) else { return Ok(()) };
                if matches!(&action, Ticket::Register(process) | Ticket::Mount(process) if self.process != Some(*process)) && result.is_err() {
                    return Ok(());
                }
                if let Ticket::Started { process, capability, handle } = &action {
                    if self.process == Some(*process) {
                        if result.is_err() {
                            self.release(*capability, *handle)?;
                        } else if *capability == api::Capability::Watch {
                            self.send("watch_ready", json!({"root":self.agent_root,"watch":handle}), Pending::Cleanup)?;
                        } else if *capability == api::Capability::Process {
                            self.send("process_ready", json!({"root":self.agent_root,"process":handle}), Pending::Cleanup)?;
                        } else if *capability == api::Capability::Terminal {
                            self.send("terminal_ready", json!({"root":self.agent_root,"terminal":handle}), Pending::Cleanup)?;
                        }
                    }
                    return Ok(());
                }
                // Completion/cancellation can cross in flight. The broker's
                // terminal result wins; an expired receipt is not a connection failure.
                if matches!(&action, Ticket::Other) && matches!(&result,
                    Err(api::Failure { kind: api::FailureKind::Unavailable, .. })) { return Ok(()) }
                let reply = result.map_err(|e| e.message)?;
                match (action, reply) {
                    (Ticket::Register(process), Reply::Environment { id }) => {
                        if self.process != Some(process) {
                            self.submit(Command::Unregister { environment: id }, Ticket::Other)?;
                        } else {
                            self.environment = Some(id);
                            self.submit(Command::Mount { environment: id,
                                root: api::ProviderPath::new(self.canonical_root.clone())? }, Ticket::Mount(process))?;
                        }
                    }
                    (Ticket::Mount(process), Reply::Scope { id }) if self.process == Some(process) => {
                        self.scope = Some(id);
                        self.submit(Command::Present { scope: id }, Ticket::Other)?;
                        let mut profile = self.connected.clone().ok_or("Connection profile is unavailable")?;
                        profile.root = self.canonical_root.clone();
                        self.submit(Command::Remember { scope: id, state: serde_json::to_string(&profile).unwrap() }, Ticket::Other)?;
                        self.label("Connected")?;
                    }
                    _ => {}
                }
            }
            Event::Delivery { event: api::Delivery::Request(request) } => {
                let result = self.agent_root.filter(|_| Some(request.scope) == self.scope)
                    .ok_or_else(|| api::Failure::new(api::FailureKind::Unavailable, "Workspace is disconnected"))
                    .and_then(|root| protocol::operation(root, &request.operation));
                match result {
                    Ok((method, mut params)) => {
                        params["excluded_roots"] = serde_json::to_value(&request.excluded_roots).unwrap();
                        let id = request.id;
                        if let Err(message) = self.send(method, params, Pending::Operation(request)) {
                            self.submit(Command::Complete { request: id,
                                result: Err(api::Failure::new(api::FailureKind::LimitExceeded, message)) }, Ticket::Other)?;
                        }
                    }
                    Err(error) => self.submit(Command::Complete { request: request.id, result: Err(error) }, Ticket::Other)?,
                }
            }
            Event::Delivery { event: api::Delivery::Cancel { request } } => {
                let id = self.pending.iter().find_map(|(id, item)| matches!(item,
                    Pending::Operation(item) if item.id == request).then_some(*id));
                if let Some(id) = id {
                    let unsent = self.outgoing.iter().any(|item| item.id == Some(id));
                    let started = match self.pending.get(&id) {
                        Some(Pending::Operation(api::Request { operation, .. }))
                            if matches!(operation, api::Operation::BeginWrite { .. } | api::Operation::Watch { .. } | api::Operation::Search { .. } | api::Operation::Spawn { .. } | api::Operation::TerminalOpen { .. }) =>
                                Some(operation.capability()),
                        _ => None,
                    };
                    self.pending.remove(&id);
                    if !unsent && let Some(capability) = started {
                        self.pending.insert(id, Pending::AbandonedStart(capability));
                    }
                    self.outgoing.retain(|item| item.id != Some(id));
                }
            }
            Event::Delivery { event: api::Delivery::Release { capability, handle, .. } } => self.release(capability, handle)?,
            _ => {}
        }
        Ok(())
    }
    fn release(&mut self, capability: api::Capability, handle: u64) -> Result<(), String> {
        if let Some(root) = self.agent_root {
            let (method, params) = match capability {
                api::Capability::FilesWrite => ("write_abort", json!({"root":root,"write":handle})),
                api::Capability::Watch => ("unwatch", json!({"root":root,"watch":handle})),
                api::Capability::Search => ("search_close", json!({"root":root,"search":handle})),
                api::Capability::Process => ("process_kill", json!({"root":root,"process":handle})),
                api::Capability::Terminal => ("terminal_close", json!({"root":root,"terminal":handle})),
                _ => return Ok(()),
            };
            self.send(method, params, Pending::Cleanup)?;
        }
        Ok(())
    }
    fn label(&mut self, text: &str) -> Result<(), String> {
        self.status_text = text.into();
        if let Some(status) = self.status {
            let mut desc = description("status", UiKind::Text, text, Some(480.0));
            desc.height = Some(68.0);
            host::ui_update(status, &desc).map_err(|e| e.message)?;
        }
        if let Some(entry) = self.status_entry {
            let state = if self.scope.is_some() { "Connected" } else if self.process.is_some() { "Connecting" } else { "Disconnected" };
            let target = self.connected.as_ref().unwrap_or(&self.profile).target.chars().take(48).collect::<String>();
            let caption = if target.is_empty() { "SSH".into() } else { format!("SSH {target}: {state}") };
            host::ui_update(entry, &description(&self.status_id(), UiKind::Button, &caption, None)).map_err(|e| e.message)?;
        }
        Ok(())
    }
    fn hide(&mut self) -> Result<(), String> {
        if let Some(node) = self.ui {
            host::ui_remove(node).map_err(|e| e.message)?;
            self.ui = None;
        }
        self.status = None;
        self.controls.retain(|node, _| Some(*node) == self.status_entry);
        Ok(())
    }
    fn show(&mut self) -> Result<(), String> {
        if self.ui.is_some() { return Ok(()) }
        if self.status_entry.is_none() {
            let entry = host::ui_create(&UiParent::Mount("ui.detached".into()),
                &description(&self.status_id(), UiKind::Button, "SSH", None)).map_err(|e| e.message)?;
            loog::ui::tree::place(entry, &loog::ui::tree::Placement {
                target: loog::ui::tree::Target::PublicId("statusbar.extensions".into()),
                relation: loog::ui::tree::Relation::Append, order: 0,
            })?;
            self.status_entry = Some(entry);
            self.controls.insert(entry, "show");
        }
        if self.profile.target.is_empty() && let Some(saved) = host::storage_get(StorageScope::User, "profile") {
            self.profile = serde_json::from_str(&saved).unwrap_or_default();
        }
        let mut panel = description("connection", UiKind::Column, "", Some(512.0));
        panel.height = Some(326.0);
        panel.padding = 16.0;
        panel.gap = 8.0;
        panel.background = Some(0xff202228);
        let node = host::ui_create(&UiParent::Mount("ui.detached".into()),
            &panel).map_err(|e| e.message)?;
        loog::ui::tree::place(node, &loog::ui::tree::Placement {
            target: loog::ui::tree::Target::PublicId("window.overlays".into()),
            relation: loog::ui::tree::Relation::Append, order: 0,
        })?;
        self.ui = Some(node);
        host::ui_create(&UiParent::Node(node), &description("title", UiKind::Text, "Remote SSH", None)).map_err(|e| e.message)?;
        for (id, title, value) in [("target", "SSH target", self.profile.target.clone()),
            ("port", "Port", self.profile.port.to_string()), ("root", "Folder", self.profile.root.clone()),
            ("config", "Config file", self.profile.config.clone())] {
            let row = host::ui_create(&UiParent::Node(node), &description(&format!("{id}-row"), UiKind::Row, "", Some(480.0))).map_err(|e| e.message)?;
            host::ui_create(&UiParent::Node(row), &description(&format!("{id}-label"), UiKind::Text, title, Some(92.0))).map_err(|e| e.message)?;
            let mut input = description(id, UiKind::TextInput, &value, Some(374.0));
            input.background = Some(0xff141519);
            let control = host::ui_create(&UiParent::Node(row), &input)
                .map_err(|e| e.message)?;
            self.controls.insert(control, id);
        }
        let buttons = host::ui_create(&UiParent::Node(node), &description("actions", UiKind::Row, "", None)).map_err(|e| e.message)?;
        for (id, text) in [("connect", "Connect"), ("disconnect", "Disconnect"), ("close", "Close")] {
            let control = host::ui_create(&UiParent::Node(buttons), &description(id, UiKind::Button, text, Some(112.0))).map_err(|e| e.message)?;
            self.controls.insert(control, id);
        }
        self.status = Some(host::ui_create(&UiParent::Node(node),
            &description("status", UiKind::Text, "", Some(480.0))).map_err(|e| e.message)?);
        let status = if self.status_text.is_empty() { "Use an existing SSH key or agent. Config file is optional.".into() } else { self.status_text.clone() };
        self.label(&status)
    }
    fn status_id(&self) -> String {
        if self.index == 0 { "show".into() } else { format!("show-{}", self.index) }
    }
}
fn description(id: &str, kind: UiKind, text: &str, width: Option<f32>) -> UiDescription {
    UiDescription { id: id.into(), kind, text: Some(text.into()), width, height: None,
        icon: None, foreground: None, background: None, padding: 2.0, gap: 6.0, visible: true,
        label: Some(match id { "target" => "SSH alias or user@host", "port" => "SSH port",
            "config" => "Optional SSH configuration file",
            "root" => "Remote folder", _ => text }.into()) }
}

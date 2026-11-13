//! SSH policy, connection UI and private Agent protocol live in this component.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
mod protocol;
mod manager;
use exports::loog::extension::guest::Guest;
use loog::extension::{host, types::*};
use loog::palette::picker;
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
    /// The status bar entry; selecting it opens this connection's menu.
    controls: BTreeMap<u64, &'static str>,
    status_entry: Option<u64>,
    /// The mark and the host name inside the status bar entry.
    entry_parts: Option<(u64, u64)>,
    status_text: String,
    error_text: String,
    /// Why the last attempt ended, until the next attempt starts.
    failure: Option<String>,
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
            for event in picker::poll() { client.palette(event); }
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
        self.ensure_entry()?;
        self.frames = Frames::default();
        self.error_text.clear();
        self.failure = None;
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
        if forget {
            // The person ended it: nothing is left to show in the status bar.
            self.status_text = "Disconnected".into();
            self.remove_entry();
        } else {
            self.label("Disconnected")?;
        }
        failure.map_or(Ok(()), Err)
    }
    fn fail(&mut self, message: &str) {
        let _ = self.disconnect(false);
        let short: String = message.chars().take(300).collect();
        self.failure = Some(short.clone());
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
        self.paint_entry()
    }
    /// How the connection looks in the status bar: the host, in the colors of its state.
    fn entry_look(&self) -> (&'static str, u32, u32, &'static str) {
        if self.scope.is_some() { ("Connected", 0xfff4f4f5, 0xff6366f1, SWAP_ICON) }
        else if self.process.is_some() { ("Connecting", 0xffffe9a6, 0xff6b4f0b, SWAP_ICON) }
        else if self.failure.is_some() { ("Failed", 0xffffd5db, 0xff7f1d2d, ALERT_ICON) }
        else { ("Disconnected", 0xffa1a1aa, 0xff262a35, SWAP_ICON) }
    }
    /// Bring the status bar entry up to date with the connection.
    fn paint_entry(&mut self) -> Result<(), String> {
        let (Some(entry), Some((icon, text))) = (self.status_entry, self.entry_parts) else { return Ok(()) };
        let (state, foreground, background, mark) = self.entry_look();
        let target = self.connected.as_ref().unwrap_or(&self.profile).label();
        let target: String = target.chars().take(40).collect();
        let mut root = entry_node(&self.status_id(), UiKind::Block, "", None, foreground, Some(background));
        root.label = Some(format!("Remote SSH: {target}, {state}"));
        host::ui_update(entry, &root).map_err(|e| e.message)?;
        let mut icon_node = entry_node("icon", UiKind::Icon, "", Some(14.0), foreground, None);
        icon_node.icon = Some(mark.into());
        host::ui_update(icon, &icon_node).map_err(|e| e.message)?;
        host::ui_update(text, &entry_node("host", UiKind::Text, &target, None, foreground, None)).map_err(|e| e.message)?;
        Ok(())
    }
    /// The status bar entry of this connection, made the first time it connects: a mark and the
    /// host, at the start of the bar's right side, before the language services. Selecting it
    /// opens this connection's actions.
    fn ensure_entry(&mut self) -> Result<(), String> {
        if self.status_entry.is_some() { return Ok(()) }
        let entry = host::ui_create(&UiParent::Mount("ui.detached".into()),
            &entry_node(&self.status_id(), UiKind::Block, "", None, 0xfff4f4f5, Some(0xff6366f1))).map_err(|e| e.message)?;
        loog::ui::tree::place(entry, &loog::ui::tree::Placement {
            target: loog::ui::tree::Target::PublicId("statusbar.start".into()),
            relation: loog::ui::tree::Relation::Before, order: self.index as i32,
        })?;
        let part = |id: &str, kind: UiKind, width: Option<f32>| host::ui_create(&UiParent::Node(entry),
            &entry_node(id, kind, "", width, 0xfff4f4f5, None)).map_err(|e| e.message);
        part("lead", UiKind::Block, Some(6.0))?;
        let icon = part("icon", UiKind::Icon, Some(14.0))?;
        let text = part("host", UiKind::Text, None)?;
        part("trail", UiKind::Block, Some(6.0))?;
        self.status_entry = Some(entry);
        self.entry_parts = Some((icon, text));
        self.controls.insert(entry, "menu");
        self.paint_entry()
    }
    /// Take the entry away, when the person ended the connection.
    fn remove_entry(&mut self) {
        if let Some(entry) = self.status_entry.take() {
            let _ = host::ui_remove(entry);
            self.controls.remove(&entry);
        }
        self.entry_parts = None;
    }
    fn status_id(&self) -> String {
        if self.index == 0 { "show".into() } else { format!("show-{}", self.index) }
    }
}
// Material Design Icons (Pictogrammers), Apache-2.0.
const SWAP_ICON: &str = "svg-path:24x24:M21,9L17,5V8H10V10H17V13M7,11L3,15L7,19V16H14V14H7V11Z";
const ALERT_ICON: &str = "svg-path:24x24:M13,13H11V7H13M13,17H11V15H13M12,2A10,10 0 0,0 2,12A10,10 0 0,0 12,22A10,10 0 0,0 22,12A10,10 0 0,0 12,2Z";
/// The height of the status bar, which the entry fills so no gap shows under its color.
const BAR_HEIGHT: f32 = 24.0;
/// A part of the status bar entry: as high as the bar, with no padding of its own.
fn entry_node(id: &str, kind: UiKind, text: &str, width: Option<f32>, foreground: u32, background: Option<u32>) -> UiDescription {
    UiDescription { id: id.into(), kind, text: Some(text.into()), width, height: Some(BAR_HEIGHT),
        icon: None, foreground: Some(foreground), background, padding: 0.0, gap: 4.0, visible: true,
        label: None }
}

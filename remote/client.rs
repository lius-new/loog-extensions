//! Channel policy, connection UI and private Agent protocol live in this component. One
//! source builds the SSH, WSL and container packages; see `transport`.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
mod protocol;
mod transport;
mod listing;
mod folders;
mod manager;
use exports::loog::extension::guest::Guest;
use loog::extension::{host, types::*};
use loog::palette::picker;
use loog::workspace::{session, tools};
use loog_workspace_api::{self as api, session::{Command, Event, Reply}};
use protocol::{Profile, Frames};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::{BTreeMap, VecDeque}};

use transport::ID;
enum Pending { Hello, Mount, Browse(String), Operation(api::Request), AbandonedStart(api::Capability), Cleanup }
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
    /// What the target has said before the agent runs: waiting for its architecture.
    boot: Option<Vec<u8>>,
    /// The folder is to be chosen by the person, after the Agent is ready and before one is mounted.
    choose_folder: bool,
    /// The Agent is ready and the connection waits for a folder to be opened.
    awaiting_folder: bool,
    /// Why the folder that was chosen could not be opened, for the person to choose again.
    mount_error: Option<String>,
    /// The machine's own name, as its Agent said it, to show a host that was given by address.
    hostname: Option<String>,
    /// How many folders the application has accepted on this connection. A folder picker
    /// that is open on a live connection is done when this goes up, not when a scope exists.
    mounts: u32,
    /// Folders the Agent has listed since they were last taken: what was asked, what the
    /// Agent named it, and the listing or the reason.
    browsed: Vec<(String, Result<(String, folders::Listing), String>)>,
}
/// The last part of a folder's path, which is how a folder is named; `/` for the root.
fn folder_name(path: &str) -> &str {
    match path.trim_end_matches('/').rsplit('/').next() {
        Some(name) if !name.is_empty() => name,
        _ => "/",
    }
}
/// The host's name as a badge, which the application takes up to 64 bytes of.
fn badge_of(name: &str) -> Option<String> {
    let mut badge = String::new();
    for c in name.chars().filter(|c| !c.is_control()) {
        if badge.len() + c.len_utf8() > 64 { break; }
        badge.push(c);
    }
    (!badge.is_empty()).then_some(badge)
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
        // The agent upload is not a request: only requests count against the input budget.
        let queued: usize = self.outgoing.iter().filter(|item| item.id.is_some()).map(|item| item.bytes.len()).sum();
        if bytes.len() > protocol::MAX_FRAME || queued + bytes.len() > 2 * protocol::MAX_FRAME {
            return Err("Agent input budget exceeded".into());
        }
        self.pending.insert(self.next, pending);
        self.outgoing.push_back(Outgoing { id: Some(self.next), bytes });
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        let Some(process) = self.process else { return Ok(()) };
        // Whole-message admission: a zero means wait for process-writable, which runs another round.
        for _ in 0..1024 {
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
        let (program, args) = self.profile.launch()?;
        self.ensure_entry()?;
        self.frames = Frames::default();
        self.error_text.clear();
        self.failure = None;
        self.awaiting_folder = false;
        self.mount_error = None;
        self.hostname = None;
        self.browsed.clear();
        self.process = Some(tools::launch(program, &args)?);
        self.connected = Some(self.profile.clone());
        // The target answers with its architecture first; the agent is sent after that.
        self.boot = Some(Vec::new());
        self.heartbeat = Some(host::timer_start(10_000).map_err(|error| error.message)?);
        self.label("Connecting")
    }
    fn disconnect(&mut self, forget: bool) -> Result<(), String> {
        if let Some(timer) = self.heartbeat.take() { host::timer_cancel(timer); }
        self.boot = None;
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
        host::log(2, &format!("{}: {short}", transport::TITLE));
    }
    fn message(&mut self, message: Inbound) -> Result<(), String> {
        match message {
            Inbound::TimerElapsed(timer) if self.heartbeat == Some(timer) => {
                self.heartbeat = None;
                if self.process.is_some() {
                    // Nothing may precede the upload on the channel.
                    if self.boot.is_none() { self.send("ping", json!({}), Pending::Cleanup)?; }
                    self.heartbeat = Some(host::timer_start(10_000).map_err(|error| error.message)?);
                }
            }
            Inbound::ProcessOutput(output) if Some(output.process) == self.process => {
                if matches!(output.channel, OutputChannel::Stderr) {
                    if self.error_text.len() < 4096 {
                        let bytes = &output.bytes[..output.bytes.len().min(4096 - self.error_text.len())];
                        self.error_text.push_str(&listing::decode(bytes));
                    }
                } else if self.boot.is_some() {
                    self.arrival(&output.bytes)?;
                } else {
                    for value in self.frames.push(&output.bytes)? { self.agent(value)?; }
                }
            }
            Inbound::ProcessExited(exit) if Some(exit.process) == self.process => {
                self.process = None;
                return Err(exit.error.map(|e| e.message).unwrap_or_else(|| {
                    if self.error_text.is_empty() { "The connection ended".into() } else { self.error_text.clone() }
                }));
            }
            _ => {}
        }
        Ok(())
    }
    /// The target's first words: look for its architecture, then send the matching agent.
    /// A login script can print before the loader does, so other lines are skipped.
    fn arrival(&mut self, bytes: &[u8]) -> Result<(), String> {
        let boot = self.boot.as_mut().ok_or("The agent is already running")?;
        boot.extend_from_slice(bytes);
        if boot.len() > 16 * 1024 { return Err("The target did not start the agent loader".into()); }
        let mut start = 0;
        while let Some(length) = boot[start..].iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8_lossy(&boot[start..start + length]).trim_end_matches('\r').to_owned();
            start += length + 1;
            let Some(answer) = line.strip_prefix(protocol::ARCH_PREFIX) else { continue };
            let rest = boot[start..].to_vec();
            self.boot = None;
            let agent = protocol::agent_for(answer).ok_or_else(|| format!(
                "This package supports Linux x86_64 and aarch64 targets, not {}", protocol::machine_name(answer)))?;
            self.label("Starting the agent")?;
            self.outgoing.push_back(Outgoing { id: None, bytes: protocol::upload_header(agent).into_bytes() });
            // Chunks stay far below the host's whole-message admission window.
            for chunk in agent.bytes.chunks(32 * 1024) {
                self.outgoing.push_back(Outgoing { id: None, bytes: chunk.to_vec() });
            }
            self.send("hello", json!({}), Pending::Hello)?;
            for value in self.frames.push(&rest)? { self.agent(value)?; }
            return Ok(());
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
                if value["protocol"] != 2 || value["platform"] != "linux" {
                    return Err("This provider requires the bundled Agent on Linux".into());
                }
                // A name the machine gives itself is shown as it is, but only if it is plain.
                self.hostname = value["hostname"].as_str()
                    .filter(|name| !name.is_empty() && name.chars().count() <= 64
                        && name.chars().all(|c| !c.is_control() && !c.is_whitespace()))
                    .map(str::to_owned);
                self.paint_entry()?;
                if self.choose_folder {
                    // The person picks the folder; the Agent is idle until one is opened.
                    self.awaiting_folder = true;
                    return self.label("Choose a folder");
                }
                let profile = self.connected.as_ref().ok_or("Connection ended during handshake")?;
                self.send("mount", json!({"path":profile.root}), Pending::Mount)?;
            }
            Pending::Browse(path) => {
                self.browsed.push((path, result.map_err(|e| e.message).and_then(|value| {
                    let canonical = value["path"].as_str().ok_or("Invalid folder listing")?.to_owned();
                    let entries = value["entries"].as_array().ok_or("Invalid folder listing")?.iter().filter_map(|entry| {
                        Some(folders::Entry { name: entry["name"].as_str()?.to_owned(), dir: entry["kind"] == "directory" })
                    }).collect();
                    Ok((canonical, folders::Listing::new(entries, value["more"].as_bool().unwrap_or(false))))
                })));
            }
            Pending::Mount => {
                // A folder the person chose may not open; the connection stays for another choice.
                let value = match result {
                    Ok(value) => value,
                    Err(error) if self.choose_folder => {
                        self.mount_error = Some(error.message);
                        self.awaiting_folder = true;
                        // A live connection keeps the folder it had.
                        if self.scope.is_some() {
                            let kept = self.canonical_root.clone();
                            if let Some(profile) = &mut self.connected { profile.root = kept; }
                        }
                        return Ok(());
                    }
                    Err(error) => return Err(error.message),
                };
                self.choose_folder = false;
                let root = value["root"].as_u64().ok_or("Invalid root handle")?;
                let canonical: String = value["path"].as_str().ok_or("Invalid root identity")?.into();
                let process = self.process.ok_or("Connection ended during handshake")?;
                if let Some(environment) = self.environment.take() {
                    // Another folder on a connection that stays up: the environment of the folder
                    // it had is let go, which takes that folder out of the file tree, and the
                    // new folder is registered below. The process and the Agent are not touched.
                    self.scope = None;
                    self.submit(Command::Unregister { environment }, Ticket::Other)?;
                }
                self.agent_root = Some(root);
                self.canonical_root = canonical;
                let profile = self.connected.as_ref().ok_or("Connection ended during handshake")?;
                self.submit(Command::Register { descriptor: api::EnvironmentDescriptor {
                    key: api::EnvironmentKey::new(ID, profile.identity())?,
                    // The file tree names the root by its folder and shows the host as a badge.
                    name: folder_name(&self.canonical_root).to_owned(),
                    badge: badge_of(&profile.host_name(self.hostname.as_deref())),
                    platform: "linux".into(),
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
                        self.mounts += 1;
                        // Presented for this session only: the file tree does not keep connections.
                        // The palette lists the hosts used before.
                        self.submit(Command::Present { scope: id }, Ticket::Other)?;
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
        let profile = self.connected.as_ref().unwrap_or(&self.profile);
        let full = profile.label();
        let target: String = profile.host_name(self.hostname.as_deref()).chars().take(40).collect();
        let mut root = entry_node(&self.status_id(), UiKind::Block, "", None, foreground, Some(background));
        // What is read aloud keeps the whole target; the bar shows the short name.
        root.label = Some(format!("{}: {full}, {state}", transport::TITLE));
        host::ui_update(entry, &root).map_err(|e| e.message)?;
        let mut icon_node = entry_node("icon", UiKind::Icon, "", Some(14.0), foreground, None);
        icon_node.icon = Some(mark.into());
        host::ui_update(icon, &icon_node).map_err(|e| e.message)?;
        host::ui_update(text, &entry_node("host", UiKind::Text, &target, None, foreground, None)).map_err(|e| e.message)?;
        Ok(())
    }
    /// Ask the Agent what a folder holds, for choosing one. The answer is in `browsed`.
    fn browse(&mut self, path: &str) -> Result<(), String> {
        self.send("browse", json!({"path": path}), Pending::Browse(path.to_owned()))
    }
    /// Open the folder the person chose, which goes on with the connection.
    fn open_folder(&mut self, path: &str) -> Result<(), String> {
        self.awaiting_folder = false;
        self.mount_error = None;
        if let Some(profile) = &mut self.connected { profile.root = path.to_owned(); }
        self.send("mount", json!({"path": path}), Pending::Mount)
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

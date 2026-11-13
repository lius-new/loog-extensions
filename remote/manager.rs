//! Bounded independent SSH connections and the command palette flow that opens them.
//! Host tickets, scopes and process IDs route to their owner; Agent handles are
//! meaningful only inside that owner. The palette shows only words and ids this
//! component supplies; the application never learns what they stand for.
use super::*;
use picker::{Accepted, Activity, Choice, Entry, Mode, View};

const MAX_CONNECTIONS: usize = 4;
const MAX_SAVED: usize = 32;
const STORAGE_KEY: &str = "hosts";
const TITLE: &str = "Remote SSH";

/// Where the person is in the palette.
enum Step {
    /// A host is being typed.
    Host,
    /// The host is known; how to authenticate is being chosen.
    Auth(Profile),
    /// The system file dialog is open to choose a private key.
    Key { profile: Profile, request: u64 },
    /// One connection is being made, or has failed.
    Connecting { client: usize, profile: Profile, failed: bool },
    /// The actions of one connection.
    Menu(usize),
}
struct Flow {
    session: u64,
    step: Step,
}

pub(super) struct Connections {
    clients: Vec<Client>,
    selected: usize,
    flow: Option<Flow>,
    saved: Vec<Profile>,
    started: bool,
    /// What was last published as palette entries, to publish again only a change.
    published: Vec<(String, String, Option<String>, String)>,
}
impl Default for Connections {
    fn default() -> Self {
        Self { clients: vec![Client::default()], selected: 0, flow: None, saved: Vec::new(),
            started: false, published: Vec::new() }
    }
}

/// The words of `text` on one line, cut to what the palette shows. SSH reports
/// failures over several lines; the palette takes one.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(180).collect()
}

fn view(tags: &[&str], placeholder: &str, mode: Mode, choices: Vec<Choice>, activity: Activity, note: Option<String>) -> View {
    View { tags: tags.iter().map(|tag| (*tag).to_owned()).collect(), placeholder: placeholder.into(),
        mode, choices, activity, note: note.map(|note| one_line(&note)) }
}
fn choice(id: &str, title: &str, detail: Option<&str>, trailing: Option<&str>) -> Choice {
    Choice { id: id.into(), title: title.into(), detail: detail.map(Into::into), group: None,
        trailing: trailing.map(Into::into) }
}

impl Connections {
    fn candidate(&mut self) -> Result<usize, String> {
        if let Some(index) = self.clients.iter().position(|client| {
            client.process.is_none() && client.environment.is_none()
                && client.tickets.is_empty() && client.pending.is_empty()
        }) { return Ok(index); }
        if self.clients.len() >= MAX_CONNECTIONS {
            return Err("At most four SSH environments can be connected; disconnect one first".into());
        }
        let index = self.clients.len();
        self.clients.push(Client { index, ..Client::default() });
        Ok(index)
    }

    /// Load what is stored the first time anything happens, and publish it.
    fn start(&mut self) {
        if self.started { return; }
        self.started = true;
        if let Some(text) = host::storage_get(StorageScope::User, STORAGE_KEY) {
            self.saved = serde_json::from_str::<Vec<Profile>>(&text).unwrap_or_default()
                .into_iter().filter(|profile| profile.validate().is_ok()).take(MAX_SAVED).collect();
        }
        self.publish();
    }

    /// The hosts and connections the palette lists when their words are typed.
    fn publish(&mut self) {
        let mut entries = Vec::new();
        for (index, client) in self.clients.iter().enumerate() {
            if let Some(profile) = client.connected.as_ref().filter(|_| client.scope.is_some()) {
                entries.push((format!("disconnect:{index}"), format!("{TITLE}: Disconnect {}", profile.label()),
                    Some(profile.root.clone()), "Connected".to_owned()));
            }
        }
        for profile in &self.saved {
            entries.push((format!("host:{}", profile.identity()), format!("{TITLE}: {}", profile.label()),
                Some(folder(&profile.root)), "Saved Hosts".to_owned()));
        }
        if entries == self.published { return; }
        let list: Vec<Entry> = entries.iter().map(|(id, title, detail, category)| Entry {
            id: id.clone(), title: title.clone(), detail: detail.clone(), category: category.clone() }).collect();
        match picker::publish_entries(&list) {
            Ok(()) => self.published = entries,
            Err(error) => host::log(2, &format!("Remote SSH: {error}")),
        }
    }

    /// Remember a host that connected, most recent first.
    fn remember(&mut self, profile: Profile) {
        self.saved.retain(|saved| saved.identity() != profile.identity());
        self.saved.insert(0, profile);
        self.saved.truncate(MAX_SAVED);
        if let Err(error) = host::storage_set(StorageScope::User, STORAGE_KEY, &serde_json::to_string(&self.saved).unwrap()) {
            host::log(2, &format!("Remote SSH: {}", error.message));
        }
    }

    /// Open the palette on the first step of a new connection.
    fn begin(&mut self) {
        let shown = view(&[TITLE], "user@host[:port]", Mode::Text, vec![], Activity::Idle, None);
        self.open(Step::Host, shown);
    }
    fn open(&mut self, step: Step, shown: View) {
        match picker::open(&shown) {
            Ok(session) => self.flow = Some(Flow { session, step }),
            Err(error) => host::log(2, &format!("Remote SSH: {error}")),
        }
    }
    fn show(&mut self, step: Step, shown: View) {
        let Some(flow) = &mut self.flow else { return };
        if let Err(error) = picker::update(flow.session, &shown) { host::log(2, &format!("Remote SSH: {error}")); }
        flow.step = step;
    }
    /// Say something in the open palette without changing the step.
    fn note(&mut self, message: &str, tags: &[&str], placeholder: &str, mode: Mode, choices: Vec<Choice>) {
        if let Some(flow) = &self.flow {
            let shown = view(tags, placeholder, mode, choices, Activity::Failed, Some(message.chars().take(180).collect()));
            if let Err(error) = picker::update(flow.session, &shown) { host::log(2, &format!("Remote SSH: {error}")); }
        }
    }
    /// What can be done about a connection that failed.
    fn failed_choices() -> Vec<Choice> {
        vec![choice("retry", "Retry", None, None),
            choice("key", "Choose Key File…", Some("Use one specific private key"), None)]
    }
    fn auth_view(profile: &Profile) -> View {
        let label = profile.label();
        view(&[TITLE, &label], "Select private key", Mode::Choose, vec![
            choice("default", "Default Location", Some("Keys in ~/.ssh and your SSH config"), Some("Recommended")),
            choice("key", "Choose Key File…", Some("Use one specific private key"), None),
        ], Activity::Idle, None)
    }

    fn accepted(&mut self, accepted: Accepted) {
        let Some(flow) = &self.flow else { return };
        if flow.session != accepted.session { return; }
        let step = std::mem::replace(&mut self.flow.as_mut().unwrap().step, Step::Host);
        match step {
            Step::Host => match Profile::from_input(&accepted.text) {
                Ok(profile) if self.active(&profile).is_some() => {
                    // Nothing to choose: say so where the host was typed.
                    self.flow.as_mut().unwrap().step = Step::Host;
                    self.note("Already connected to this host", &[TITLE], "user@host[:port]", Mode::Text, vec![]);
                }
                Ok(profile) => { let shown = Self::auth_view(&profile); self.show(Step::Auth(profile), shown); }
                Err(error) => {
                    self.flow.as_mut().unwrap().step = Step::Host;
                    self.note(&error, &[TITLE], "user@host[:port]", Mode::Text, vec![]);
                }
            },
            Step::Auth(profile) => match accepted.choice.as_deref() {
                Some("default") => self.connect_in_palette(profile),
                Some("key") => match picker::pick_file("Select Private Key") {
                    Ok(request) => self.flow.as_mut().unwrap().step = Step::Key { profile, request },
                    Err(error) => {
                        self.flow.as_mut().unwrap().step = Step::Auth(profile);
                        self.note(&error, &[TITLE], "Select private key", Mode::Choose, vec![]);
                    }
                },
                _ => self.flow.as_mut().unwrap().step = Step::Auth(profile),
            },
            Step::Key { profile, request } => self.flow.as_mut().unwrap().step = Step::Key { profile, request },
            Step::Connecting { client, profile, failed } => {
                // A failed attempt offers to try again or to use another key; while one runs nothing is chosen.
                match (failed, accepted.choice.as_deref()) {
                    (true, Some("retry")) => self.connect_in_palette(profile),
                    (true, Some("key")) => match picker::pick_file("Select Private Key") {
                        Ok(request) => self.flow.as_mut().unwrap().step = Step::Key { profile, request },
                        Err(error) => {
                            self.flow.as_mut().unwrap().step = Step::Connecting { client, profile, failed };
                            self.note(&error, &[TITLE], "Connection failed", Mode::Choose, Self::failed_choices());
                        }
                    },
                    _ => self.flow.as_mut().unwrap().step = Step::Connecting { client, profile, failed },
                }
            }
            Step::Menu(index) => {
                self.flow.as_mut().unwrap().step = Step::Menu(index);
                match accepted.choice.as_deref() {
                    Some("reconnect") => {
                        let profile = self.clients[index].connected.clone().unwrap_or_else(|| self.clients[index].profile.clone());
                        self.connect_in_palette(profile);
                    }
                    Some("disconnect") => {
                        let _ = self.clients[index].disconnect(true);
                        self.close();
                    }
                    Some("new") => self.begin(),
                    _ => {}
                }
            }
        }
    }

    /// Start connecting while the palette stays open to show how it goes.
    fn connect_in_palette(&mut self, profile: Profile) {
        let label = profile.label();
        match self.connect(profile.clone()) {
            Ok(client) => {
                let shown = view(&[TITLE, &label], "Connecting…", Mode::Choose, vec![], Activity::Busy, None);
                self.show(Step::Connecting { client, profile, failed: false }, shown);
            }
            Err(error) => {
                self.flow.as_mut().unwrap().step = Step::Auth(profile.clone());
                self.note(&error, &[TITLE, &label], "Select private key", Mode::Choose, Self::auth_view(&profile).choices);
            }
        }
    }

    /// The connection already made or being made to this host, if any.
    fn active(&self, profile: &Profile) -> Option<usize> {
        let identity = profile.identity();
        self.clients.iter().position(|client| client.process.is_some() && client.connected.as_ref()
            .is_some_and(|active| active.identity() == identity))
    }

    /// Begin one connection on a free slot, or say why it cannot.
    fn connect(&mut self, profile: Profile) -> Result<usize, String> {
        profile.validate()?;
        if let Some(index) = self.active(&profile) {
            self.selected = index;
            return Err("This SSH environment is already connected".into());
        }
        let index = self.candidate()?;
        self.selected = index;
        self.clients[index].profile = profile;
        if let Err(error) = self.clients[index].connect() {
            self.clients[index].fail(&error);
            return Err(error);
        }
        Ok(index)
    }

    fn cancelled(&mut self, session: u64) {
        if self.flow.as_ref().is_none_or(|flow| flow.session != session) { return; }
        if let Some(Flow { step: Step::Connecting { client, .. }, .. }) = &self.flow {
            let client = *client;
            if self.clients[client].scope.is_none() { let _ = self.clients[client].disconnect(false); }
        }
        self.flow = None;
    }

    fn close(&mut self) {
        if let Some(flow) = self.flow.take() {
            let _ = picker::close(flow.session);
        }
    }

    fn open_menu(&mut self, index: usize) {
        let client = &self.clients[index];
        let profile = client.connected.as_ref().unwrap_or(&client.profile);
        let label = profile.label();
        let live = client.scope.is_some() || client.process.is_some();
        let mut choices = Vec::new();
        if !live { choices.push(choice("reconnect", "Reconnect", None, None)); }
        if live { choices.push(choice("disconnect", "Disconnect", None, None)); }
        choices.push(choice("new", "Connect to New Host…", None, None));
        let shown = view(&[TITLE, &label], "Select an action", Mode::Choose, choices,
            if client.failure.is_some() { Activity::Failed } else { Activity::Idle }, client.failure.clone());
        self.open(Step::Menu(index), shown);
    }

    pub(super) fn palette(&mut self, event: picker::Event) {
        self.start();
        match event {
            picker::Event::Accepted(accepted) => self.accepted(accepted),
            picker::Event::Cancelled(session) => self.cancelled(session),
            picker::Event::Entry(id) => self.entry(&id),
            picker::Event::File(picked) => self.picked(picked),
        }
        self.sync();
    }

    /// The file dialog for a private key was answered.
    fn picked(&mut self, picked: picker::Picked) {
        let Some(flow) = &mut self.flow else { return };
        let Step::Key { request, .. } = &flow.step else { return };
        if *request != picked.request { return; }
        let Step::Key { profile, .. } = std::mem::replace(&mut flow.step, Step::Host) else { return };
        match picked.path {
            Some(path) => self.connect_in_palette(Profile { key: path, ..profile }),
            None => {
                // The dialog was dismissed: back to choosing how.
                let shown = Self::auth_view(&profile);
                self.show(Step::Auth(profile), shown);
            }
        }
    }

    /// A row the palette listed for this extension was chosen.
    fn entry(&mut self, id: &str) {
        if let Some(index) = id.strip_prefix("disconnect:").and_then(|index| index.parse::<usize>().ok()) {
            if let Some(client) = self.clients.get_mut(index) { let _ = client.disconnect(true); }
        } else if let Some(identity) = id.strip_prefix("host:") {
            if let Some(profile) = self.saved.iter().find(|profile| profile.identity() == identity).cloned() {
                // Already connected: the host is just the one to work with, and the palette is done.
                if let Some(index) = self.active(&profile) {
                    self.selected = index;
                    return;
                }
                // Choosing a host closed the palette; it opens again to show how the connection goes.
                let label = profile.label();
                let shown = view(&[TITLE, &label], "Connecting…", Mode::Choose, vec![], Activity::Busy, None);
                self.open(Step::Auth(profile.clone()), shown);
                if self.flow.is_some() {
                    self.connect_in_palette(profile);
                } else if let Err(error) = self.connect(profile) {
                    host::log(2, &format!("Remote SSH: {error}"));
                }
            }
        }
    }

    /// Follow the connection the palette is showing: leave when it connected, say why when it failed.
    fn sync(&mut self) {
        if let Some(Flow { step: Step::Connecting { client, profile, failed }, .. }) = &self.flow {
            let (client, mut profile, failed) = (*client, profile.clone(), *failed);
            if self.clients[client].scope.is_some() {
                profile.root = self.clients[client].canonical_root.clone();
                self.remember(profile);
                self.close();
            } else if let (Some(why), false) = (self.clients[client].failure.clone(), failed) {
                let label = profile.label();
                self.note(&why, &[TITLE, &label], "Connection failed", Mode::Choose, Self::failed_choices());
                if let Some(flow) = &mut self.flow { flow.step = Step::Connecting { client, profile, failed: true }; }
            }
        }
        self.publish();
    }

    fn message_owner(&self, message: &Inbound) -> Option<usize> {
        self.clients.iter().position(|client| match message {
            Inbound::TimerElapsed(timer) => client.heartbeat == Some(*timer),
            Inbound::ProcessOutput(output) => client.process == Some(output.process),
            Inbound::ProcessExited(exit) => client.process == Some(exit.process),
            Inbound::UiEvent(event) => client.controls.contains_key(&event.node),
            _ => false,
        })
    }
    pub(super) fn message(&mut self, message: Inbound) {
        self.start();
        if let Inbound::RunCommand(command) = &message {
            if command.command == "loog.remote.ssh.connect" {
                self.begin();
                return;
            }
            if command.command == "loog.remote.ssh.disconnect" {
                if let Err(error) = self.clients[self.selected].disconnect(true) { host::log(2, &format!("Remote SSH: {error}")); }
                self.sync();
                return;
            }
        }
        let Some(index) = self.message_owner(&message) else { return };
        if let Inbound::UiEvent(event) = &message {
            if matches!(event.payload, UiEventPayload::Click) && self.clients[index].controls.get(&event.node) == Some(&"menu") {
                self.open_menu(index);
                return;
            }
        }
        if let Err(error) = self.clients[index].message(message) { self.clients[index].fail(&error); }
        self.sync();
    }
    fn event_owner(&self, event: &Event) -> Option<usize> {
        self.clients.iter().position(|client| match event {
            Event::Reply { ticket, .. } => client.tickets.contains_key(ticket),
            Event::Delivery { event: api::Delivery::Request(request) } => client.scope == Some(request.scope),
            Event::Delivery { event: api::Delivery::Release { scope, .. } } => client.scope == Some(*scope),
            Event::Delivery { event: api::Delivery::Cancel { request } } => client.pending.values().any(|pending|
                matches!(pending, Pending::Operation(operation) if operation.id == *request)),
            _ => false,
        })
    }
    pub(super) fn workspace(&mut self, event: Event) {
        self.start();
        if let Event::Restore { bookmark } = &event {
            let result = (|| {
                let profile: Profile = serde_json::from_str(&bookmark.state).map_err(|_| "Invalid saved connection profile".to_owned())?;
                profile.validate()?;
                let expected = api::ResourceRef::provided(api::EnvironmentKey::new(ID, profile.identity())?, profile.root.clone())?;
                if expected != bookmark.resource { return Err("Saved connection identity does not match its profile".into()); }
                if let Some(client) = self.clients.iter().find(|client| client.connected.as_ref()
                    .is_some_and(|active| active.identity() == profile.identity())) {
                    if client.connected.as_ref().unwrap().root != profile.root && client.canonical_root != profile.root {
                        return Err("This SSH environment already has another folder connected".into());
                    }
                    return Ok(());
                }
                self.connect(profile).map(|_| ())
            })();
            if let Err(error) = result { host::log(2, &format!("Remote SSH: {error}")); }
            self.sync();
            return;
        }
        let Some(index) = self.event_owner(&event) else { return };
        if let Err(error) = self.clients[index].workspace(event) { self.clients[index].fail(&error); }
        self.sync();
    }
    pub(super) fn flush(&mut self) {
        for client in &mut self.clients {
            if let Err(error) = client.flush() { client.fail(&error); }
        }
    }
}

/// How a remote folder is named to the person.
fn folder(root: &str) -> String {
    if root == protocol::HOME { "Home folder".into() } else { root.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_scopes_tickets_timers_and_cancellation_have_one_owner() {
        let mut connections = Connections::default();
        for index in 0..MAX_CONNECTIONS {
            assert_eq!(connections.candidate().unwrap(), index);
            let client = &mut connections.clients[index];
            client.process = Some(index as u64 + 10);
            client.scope = Some(index as u64 + 20);
            client.heartbeat = Some(index as u64 + 30);
            client.tickets.insert(index as u64 + 40, Ticket::Other);
            client.pending.insert(1, Pending::Operation(api::Request {
                id: index as u64 + 50, environment: index as u64 + 60,
                scope: index as u64 + 20, root: api::ProviderPath::new("/same").unwrap(),
                excluded_roots: vec![], operation: api::Operation::Stat { path: api::ProviderPath::new("/same/file").unwrap() },
            }));
        }
        assert!(connections.candidate().is_err());
        for index in 0..MAX_CONNECTIONS {
            assert_eq!(connections.message_owner(&Inbound::TimerElapsed(index as u64 + 30)), Some(index));
            assert_eq!(connections.event_owner(&Event::Reply { ticket: index as u64 + 40, result: Ok(Reply::Done) }), Some(index));
            assert_eq!(connections.event_owner(&Event::Delivery { event: api::Delivery::Cancel { request: index as u64 + 50 } }), Some(index));
            assert_eq!(connections.event_owner(&Event::Delivery { event: api::Delivery::Release {
                scope: index as u64 + 20, capability: api::Capability::Process, handle: 1,
            } }), Some(index));
        }
        assert_eq!(connections.message_owner(&Inbound::TimerElapsed(999)), None);
        connections.clients[2].process = None;
        assert!(connections.candidate().is_err(), "pending receipts prevent reusing an old connection slot");
        connections.clients[2].tickets.clear();
        connections.clients[2].pending.clear();
        assert_eq!(connections.candidate().unwrap(), 2);
    }
    #[test]
    fn a_multi_line_ssh_failure_becomes_one_short_line() {
        let text = "No ED25519 host key is known.\r\nHost key verification failed.\r\n";
        assert_eq!(one_line(text), "No ED25519 host key is known. Host key verification failed.");
        assert!(!one_line(&"word ".repeat(100)).contains('\n'));
        assert_eq!(one_line(&"é".repeat(300)).chars().count(), 180);
    }
    #[test]
    fn a_remote_folder_is_named_for_the_person() {
        assert_eq!(folder(protocol::HOME), "Home folder");
        assert_eq!(folder("/srv/app"), "/srv/app");
    }
}

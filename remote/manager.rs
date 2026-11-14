//! Bounded independent connections and the command palette flow that opens them.
//! Host tickets, scopes and process IDs route to their owner; Agent handles are
//! meaningful only inside that owner. The palette shows only words and ids this
//! component supplies; the application never learns what they stand for.
use super::*;
use crate::transport::{SAVED, TITLE};
use picker::{Accepted, Activity, Choice, Entry, Mode, View};

const MAX_CONNECTIONS: usize = 4;
const MAX_SAVED: usize = 32;
const STORAGE_KEY: &str = "hosts";

/// Where the person is in the palette.
enum Step {
    /// A host is being typed.
    #[cfg(transport = "ssh")]
    Host,
    /// The host is known; how to authenticate is being chosen.
    #[cfg(transport = "ssh")]
    Auth(Profile),
    /// The system file dialog is open to choose a private key.
    #[cfg(transport = "ssh")]
    Key { profile: Profile, request: u64 },
    /// The list of targets is being read.
    #[cfg(not(transport = "ssh"))]
    List,
    /// The targets are listed and one is being chosen.
    #[cfg(not(transport = "ssh"))]
    Pick,
    /// A connection could not be started; the choices offer another try.
    #[cfg(not(transport = "ssh"))]
    Failed(Profile),
    /// One connection is being made, or has failed.
    Connecting { client: usize, profile: Profile, failed: bool },
    /// The folder to open is being chosen from the tree.
    Browse(Box<BrowseStep>),
    /// The actions of one connection.
    Menu(usize),
}
/// A connection whose Agent is ready, while the person chooses which folder it opens.
struct BrowseStep {
    client: usize,
    browse: folders::Browse,
    /// The login folder has been asked for, to start from.
    home_pending: bool,
    /// A folder has been chosen and is being opened.
    opening: bool,
    /// The picker as last drawn, so an unchanged picker is not drawn again.
    last_view: String,
    /// The client's mounts when the picker opened; the picker is done when it goes up.
    mounts: u32,
}
struct Flow {
    session: u64,
    step: Step,
}

/// The process that prints the list of targets, and what it has printed.
#[cfg(not(transport = "ssh"))]
struct Lister {
    process: u64,
    output: Vec<u8>,
    errors: Vec<u8>,
}

pub(super) struct Connections {
    clients: Vec<Client>,
    selected: usize,
    flow: Option<Flow>,
    saved: Vec<Profile>,
    started: bool,
    /// What was last published as palette entries, to publish again only a change.
    published: Vec<(String, String, Option<String>, String)>,
    #[cfg(not(transport = "ssh"))]
    lister: Option<Lister>,
}
impl Default for Connections {
    fn default() -> Self {
        Self { clients: vec![Client::default()], selected: 0, flow: None, saved: Vec::new(),
            started: false, published: Vec::new(),
            #[cfg(not(transport = "ssh"))]
            lister: None }
    }
}

/// The most bytes of a note the palette takes (it takes 200); text in other scripts is wider than its length.
const NOTE_BYTES: usize = 190;

/// The words of `text` on one line, cut to what the palette shows. Channels report failures
/// over several lines, often a warning first and the cause last, so a long text keeps its end.
fn one_line(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control() || c.is_whitespace()).collect();
    let line = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.len() <= NOTE_BYTES { return line; }
    let mut start = line.len() - (NOTE_BYTES - 3);
    while !line.is_char_boundary(start) { start += 1; }
    format!("...{}", &line[start..])
}

/// What a failure means to the person, when it is a failure they can fix.
fn explain(line: String) -> String {
    // The agent loader says in its own words why it could not go on.
    if let Some((_, reason)) = line.rsplit_once(crate::protocol::ERROR_PREFIX.trim_end()) {
        return reason.trim().to_owned();
    }
    let lower = line.to_lowercase();
    if lower.contains("execvpe(sh)") || (lower.contains("exec") && lower.contains("\"sh\"") && lower.contains("not found")) {
        return "The target has no POSIX shell (sh) to start the agent".into();
    }
    line
}

fn view(tags: &[&str], placeholder: &str, mode: Mode, choices: Vec<Choice>, activity: Activity, note: Option<String>) -> View {
    View { tags: tags.iter().map(|tag| (*tag).to_owned()).collect(), placeholder: placeholder.into(),
        mode, choices, activity, note: note.map(|note| explain(one_line(&note))), text: None, selected: None }
}
fn choice(id: &str, title: &str, detail: Option<&str>, trailing: Option<&str>) -> Choice {
    Choice { id: id.into(), title: title.into(), detail: detail.map(Into::into), group: None,
        trailing: trailing.map(Into::into), depth: 0, expanded: None, icon: None, action: None, dim: false }
}

/// The most rows of the folder tree the palette is given; a longer tree is cut with a note.
const MAX_TREE_ROWS: usize = 960;

/// The palette view of a folder picker: the recent folders, then the tree from `/`. `text` puts
/// that text in the field and scrolls to the folder it names.
fn folder_view(browse: &folders::Browse, label: &str, busy: bool, text: Option<String>, scroll: bool) -> View {
    let mut choices: Vec<Choice> = Vec::new();
    for (id, path) in folders::recent_ids(&browse.recent) {
        choices.push(Choice { group: Some("RECENT".into()), icon: Some(picker::Icon::Folder), ..choice(&id, &path, None, None) });
    }
    let rows = browse.rows();
    let cut = rows.len() > MAX_TREE_ROWS;
    for row in rows.into_iter().take(MAX_TREE_ROWS) {
        choices.push(Choice {
            group: Some("FILE TREE".into()),
            depth: row.depth,
            expanded: row.expanded,
            icon: Some(if row.dir { picker::Icon::Folder } else { picker::Icon::File }),
            action: (row.dir && !row.note).then(|| "Open this folder".to_owned()),
            dim: row.dim,
            ..choice(&row.id, &row.title, None, None)
        });
    }
    if cut {
        choices.push(Choice { group: Some("FILE TREE".into()), icon: Some(picker::Icon::File), dim: true,
            ..choice("more:/", "…", Some("more rows are not shown"), None) });
    }
    let (activity, note) = match (&browse.error, busy || !browse.loading.is_empty()) {
        (Some(why), _) => (Activity::Failed, Some(why.clone())),
        (None, true) => (Activity::Busy, None),
        (None, false) => (Activity::Idle, None),
    };
    let selected = if scroll || text.is_some() { browse.selected() } else { None };
    View { tags: vec![TITLE.to_owned(), label.to_owned()], placeholder: "Enter a folder path".into(),
        mode: Mode::Browse, choices, activity, note: note.map(|note| explain(one_line(&note))), text, selected }
}

impl Connections {
    fn candidate(&mut self) -> Result<usize, String> {
        if let Some(index) = self.clients.iter().position(|client| {
            client.process.is_none() && client.environment.is_none()
                && client.tickets.is_empty() && client.pending.is_empty()
        }) { return Ok(index); }
        if self.clients.len() >= MAX_CONNECTIONS {
            return Err("At most four environments can be connected; disconnect one first".into());
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
                None, SAVED.to_owned()));
        }
        if entries == self.published { return; }
        let list: Vec<Entry> = entries.iter().map(|(id, title, detail, category)| Entry {
            id: id.clone(), title: title.clone(), detail: detail.clone(), category: category.clone() }).collect();
        match picker::publish_entries(&list) {
            Ok(()) => self.published = entries,
            Err(error) => host::log(2, &format!("{TITLE}: {error}")),
        }
    }

    /// Remember a host that connected, most recent first.
    fn remember(&mut self, profile: Profile) {
        self.saved.retain(|saved| saved.identity() != profile.identity());
        self.saved.insert(0, profile);
        self.saved.truncate(MAX_SAVED);
        if let Err(error) = host::storage_set(StorageScope::User, STORAGE_KEY, &serde_json::to_string(&self.saved).unwrap()) {
            host::log(2, &format!("{TITLE}: {}", error.message));
        }
    }

    /// Open the palette on the first step of a new connection.
    #[cfg(transport = "ssh")]
    fn begin(&mut self) {
        let shown = view(&[TITLE], "user@host[:port]", Mode::Text, vec![], Activity::Idle, None);
        self.open(Step::Host, shown);
    }
    /// Open the palette on the list of targets, which is read first.
    #[cfg(not(transport = "ssh"))]
    fn begin(&mut self) {
        self.stop_listing();
        let shown = view(&[TITLE], "Listing…", Mode::Choose, vec![], Activity::Busy, None);
        self.open(Step::List, shown);
        if self.flow.is_none() { return; }
        let (program, args) = listing::command();
        match tools::launch(program, &args) {
            Ok(process) => {
                host::log(1, &format!("{TITLE}: listing started with {program}"));
                self.lister = Some(Lister { process, output: Vec::new(), errors: Vec::new() });
            }
            Err(error) => self.listing_failed(&error),
        }
    }
    #[cfg(not(transport = "ssh"))]
    fn stop_listing(&mut self) {
        if let Some(lister) = self.lister.take() { host::process_kill(lister.process); }
    }
    /// The list could not be read: say so where it would have been.
    #[cfg(not(transport = "ssh"))]
    fn listing_failed(&mut self, why: &str) {
        self.lister = None;
        if let Some(flow) = &mut self.flow { flow.step = Step::List; }
        // The reason is the whole message here, so it is said in the field, not only on the mark.
        let reason = explain(one_line(why));
        self.note(why, &[TITLE], &reason, Mode::Choose, vec![]);
    }
    /// The program that lists the targets spoke or ended. True when the message was its own.
    #[cfg(not(transport = "ssh"))]
    fn lister_message(&mut self, message: &Inbound) -> bool {
        let Some(lister) = &mut self.lister else { return false };
        match message {
            Inbound::ProcessOutput(output) if output.process == lister.process => {
                let (kept, limit) = if matches!(output.channel, OutputChannel::Stderr) { (&mut lister.errors, 4096) }
                    else { (&mut lister.output, listing::MAX_BYTES) };
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&output.bytes[..output.bytes.len().min(room)]);
                true
            }
            Inbound::ProcessExited(exit) if exit.process == lister.process => {
                let Some(lister) = self.lister.take() else { return true };
                let targets = listing::parse(&lister.output);
                host::log(1, &format!("{TITLE}: listing ended with code {:?}, {} bytes, {} targets",
                    exit.code, lister.output.len(), targets.len()));
                if exit.error.is_some() || exit.code != Some(0) && targets.is_empty() {
                    let said = listing::decode(&lister.errors);
                    let why = if said.trim().is_empty() { format!("{} is not available. Install it and make sure it is running", crate::transport::LISTER) } else { said };
                    self.listing_failed(&why);
                } else if targets.is_empty() {
                    self.listing_failed(crate::transport::NONE_FOUND);
                } else if self.flow.as_ref().is_some_and(|flow| matches!(flow.step, Step::List)) {
                    let choices = targets.iter().map(|target| choice(&target.name, &target.name, Some(&target.detail), None)).collect();
                    let shown = view(&[TITLE], crate::transport::PICK, Mode::Choose, choices, Activity::Idle, None);
                    self.show(Step::Pick, shown);
                }
                true
            }
            _ => false,
        }
    }
    fn open(&mut self, step: Step, shown: View) {
        match picker::open(&shown) {
            Ok(session) => self.flow = Some(Flow { session, step }),
            Err(error) => host::log(2, &format!("{TITLE}: {error}")),
        }
    }
    fn show(&mut self, step: Step, shown: View) {
        let Some(flow) = &mut self.flow else { return };
        if let Err(error) = picker::update(flow.session, &shown) { host::log(2, &format!("{TITLE}: {error}")); }
        flow.step = step;
    }
    /// Say something in the open palette without changing the step.
    fn note(&mut self, message: &str, tags: &[&str], placeholder: &str, mode: Mode, choices: Vec<Choice>) {
        if let Some(flow) = &self.flow {
            let shown = view(tags, placeholder, mode, choices, Activity::Failed, Some(message.to_owned()));
            if let Err(error) = picker::update(flow.session, &shown) { host::log(2, &format!("{TITLE}: {error}")); }
        }
    }
    /// What can be done about a connection that failed.
    #[cfg(transport = "ssh")]
    fn failed_choices() -> Vec<Choice> {
        vec![choice("retry", "Retry", None, None),
            choice("key", "Choose Key File…", Some("Use one specific private key"), None)]
    }
    #[cfg(not(transport = "ssh"))]
    fn failed_choices() -> Vec<Choice> {
        vec![choice("retry", "Retry", None, None)]
    }
    #[cfg(transport = "ssh")]
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
        // Every arm below puts a step back; this one only stands in while the step is taken.
        let step = std::mem::replace(&mut self.flow.as_mut().unwrap().step, Step::Menu(usize::MAX));
        match step {
            #[cfg(transport = "ssh")]
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
            #[cfg(transport = "ssh")]
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
            #[cfg(transport = "ssh")]
            Step::Key { profile, request } => self.flow.as_mut().unwrap().step = Step::Key { profile, request },
            #[cfg(not(transport = "ssh"))]
            Step::List => self.flow.as_mut().unwrap().step = Step::List,
            #[cfg(not(transport = "ssh"))]
            Step::Pick => match accepted.choice.as_deref().map(Profile::for_target) {
                Some(Ok(profile)) => self.connect_in_palette(profile),
                Some(Err(error)) => {
                    self.flow.as_mut().unwrap().step = Step::Pick;
                    self.note(&error, &[TITLE], crate::transport::PICK, Mode::Choose, vec![]);
                }
                None => self.flow.as_mut().unwrap().step = Step::Pick,
            },
            #[cfg(not(transport = "ssh"))]
            Step::Failed(profile) => match accepted.choice.as_deref() {
                Some("retry") => self.connect_in_palette(profile),
                _ => self.flow.as_mut().unwrap().step = Step::Failed(profile),
            },
            Step::Connecting { client, profile, failed } => {
                // A failed attempt offers to try again or to use another key; while one runs nothing is chosen.
                match (failed, accepted.choice.as_deref()) {
                    (true, Some("retry")) => self.connect_in_palette(profile),
                    #[cfg(transport = "ssh")]
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
            Step::Browse(step) => self.browse_accepted(step, accepted),
            Step::Menu(index) => {
                self.flow.as_mut().unwrap().step = Step::Menu(index);
                match accepted.choice.as_deref() {
                    Some("folder") => self.change_folder(index),
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

    /// Choose another folder for a connection. A connection that is up stays up and the picker
    /// opens on it; one that is not is made again and asks which folder to open.
    fn change_folder(&mut self, index: usize) {
        let client = &mut self.clients[index];
        if client.process.is_some() && client.environment.is_some() && client.scope.is_some() && client.agent_root.is_some() {
            client.choose_folder = true;
            self.begin_browse(index);
            return;
        }
        let mut profile = self.clients[index].connected.clone().unwrap_or_else(|| self.clients[index].profile.clone());
        profile.root = protocol::HOME.into();
        let _ = self.clients[index].disconnect(false);
        self.connect_in_palette_at(Some(index), profile);
    }

    /// The Agent is ready and no folder is chosen yet: the palette becomes the folder picker,
    /// which starts at the login folder.
    fn begin_browse(&mut self, client: usize) {
        let Some(profile) = self.clients[client].connected.clone() else { return };
        let mut step = BrowseStep { client, browse: folders::Browse::new(profile.recent.clone()),
            home_pending: true, opening: false, last_view: String::new(), mounts: self.clients[client].mounts };
        if let Err(error) = self.clients[client].browse(protocol::HOME) {
            step.browse.error = Some(error);
            step.home_pending = false;
        }
        let shown = folder_view(&step.browse, &profile.label(), true, Some(String::new()), false);
        self.show(Step::Browse(Box::new(step)), shown);
    }

    /// Ask for the folders that are open and not yet read, and draw the picker again. `text`
    /// replaces what the field holds; `scroll` brings the folder it names into view.
    fn refresh_browse(&mut self, text: Option<String>, scroll: bool) {
        let Some(Flow { session, step: Step::Browse(step) }) = &mut self.flow else { return };
        for path in step.browse.reveal() {
            if let Err(error) = self.clients[step.client].browse(&path) {
                step.browse.loading.remove(&path);
                step.browse.error = Some(error);
            }
        }
        let label = self.clients[step.client].connected.as_ref().map(Profile::label).unwrap_or_default();
        let shown = folder_view(&step.browse, &label, step.opening, text, scroll);
        // Redrawing the same list makes it flicker and drops the scroll position.
        let drawn = format!("{shown:?}");
        if drawn == step.last_view { return; }
        step.last_view = drawn;
        if let Err(error) = picker::update(*session, &shown) { host::log(2, &format!("{TITLE}: {error}")); }
    }

    /// Take what the Agent listed since the last look, and open the folder it was asked for.
    fn pump_browse(&mut self) {
        let Some(Flow { step: Step::Browse(step), .. }) = &mut self.flow else { return };
        let client = &mut self.clients[step.client];
        let results = std::mem::take(&mut client.browsed);
        let refused = client.mount_error.take();
        if results.is_empty() && refused.is_none() { return; }
        let mut text = None;
        for (requested, result) in results {
            if requested == protocol::HOME && step.home_pending {
                // The login folder is where the field starts.
                step.home_pending = false;
                match result {
                    Ok((canonical, listing)) => {
                        step.browse.apply(&requested, &canonical, Ok(listing));
                        text = Some(if canonical == "/" { canonical } else { format!("{canonical}/") });
                    }
                    Err(why) => {
                        step.browse.apply(&requested, "/", Err(why));
                        text = Some("/".to_owned());
                    }
                }
                step.browse.text = text.clone().unwrap_or_default();
            } else {
                let canonical = result.as_ref().map_or_else(|_| requested.clone(), |(path, _)| path.clone());
                step.browse.apply(&requested, &canonical, result.map(|(_, listing)| listing));
            }
        }
        if let Some(why) = refused {
            step.opening = false;
            step.browse.error = Some(why);
        }
        let scroll = text.is_some();
        self.refresh_browse(text, scroll);
    }

    /// Open `path` as the folder of the connection being made.
    fn open_chosen(&mut self, path: &str) {
        let Some(Flow { step: Step::Browse(step), .. }) = &mut self.flow else { return };
        let path = folders::normalize(path);
        step.browse.error = None;
        step.opening = true;
        if let Err(error) = self.clients[step.client].open_folder(&path) {
            step.opening = false;
            step.browse.error = Some(error);
        }
        self.refresh_browse(None, false);
    }

    /// Enter, a click or the mark on a row of the folder picker.
    fn browse_accepted(&mut self, step: Box<BrowseStep>, accepted: Accepted) {
        let busy = step.opening;
        self.flow.as_mut().unwrap().step = Step::Browse(step);
        // While a folder is being opened nothing else is chosen.
        if busy { return; }
        let choice = accepted.choice.as_deref();
        match (accepted.trigger, choice) {
            (picker::Trigger::Enter, _) => self.open_chosen(&accepted.text),
            (picker::Trigger::Action, Some(id)) if !id.starts_with("more:") => self.open_chosen(id),
            (picker::Trigger::Click, Some(id)) if id.starts_with("recent:") => self.open_chosen(&id["recent:".len()..]),
            (picker::Trigger::Click, Some(id)) if !id.starts_with("more:") => {
                let Some(Flow { step: Step::Browse(step), .. }) = &mut self.flow else { return };
                let dir = step.browse.is_dir(id);
                let text = step.browse.click(id, dir);
                self.refresh_browse(Some(text), true);
            }
            _ => {}
        }
    }

    /// The text of the folder picker's field changed.
    fn typed(&mut self, input: picker::Input) {
        let Some(Flow { session, step: Step::Browse(step) }) = &mut self.flow else { return };
        if *session != input.session { return; }
        // The field echoing a click's own text is not typing.
        if step.browse.text != input.text { step.browse.closed = None; }
        step.browse.text = input.text;
        // The picker activates the folder the field names when it is in the list, and
        // moves the list only as far as that takes.
        self.refresh_browse(None, true);
    }

    /// Tab on a row of the folder picker.
    fn completed(&mut self, accepted: Accepted) {
        let Some(Flow { session, step: Step::Browse(step) }) = &mut self.flow else { return };
        if *session != accepted.session { return; }
        let Some(id) = accepted.choice.as_deref().filter(|id| !id.starts_with("more:")) else { return };
        let id = id.strip_prefix("recent:").unwrap_or(id);
        let text = step.browse.completion(id);
        step.browse.text = text.clone();
        self.refresh_browse(Some(text), true);
    }

    /// Start connecting while the palette stays open to show how it goes.
    fn connect_in_palette(&mut self, profile: Profile) {
        self.connect_in_palette_at(None, profile);
    }

    fn connect_in_palette_at(&mut self, slot: Option<usize>, profile: Profile) {
        let label = profile.label();
        match self.connect_at(slot, profile.clone()) {
            Ok(client) => {
                let shown = view(&[TITLE, &label], "Connecting…", Mode::Choose, vec![], Activity::Busy, None);
                self.show(Step::Connecting { client, profile, failed: false }, shown);
            }
            Err(error) => {
                #[cfg(transport = "ssh")]
                {
                    self.flow.as_mut().unwrap().step = Step::Auth(profile.clone());
                    self.note(&error, &[TITLE, &label], "Select private key", Mode::Choose, Self::auth_view(&profile).choices);
                }
                #[cfg(not(transport = "ssh"))]
                {
                    self.flow.as_mut().unwrap().step = Step::Failed(profile);
                    self.note(&error, &[TITLE, &label], "Connection failed", Mode::Choose, Self::failed_choices());
                }
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
        self.connect_at(None, profile)
    }

    /// [`Self::connect`], on the slot `slot` when given: a connection made again to choose
    /// another folder keeps its own slot instead of taking a free one, which a slot that
    /// still waits for the answer to its last request is not.
    fn connect_at(&mut self, slot: Option<usize>, profile: Profile) -> Result<usize, String> {
        profile.validate()?;
        if let Some(index) = self.active(&profile) {
            self.selected = index;
            return Err("Already connected".into());
        }
        let index = match slot {
            Some(index) if index < self.clients.len() => index,
            _ => self.candidate()?,
        };
        self.selected = index;
        let mut profile = profile;
        // A host typed again still has the folders opened on it before.
        if profile.recent.is_empty() {
            if let Some(saved) = self.saved.iter().find(|saved| saved.identity() == profile.identity()) {
                profile.recent = saved.recent.clone();
            }
        }
        // A host with no folder yet asks the person which to open; one with a folder opens it.
        self.clients[index].choose_folder = profile.root == protocol::HOME;
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
        // Leaving the folder picker ends the connection that was waiting for a folder.
        if let Some(Flow { step: Step::Browse(step), .. }) = &self.flow {
            let client = step.client;
            if self.clients[client].scope.is_none() {
                let _ = self.clients[client].disconnect(false);
            } else {
                // A connection that was up stays up with the folder it had.
                self.clients[client].choose_folder = false;
                self.clients[client].awaiting_folder = false;
            }
        }
        #[cfg(not(transport = "ssh"))]
        self.stop_listing();
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
        if client.scope.is_some() { choices.push(choice("folder", crate::transport::OPEN_FOLDER, None, None)); }
        if !live { choices.push(choice("reconnect", "Reconnect", None, None)); }
        if live { choices.push(choice("disconnect", "Disconnect", None, None)); }
        choices.push(choice("new", crate::transport::NEW, None, None));
        let shown = view(&[TITLE, &label], "Select an action", Mode::Choose, choices,
            if client.failure.is_some() { Activity::Failed } else { Activity::Idle }, client.failure.clone());
        self.open(Step::Menu(index), shown);
    }

    pub(super) fn palette(&mut self, event: picker::Event) {
        self.start();
        match event {
            picker::Event::Accepted(accepted) => self.accepted(accepted),
            picker::Event::Input(input) => self.typed(input),
            picker::Event::Complete(accepted) => self.completed(accepted),
            picker::Event::Cancelled(session) => self.cancelled(session),
            picker::Event::Entry(id) => self.entry(&id),
            #[cfg(transport = "ssh")]
            picker::Event::File(picked) => self.picked(picked),
            // Only a private key is asked of the system dialog, and only the SSH package asks.
            #[cfg(not(transport = "ssh"))]
            picker::Event::File(_) => {}
        }
        self.sync();
    }

    /// The file dialog for a private key was answered.
    #[cfg(transport = "ssh")]
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
            if let Some(mut profile) = self.saved.iter().find(|profile| profile.identity() == identity).cloned() {
                // A host that is used again asks which folder to open; the ones opened before are
                // offered first in the picker.
                profile.root = protocol::HOME.into();
                // Already connected: the host is just the one to work with, and the palette is done.
                if let Some(index) = self.active(&profile) {
                    self.selected = index;
                    return;
                }
                // Choosing a host closed the palette; it opens again to show how the connection goes.
                let label = profile.label();
                let shown = view(&[TITLE, &label], "Connecting…", Mode::Choose, vec![], Activity::Busy, None);
                // The step stands in until the connection starts and sets its own.
                self.open(Step::Menu(usize::MAX), shown);
                if self.flow.is_some() {
                    self.connect_in_palette(profile);
                } else if let Err(error) = self.connect(profile) {
                    host::log(2, &format!("{TITLE}: {error}"));
                }
            }
        }
    }

    /// Follow the connection the palette is showing: leave when it connected, say why when it failed.
    /// The folder is open: the host is remembered with it, and the palette is done.
    fn finish(&mut self, client: usize) {
        let canonical = self.clients[client].canonical_root.clone();
        let mut profile = self.clients[client].connected.clone().unwrap_or_else(|| self.clients[client].profile.clone());
        profile.root = canonical.clone();
        profile.remember_folder(&canonical);
        self.clients[client].connected = Some(profile.clone());
        self.remember(profile);
        self.close();
    }

    fn sync(&mut self) {
        self.pump_browse();
        if let Some(Flow { step: Step::Browse(step), .. }) = &self.flow {
            let client = step.client;
            if self.clients[client].scope.is_some() && self.clients[client].mounts != step.mounts {
                self.finish(client);
            } else if let Some(why) = self.clients[client].failure.clone() {
                // The connection ended while a folder was being chosen.
                let label = self.clients[client].profile.label();
                let profile = self.clients[client].profile.clone();
                self.note(&why, &[TITLE, &label], "Connection failed", Mode::Choose, Self::failed_choices());
                if let Some(flow) = &mut self.flow { flow.step = Step::Connecting { client, profile, failed: true }; }
            }
        }
        if let Some(Flow { step: Step::Connecting { client, profile, failed }, .. }) = &self.flow {
            let (client, profile, failed) = (*client, profile.clone(), *failed);
            if self.clients[client].scope.is_some() {
                self.finish(client);
            } else if self.clients[client].awaiting_folder && !failed {
                self.begin_browse(client);
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
            if command.command == format!("{ID}.connect") {
                self.begin();
                return;
            }
            if command.command == format!("{ID}.folder") {
                // The menu of the current connection has the action; with none, there is nothing to change.
                let index = self.selected;
                if self.clients.get(index).is_some_and(|client| client.scope.is_some()) {
                    let label = self.clients[index].profile.label();
                    let shown = view(&[TITLE, &label], "Connecting…", Mode::Choose, vec![], Activity::Busy, None);
                    self.open(Step::Menu(usize::MAX), shown);
                    if self.flow.is_some() { self.change_folder(index); }
                    self.sync();
                }
                return;
            }
            if command.command == format!("{ID}.disconnect") {
                if let Err(error) = self.clients[self.selected].disconnect(true) { host::log(2, &format!("{TITLE}: {error}")); }
                self.sync();
                return;
            }
        }
        // What the list of targets prints is not a connection's.
        #[cfg(not(transport = "ssh"))]
        if self.lister_message(&message) {
            self.sync();
            return;
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
        // A connection is never made on its own when the application starts: a remote folder
        // left in the file tree by an earlier version stays offline until the person
        // connects again from the palette.
        if matches!(event, Event::Restore { .. }) { return; }
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
    fn a_multi_line_failure_becomes_one_short_line() {
        let text = "No ED25519 host key is known.\r\nHost key verification failed.\r\n";
        assert_eq!(one_line(text), "No ED25519 host key is known. Host key verification failed.");
        assert!(!one_line(&"word ".repeat(100)).contains('\n'));
    }
    #[test]
    fn a_long_note_keeps_its_end_and_fits_the_palette_in_bytes_whatever_the_script() {
        for text in ["é".repeat(300), "检测到代理配置 ".repeat(40) + "execvpe(sh) failed", "a\0b\u{7}c".repeat(100)] {
            let line = one_line(&text);
            assert!(line.len() <= 200 && !line.contains(['\n', '\0', '\r']), "{line}");
        }
        assert!(one_line(&("x ".repeat(200) + "the cause")).ends_with("the cause"));
    }
    #[test]
    fn a_loader_failure_is_said_in_its_own_words() {
        let line = one_line("Warning: Permanently added 'h' to the list of known hosts.\nloog-error: no directory on this machine allows running the agent");
        assert_eq!(explain(line), "no directory on this machine allows running the agent");
        let line = one_line("<3>WSL (12 - Relay) ERROR: CreateProcessCommon:800: execvpe(sh) failed: No such file or directory");
        assert!(explain(line).starts_with("The target has no POSIX shell"));
        assert_eq!(explain("Host key verification failed.".into()), "Host key verification failed.");
    }
    #[test]
    fn a_remote_folder_is_named_for_the_person() {
        assert_eq!(folder(protocol::HOME), "Home folder");
        assert_eq!(folder("/srv/app"), "/srv/app");
    }
}

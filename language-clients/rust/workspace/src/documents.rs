//! Bounded, version-fixed buffer discovery. All path interpretation stays with
//! the resource provider; neither native Path nor URI decoding is used here.
use super::*;
use api::session::{DocumentResource, DocumentVersion};
use std::collections::BTreeSet;
use std::sync::Arc;

pub const MAX_TEXT: usize = 4 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 128;
const MAX_ROOTS: usize = 128;
const MAX_SETTINGS_PROJECTS: usize = 16;
const MAX_SETTINGS_TOTAL: usize = 1024 * 1024;
pub type Settings = Arc<BTreeMap<String, String>>;

pub struct ReadyDocument {
    pub descriptor: DocumentResource,
    pub root: api::PresentedRoot,
    pub project: api::ProviderPath,
    pub root_uri: String,
    pub uri: String,
    pub text: String,
    pub settings: Settings,
}
pub enum DocumentEvent {
    Ready(ReadyDocument),
    Closed(u64),
    Roots(Vec<api::PresentedRoot>),
    Invalidated(api::ScopeId),
    Error(String),
}
struct Loaded {
    descriptor: DocumentResource,
    scope: api::ScopeId,
    project: api::ProviderPath,
}
struct Load {
    descriptor: DocumentResource,
    candidates: Vec<api::PresentedRoot>,
    uri: String,
    root_uri: String,
    text: String,
    project: Option<api::ProviderPath>,
    probe: Option<crate::project::Probe>,
    settings: Settings,
    settings_paths: BTreeSet<api::ProviderPath>,
}
struct CachedSettings {
    values: Settings,
    paths: BTreeSet<api::ProviderPath>,
}
enum Action {
    Roots(Vec<api::PresentedRoot>),
    Documents(Vec<DocumentResource>),
    Uri(Load),
    Project(Load),
    Settings(Load, crate::settings::ProjectSettings),
    ResolveSettings(Load),
    RootUri(Load),
    Read(Load),
}
struct Pending {
    command: Command,
    action: Action,
    ticket: Option<u64>,
    request: Option<u64>,
    epoch: u64,
}
pub struct Documents {
    languages: Vec<String>,
    epoch: u64,
    roots_dirty: bool,
    documents_dirty: bool,
    roots: Vec<api::PresentedRoot>,
    desired: Vec<DocumentResource>,
    loaded: BTreeMap<u64, Loaded>,
    skipped: BTreeSet<u64>,
    pending: Option<Pending>,
    stopped: bool,
    project_policy: Option<crate::project::Policy>,
    read_settings: bool,
    settings: BTreeMap<(api::ScopeId, api::ProviderPath), CachedSettings>,
    watches: crate::watch::Watches,
    dependencies: BTreeMap<(api::ScopeId, u64), BTreeSet<api::ProviderPath>>,
    invalidated: BTreeSet<api::ScopeId>,
    monitoring_errors: Vec<String>,
}
impl Documents {
    pub fn new(languages: Vec<String>) -> Self {
        Self {
            languages,
            epoch: 0,
            roots_dirty: true,
            documents_dirty: true,
            roots: vec![],
            desired: vec![],
            loaded: BTreeMap::new(),
            skipped: BTreeSet::new(),
            pending: None,
            stopped: false,
            project_policy: None,
            read_settings: false,
            settings: BTreeMap::new(),
            watches: Default::default(),
            dependencies: BTreeMap::new(),
            invalidated: BTreeSet::new(),
            monitoring_errors: vec![],
        }
    }
    pub fn with_projects(
        mut self,
        markers: Vec<String>,
        outermost: bool,
        file_fallback: bool,
    ) -> Self {
        self.project_policy = Some(crate::project::Policy {
            markers,
            outermost,
            file_fallback,
        });
        self
    }
    pub fn with_settings(mut self) -> Self {
        self.read_settings = true;
        self
    }
    fn project_uri(&mut self, load: Load) {
        self.request(
            load.candidates[0].scope,
            api::Operation::ResourceUri {
                path: load.project.clone().unwrap(),
            },
            Action::RootUri(load),
        );
    }
    fn project_settings(&mut self, mut load: Load, path: api::ProviderPath) {
        load.project = Some(path.clone());
        let scope = load.candidates[0].scope;
        if !self.read_settings {
            self.project_uri(load);
            return;
        }
        if let Some(settings) = self.settings.get(&(scope, path.clone())) {
            load.settings = settings.values.clone();
            load.settings_paths = settings.paths.clone();
            for path in &load.settings_paths {
                self.track(scope, load.descriptor.stamp.document, path.clone());
            }
            self.project_uri(load);
            return;
        }
        let reader = crate::settings::ProjectSettings::new(path);
        self.request(
            scope,
            reader.operation().unwrap(),
            Action::Settings(load, reader),
        );
    }
    fn locate_project(&mut self, mut load: Load) -> Result<(), String> {
        let root = &load.candidates[0];
        if let Some(old) = self
            .loaded
            .get(&load.descriptor.stamp.document)
            .filter(|old| {
                old.scope == root.scope
                    && old.descriptor.resource == load.descriptor.resource
                    && old.descriptor.stamp.lifecycle == load.descriptor.stamp.lifecycle
            })
        {
            // Buffer edits reuse the discovered project. Closing/reopening or
            // explicit reload re-probes, without filesystem reads per keystroke.
            let project = old.project.clone();
            self.project_settings(load, project);
        } else if let Some(policy) = &self.project_policy {
            let api::ResourceRef::Provided {
                path: root_path, ..
            } = &root.resource
            else {
                unreachable!()
            };
            let api::ResourceRef::Provided { path, .. } = &load.descriptor.resource else {
                unreachable!()
            };
            let probe =
                crate::project::Probe::new(root_path.clone(), path.clone(), policy.clone())?;
            self.dependencies
                .remove(&(root.scope, load.descriptor.stamp.document));
            let operation = probe.operation();
            load.probe = Some(probe);
            self.request(root.scope, operation, Action::Project(load));
        } else {
            let api::ResourceRef::Provided { path, .. } = &root.resource else {
                unreachable!()
            };
            let path = path.clone();
            self.project_settings(load, path);
        }
        Ok(())
    }
    pub fn current(&self, stamp: DocumentVersion) -> bool {
        !self.documents_dirty && self.desired.iter().any(|d| d.stamp == stamp)
    }
    pub fn is_open(&self, resource: &api::ResourceRef) -> bool {
        self.desired
            .iter()
            .any(|document| &document.resource == resource)
    }
    /// Readable roots in the exact environment, with the source scope first.
    /// Dependency locations may be readable without allowing code execution.
    pub fn read_scopes(&self, environment: &api::EnvironmentKey, preferred: u64) -> Vec<u64> {
        if self.stopped || self.roots_dirty {
            return vec![];
        }
        let mut scopes: Vec<_> = self
            .roots
            .iter()
            .filter(|root| {
                root.resource.environment() == Some(environment)
                    && root.capabilities.contains(&api::Capability::FilesRead)
            })
            .map(|root| root.scope)
            .collect();
        scopes.sort_unstable_by_key(|scope| (*scope != preferred, *scope));
        scopes.dedup();
        scopes
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.pending = None;
        self.settings.clear();
        self.watches.sync(&[]);
        self.dependencies.clear();
    }
    pub fn refresh(&mut self) {
        self.epoch += 1;
        self.roots_dirty = true;
        self.documents_dirty = true;
    }
    pub fn reload(&mut self) {
        self.loaded.clear();
        self.settings.clear();
        self.dependencies.clear();
        self.refresh();
    }
    pub fn needs_drive(&self) -> bool {
        self.watches.needs_drive()
            || !self.invalidated.is_empty()
            || (!self.stopped
                && self.pending.as_ref().map_or_else(
                    || {
                        self.roots_dirty
                            || self.documents_dirty
                            || self.desired.iter().any(|d| {
                                !self.skipped.contains(&d.stamp.document)
                                    && self
                                        .loaded
                                        .get(&d.stamp.document)
                                        .is_none_or(|old| old.descriptor.stamp != d.stamp)
                            })
                    },
                    |pending| pending.ticket.is_none() && pending.request.is_none(),
                ))
    }
    fn track(&mut self, scope: api::ScopeId, id: u64, path: api::ProviderPath) {
        if self
            .dependencies
            .get(&(scope, id))
            .is_some_and(|paths| paths.contains(&path))
        {
            return;
        }
        let count = self.dependencies.values().map(BTreeSet::len).sum::<usize>();
        let bytes = self
            .dependencies
            .values()
            .flatten()
            .map(|path| path.as_str().len())
            .sum::<usize>();
        if count >= 4096 || bytes + path.as_str().len() > 1024 * 1024 {
            if self.monitoring_errors.is_empty() {
                self.monitoring_errors
                    .push("Project monitoring dependency budget exceeded".into());
            }
            return;
        }
        self.dependencies
            .entry((scope, id))
            .or_default()
            .insert(path);
    }
    fn watch_events(&mut self, events: Vec<crate::watch::Change>) {
        for event in events {
            match event {
                crate::watch::Change::Error(error) if self.monitoring_errors.len() < 16 => {
                    self.monitoring_errors.push(error)
                }
                crate::watch::Change::Paths(scope, paths) if !self.stopped => {
                    if paths.is_none()
                        || self.dependencies.iter().any(|((owner, _), dependencies)| {
                            *owner == scope
                                && paths
                                    .as_ref()
                                    .unwrap()
                                    .iter()
                                    .any(|path| dependencies.contains(path))
                        })
                    {
                        self.invalidated.insert(scope);
                    }
                }
                _ => {}
            }
        }
    }
    fn schedule(&mut self, command: Command, action: Action) {
        self.pending = Some(Pending {
            command,
            action,
            ticket: None,
            request: None,
            epoch: self.epoch,
        });
    }
    fn request(&mut self, scope: u64, operation: api::Operation, action: Action) {
        self.schedule(
            Command::Request {
                scope,
                operation,
                timeout_ms: 15_000,
            },
            action,
        );
    }
    fn file_uri(&mut self, load: Load) {
        let api::ResourceRef::Provided { path, .. } = &load.descriptor.resource else {
            unreachable!()
        };
        self.request(
            load.candidates[0].scope,
            api::Operation::ResourceUri { path: path.clone() },
            Action::Uri(load),
        );
    }
    pub fn drive(
        &mut self,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<DocumentEvent> {
        let changes = self.watches.drive(submit);
        self.watch_events(changes);
        let mut events: Vec<_> = std::mem::take(&mut self.monitoring_errors)
            .into_iter()
            .map(DocumentEvent::Error)
            .collect();
        if self.stopped {
            return events;
        }
        for scope in std::mem::take(&mut self.invalidated) {
            self.epoch += 1;
            self.loaded.retain(|_, old| old.scope != scope);
            self.settings.retain(|(owner, _), _| *owner != scope);
            self.skipped.clear();
            self.documents_dirty = true;
            events.push(DocumentEvent::Invalidated(scope));
        }
        if self.pending.is_none() {
            if self.roots_dirty {
                self.roots_dirty = false;
                self.schedule(
                    Command::Roots {
                        after: None,
                        limit: 32,
                    },
                    Action::Roots(vec![]),
                );
            } else if self.documents_dirty {
                self.documents_dirty = false;
                self.schedule(
                    Command::Documents {
                        after: None,
                        revision: None,
                        limit: 32,
                    },
                    Action::Documents(vec![]),
                );
            } else if let Some(descriptor) = self
                .desired
                .iter()
                .find(|d| {
                    !self.skipped.contains(&d.stamp.document)
                        && self
                            .loaded
                            .get(&d.stamp.document)
                            .is_none_or(|old| old.descriptor.stamp != d.stamp)
                })
                .cloned()
            {
                let candidates: Vec<_> = self
                    .roots
                    .iter()
                    .filter(|r| {
                        r.resource.environment() == descriptor.resource.environment()
                            && r.capabilities.contains(&api::Capability::Process)
                            && r.capabilities.contains(&api::Capability::FilesRead)
                    })
                    .cloned()
                    .collect();
                if candidates.is_empty() {
                    self.skipped.insert(descriptor.stamp.document);
                } else {
                    self.file_uri(Load {
                        descriptor,
                        candidates,
                        uri: String::new(),
                        root_uri: String::new(),
                        text: String::new(),
                        project: None,
                        probe: None,
                        settings: Arc::default(),
                        settings_paths: BTreeSet::new(),
                    });
                }
            }
        }
        if let Some(pending) = &mut self.pending {
            if pending.ticket.is_none() && pending.request.is_none() {
                match submit(pending.command.clone()) {
                    Ok(ticket) => pending.ticket = Some(ticket),
                    Err(error) => events.push(DocumentEvent::Error(error)),
                }
            }
        }
        events
    }
    pub fn receive(&mut self, event: &Event) -> Vec<DocumentEvent> {
        let changes = self.watches.receive(event);
        self.watch_events(changes);
        if self.stopped {
            return vec![];
        }
        match event {
            Event::RootsChanged => {
                if self.read_settings {
                    self.reload();
                } else {
                    self.refresh();
                }
                return vec![];
            }
            Event::SettingsChanged if self.read_settings => {
                self.reload();
                return vec![];
            }
            Event::DocumentsChanged { .. } => {
                self.epoch += 1;
                self.documents_dirty = true;
                return vec![];
            }
            _ => {}
        }
        let Some(pending) = &self.pending else {
            return vec![];
        };
        let result = match event {
            Event::Reply { ticket, result } if Some(*ticket) == pending.ticket => {
                if let Ok(Reply::Request { id }) = result {
                    let pending = self.pending.as_mut().unwrap();
                    pending.ticket = None;
                    pending.request = Some(*id);
                    return vec![];
                }
                result.clone().map(Response::Reply)
            }
            Event::Delivery {
                event: api::Delivery::Complete { request, result },
            } if Some(*request) == pending.request => result.clone().map(Response::Answer),
            _ => return vec![],
        };
        let pending = self.pending.take().unwrap();
        if pending.epoch != self.epoch {
            // Finish a bounded read before restarting. A stream of keystrokes
            // cannot accumulate abandoned requests or retained partial buffers.
            if matches!(pending.action, Action::Roots(_)) {
                self.roots_dirty = true;
            }
            return vec![];
        }
        self.complete(pending.action, result)
    }
    fn complete(
        &mut self,
        action: Action,
        result: Result<Response, api::Failure>,
    ) -> Vec<DocumentEvent> {
        let mut events = vec![];
        match (action, result) {
            (Action::Roots(mut roots), Ok(Response::Reply(Reply::Roots { roots: page, next }))) => {
                if roots.len() + page.len() > MAX_ROOTS {
                    return vec![DocumentEvent::Error("Language root budget exceeded".into())];
                }
                roots.extend(page);
                if let Some(after) = next {
                    self.schedule(
                        Command::Roots {
                            after: Some(after),
                            limit: 32,
                        },
                        Action::Roots(roots),
                    );
                } else {
                    let allowed: BTreeSet<_> = roots
                        .iter()
                        .filter(|r| r.capabilities.contains(&api::Capability::Process))
                        .map(|r| r.scope)
                        .collect();
                    self.loaded.retain(|id, old| {
                        if allowed.contains(&old.scope) {
                            true
                        } else {
                            events.push(DocumentEvent::Closed(*id));
                            false
                        }
                    });
                    self.roots = roots.clone();
                    let changes = self.watches.sync(&roots);
                    self.watch_events(changes);
                    self.dependencies
                        .retain(|(scope, _), _| allowed.contains(scope));
                    self.skipped.clear();
                    self.documents_dirty = true;
                    events.push(DocumentEvent::Roots(roots));
                }
            }
            (
                Action::Documents(mut documents),
                Ok(Response::Reply(Reply::Documents {
                    documents: page,
                    revision,
                    next,
                })),
            ) => {
                documents.extend(page.into_iter().filter(|d| {
                    d.resource.environment().is_some()
                        && self.languages.contains(&d.language)
                        && d.synchronized
                        && d.byte_length <= MAX_TEXT as u64
                }));
                if documents.len() > MAX_DOCUMENTS {
                    return vec![DocumentEvent::Error(
                        "Language document budget exceeded".into(),
                    )];
                }
                if let Some(after) = next {
                    self.schedule(
                        Command::Documents {
                            after: Some(after),
                            revision: Some(revision),
                            limit: 32,
                        },
                        Action::Documents(documents),
                    );
                } else {
                    self.loaded.retain(|id, old| {
                        let keep = documents.iter().any(|d| {
                            d.stamp.document == *id
                                && d.stamp.lifecycle == old.descriptor.stamp.lifecycle
                                && d.resource == old.descriptor.resource
                        });
                        if !keep {
                            events.push(DocumentEvent::Closed(*id));
                        }
                        keep
                    });
                    self.desired = documents;
                    self.dependencies.retain(|(_, id), _| {
                        self.desired.iter().any(|doc| doc.stamp.document == *id)
                    });
                    self.settings.retain(|(scope, project), _| {
                        self.loaded
                            .values()
                            .any(|old| old.scope == *scope && old.project == *project)
                    });
                    self.skipped.clear();
                }
            }
            (Action::Uri(mut load), Ok(Response::Answer(api::Answer::Uri { uri }))) => {
                load.uri = uri;
                let id = load.descriptor.stamp.document;
                if let Err(error) = self.locate_project(load) {
                    self.skipped.insert(id);
                    events.push(DocumentEvent::Error(error));
                }
            }
            (Action::Project(mut load), result) => {
                if let Ok(Response::Answer(api::Answer::Path { path })) = &result {
                    self.track(
                        load.candidates[0].scope,
                        load.descriptor.stamp.document,
                        path.clone(),
                    );
                }
                let result = result.and_then(|response| match response {
                    Response::Answer(answer) => Ok(answer),
                    _ => Err(api::Failure::new(
                        api::FailureKind::InvalidRequest,
                        "Unexpected project reply",
                    )),
                });
                let probe = load.probe.as_mut().unwrap();
                match probe.receive(result) {
                    Ok(Some(project)) => {
                        load.probe = None;
                        self.project_settings(load, project);
                    }
                    Ok(None) => {
                        let operation = probe.operation();
                        self.request(load.candidates[0].scope, operation, Action::Project(load));
                    }
                    Err(error) => {
                        self.skipped.insert(load.descriptor.stamp.document);
                        events.push(DocumentEvent::Error(error));
                    }
                }
            }
            (Action::Settings(mut load, mut reader), result) => {
                if let Ok(Response::Answer(api::Answer::Path { path })) = &result {
                    let scope = load.candidates[0].scope;
                    let id = load.descriptor.stamp.document;
                    if let Some(api::Operation::ChildPath { parent, name }) = reader.operation()
                        && name == "extension-settings.json"
                    {
                        // Watch the directory only until its settings filename
                        // can be resolved; sibling files must not restart LSP.
                        load.settings_paths.remove(&parent);
                        if let Some(paths) = self.dependencies.get_mut(&(scope, id)) {
                            paths.remove(&parent);
                        }
                    }
                    load.settings_paths.insert(path.clone());
                    self.track(scope, id, path.clone());
                }
                let result = result.and_then(|response| match response {
                    Response::Answer(answer) => Ok(answer),
                    _ => Err(api::Failure::new(
                        api::FailureKind::InvalidRequest,
                        "Unexpected settings reply",
                    )),
                });
                match reader.receive(result) {
                    Ok(Some(project)) => self.schedule(
                        Command::ResolveSettings {
                            scope: Some(load.candidates[0].scope),
                            project: Some(project),
                        },
                        Action::ResolveSettings(load),
                    ),
                    Ok(None) => self.request(
                        load.candidates[0].scope,
                        reader.operation().unwrap(),
                        Action::Settings(load, reader),
                    ),
                    Err(error) => {
                        self.skipped.insert(load.descriptor.stamp.document);
                        events.push(DocumentEvent::Error(error));
                    }
                }
            }
            (
                Action::ResolveSettings(mut load),
                Ok(Response::Reply(Reply::Settings { values })),
            ) => {
                let bytes = |values: &BTreeMap<String, String>| {
                    values.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()
                };
                if values.len() > 256
                    || bytes(&values) > crate::settings::MAX_SETTINGS
                    || self.settings.len() >= MAX_SETTINGS_PROJECTS
                    || self
                        .settings
                        .values()
                        .map(|v| bytes(&v.values))
                        .sum::<usize>()
                        + bytes(&values)
                        > MAX_SETTINGS_TOTAL
                {
                    self.skipped.insert(load.descriptor.stamp.document);
                    return vec![DocumentEvent::Error(
                        "Project settings cache exceeds its budget".into(),
                    )];
                }
                load.settings = Arc::new(values);
                self.settings.insert(
                    (load.candidates[0].scope, load.project.clone().unwrap()),
                    CachedSettings {
                        values: load.settings.clone(),
                        paths: load.settings_paths.clone(),
                    },
                );
                self.project_uri(load);
            }
            (Action::Uri(mut load), Err(error))
                if matches!(
                    error.kind,
                    api::FailureKind::PermissionDenied | api::FailureKind::NotFound
                ) =>
            {
                load.candidates.remove(0);
                if !load.candidates.is_empty() {
                    self.file_uri(load);
                } else {
                    self.skipped.insert(load.descriptor.stamp.document);
                }
            }
            (Action::RootUri(mut load), Ok(Response::Answer(api::Answer::Uri { uri }))) => {
                load.root_uri = uri;
                self.schedule(
                    Command::ReadDocument {
                        stamp: load.descriptor.stamp,
                        offset: 0,
                        limit: 64 * 1024,
                    },
                    Action::Read(load),
                );
            }
            (
                Action::Read(mut load),
                Ok(Response::Reply(Reply::DocumentChunk { stamp, text, next })),
            ) => {
                if stamp != load.descriptor.stamp
                    || load.text.len() + text.len() > load.descriptor.byte_length as usize
                    || next.is_some_and(|next| {
                        next != (load.text.len() + text.len()) as u64 || text.is_empty()
                    })
                {
                    self.skipped.insert(load.descriptor.stamp.document);
                    return vec![DocumentEvent::Error(
                        "Invalid version-fixed document chunk".into(),
                    )];
                }
                load.text.push_str(&text);
                if let Some(offset) = next {
                    self.schedule(
                        Command::ReadDocument {
                            stamp,
                            offset,
                            limit: 64 * 1024,
                        },
                        Action::Read(load),
                    );
                } else if load.text.len() != load.descriptor.byte_length as usize {
                    self.skipped.insert(stamp.document);
                    events.push(DocumentEvent::Error("Incomplete document snapshot".into()));
                } else {
                    self.loaded.insert(
                        stamp.document,
                        Loaded {
                            descriptor: load.descriptor.clone(),
                            scope: load.candidates[0].scope,
                            project: load.project.clone().unwrap(),
                        },
                    );
                    events.push(DocumentEvent::Ready(ReadyDocument {
                        descriptor: load.descriptor,
                        root: load.candidates.remove(0),
                        project: load.project.unwrap(),
                        root_uri: load.root_uri,
                        uri: load.uri,
                        text: load.text,
                        settings: load.settings,
                    }));
                }
            }
            (_, Err(error)) if error.kind == api::FailureKind::Conflict => {
                self.documents_dirty = true
            }
            (action, result) => {
                match action {
                    Action::Uri(load)
                    | Action::RootUri(load)
                    | Action::Read(load)
                    | Action::ResolveSettings(load) => {
                        self.skipped.insert(load.descriptor.stamp.document);
                    }
                    _ => {}
                }
                events.push(DocumentEvent::Error(match result {
                    Err(error) => error.message,
                    _ => "Unexpected language workspace reply".into(),
                }));
            }
        }
        events
    }
}
enum Response {
    Reply(Reply),
    Answer(api::Answer),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(scope: u64, environment: &str) -> api::PresentedRoot {
        api::PresentedRoot {
            scope,
            resource: api::ResourceRef::provided(
                api::EnvironmentKey::new("provider", environment).unwrap(),
                "opaque-root",
            )
            .unwrap(),
            name: environment.into(),
            capabilities: vec![api::Capability::Process, api::Capability::FilesRead],
        }
    }
    fn document(id: u64, environment: &str, text: &str) -> DocumentResource {
        DocumentResource {
            stamp: DocumentVersion {
                document: id,
                lifecycle: 1,
                version: 1,
            },
            resource: api::ResourceRef::provided(
                api::EnvironmentKey::new("provider", environment).unwrap(),
                "opaque\\中",
            )
            .unwrap(),
            language: "rust".into(),
            variant: "rust".into(),
            byte_length: text.len() as u64,
            synchronized: true,
        }
    }
    fn command(docs: &mut Documents) -> Command {
        let mut command = None;
        assert!(
            docs.drive(&mut |c| {
                command = Some(c);
                Ok(1)
            })
            .is_empty()
        );
        command.unwrap()
    }
    fn reply(docs: &mut Documents, reply: Reply) -> Vec<DocumentEvent> {
        docs.receive(&Event::Reply {
            ticket: 1,
            result: Ok(reply),
        })
    }
    fn answer(docs: &mut Documents, answer: api::Answer) -> Vec<DocumentEvent> {
        assert!(reply(docs, Reply::Request { id: 7 }).is_empty());
        docs.receive(&Event::Delivery {
            event: api::Delivery::Complete {
                request: 7,
                result: Ok(answer),
            },
        })
    }
    fn listed(roots: Vec<api::PresentedRoot>, documents: Vec<DocumentResource>) -> Documents {
        let mut docs = Documents::new(vec!["rust".into()]);
        assert!(matches!(command(&mut docs), Command::Roots { .. }));
        reply(&mut docs, Reply::Roots { roots, next: None });
        assert!(matches!(command(&mut docs), Command::Documents { .. }));
        reply(
            &mut docs,
            Reply::Documents {
                documents,
                revision: 1,
                next: None,
            },
        );
        docs
    }

    fn settings_read(docs: &mut Documents, scope: u64, value: &str) {
        assert!(matches!(command(docs), Command::Request {
            scope: actual, operation: api::Operation::ResourceUri { .. }, .. } if actual == scope));
        answer(
            docs,
            api::Answer::Uri {
                uri: "file:///same-file".into(),
            },
        );
        for (name, path) in [
            (".loog", "opaque-config"),
            ("extension-settings.json", "opaque-settings"),
        ] {
            assert!(matches!(command(docs), Command::Request {
                scope: actual, operation: api::Operation::ChildPath { name: actual_name, .. }, .. }
                if actual == scope && actual_name == name));
            answer(
                docs,
                api::Answer::Path {
                    path: api::ProviderPath::new(path).unwrap(),
                },
            );
        }
        assert!(matches!(command(docs), Command::Request {
            scope: actual, operation: api::Operation::Read { offset: 0, .. }, .. } if actual == scope));
        let project = serde_json::json!({"plugin.configuration":value});
        answer(
            docs,
            api::Answer::Bytes {
                bytes: project.to_string().into_bytes(),
                revision: Some("one".into()),
                eof: true,
            },
        );
        assert!(
            matches!(command(docs), Command::ResolveSettings { scope: Some(actual), project: Some(actual_project) }
            if actual == scope && actual_project == project)
        );
    }
    fn ready_settings(docs: &mut Documents, descriptor: &DocumentResource) -> ReadyDocument {
        assert!(matches!(
            command(docs),
            Command::Request {
                operation: api::Operation::ResourceUri { .. },
                ..
            }
        ));
        answer(
            docs,
            api::Answer::Uri {
                uri: "file:///same-root".into(),
            },
        );
        assert!(
            matches!(command(docs), Command::ReadDocument { stamp, .. } if stamp == descriptor.stamp)
        );
        let mut events = reply(
            docs,
            Reply::DocumentChunk {
                stamp: descriptor.stamp,
                text: "code".into(),
                next: None,
            },
        );
        let DocumentEvent::Ready(doc) = events.remove(0) else {
            panic!("expected configured document")
        };
        doc
    }
    #[test]
    fn project_notifications_invalidate_only_the_owning_scope_and_exact_dependencies() {
        let one = document(1, "one", "code");
        let two = document(2, "two", "code");
        let mut docs = listed(
            vec![root(10, "one"), root(20, "two")],
            vec![one.clone(), two.clone()],
        )
        .with_settings();
        for (scope, descriptor) in [(10, &one), (20, &two)] {
            settings_read(&mut docs, scope, "configured");
            reply(
                &mut docs,
                Reply::Settings {
                    values: BTreeMap::new(),
                },
            );
            ready_settings(&mut docs, descriptor);
        }
        let path = api::ProviderPath::new("opaque-settings").unwrap();
        let dependency = &docs.dependencies[&(10, 1)];
        assert!(dependency.contains(&path));
        assert!(!dependency.contains(&api::ProviderPath::new("opaque-config").unwrap()));
        docs.watch_events(vec![
            crate::watch::Change::Paths(30, Some(vec![path.clone()])),
            crate::watch::Change::Paths(10, Some(vec![api::ProviderPath::new("sibling").unwrap()])),
        ]);
        assert!(docs.invalidated.is_empty());
        docs.watch_events(vec![crate::watch::Change::Paths(10, Some(vec![path]))]);
        let mut commands = vec![];
        let events = docs.drive(&mut |command| {
            commands.push(command);
            Ok(1)
        });
        assert!(matches!(&events[..], [DocumentEvent::Invalidated(10)]));
        assert!(matches!(&commands[..], [Command::Documents { .. }]));
        assert!(!docs.loaded.contains_key(&1));
        assert!(docs.loaded.contains_key(&2));
        assert_eq!(
            docs.settings
                .keys()
                .map(|(scope, _)| *scope)
                .collect::<Vec<_>>(),
            [20]
        );
        docs.watch_events(vec![crate::watch::Change::Paths(20, None)]);
        assert!(docs.invalidated.contains(&20));
    }
    #[test]
    fn project_settings_are_scoped_cached_for_edits_and_released_on_close() {
        let mut one = document(1, "one", "code");
        let two = document(2, "two", "code");
        let mut docs = listed(
            vec![root(10, "one"), root(20, "two")],
            vec![one.clone(), two.clone()],
        )
        .with_settings();
        let values =
            |v: &str| BTreeMap::from([("plugin.configuration".into(), format!("\"{v}\""))]);
        settings_read(&mut docs, 10, "first");
        reply(
            &mut docs,
            Reply::Settings {
                values: values("first"),
            },
        );
        let first = ready_settings(&mut docs, &one);
        settings_read(&mut docs, 20, "second");
        reply(
            &mut docs,
            Reply::Settings {
                values: values("second"),
            },
        );
        let second = ready_settings(&mut docs, &two);
        assert_eq!(*first.settings, values("first"));
        assert_eq!(*second.settings, values("second"));
        assert_eq!(docs.settings.len(), 2);

        one.stamp.version += 1;
        docs.receive(&Event::DocumentsChanged { revision: 2 });
        assert!(matches!(command(&mut docs), Command::Documents { .. }));
        reply(
            &mut docs,
            Reply::Documents {
                documents: vec![one.clone(), two.clone()],
                revision: 2,
                next: None,
            },
        );
        command(&mut docs);
        answer(
            &mut docs,
            api::Answer::Uri {
                uri: "file:///same-file".into(),
            },
        );
        let updated = ready_settings(&mut docs, &one);
        assert!(Arc::ptr_eq(&first.settings, &updated.settings));
        docs.receive(&Event::DocumentsChanged { revision: 3 });
        command(&mut docs);
        let events = reply(
            &mut docs,
            Reply::Documents {
                documents: vec![two],
                revision: 3,
                next: None,
            },
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, DocumentEvent::Closed(1)))
        );
        assert_eq!(docs.settings.len(), 1);
        assert_eq!(docs.settings.keys().next().unwrap().0, 20);
    }

    #[test]
    fn settings_changes_discard_in_flight_resolution_and_recheck_roots() {
        let one = document(1, "one", "code");
        let mut docs = listed(vec![root(10, "one")], vec![one.clone()]).with_settings();
        settings_read(&mut docs, 10, "old");
        docs.receive(&Event::SettingsChanged);
        assert!(
            reply(
                &mut docs,
                Reply::Settings {
                    values: BTreeMap::new()
                }
            )
            .is_empty()
        );
        assert!(docs.settings.is_empty());
        assert!(!docs.current(one.stamp));
        assert!(matches!(command(&mut docs), Command::Roots { .. }));
        reply(
            &mut docs,
            Reply::Roots {
                roots: vec![root(10, "one")],
                next: None,
            },
        );
        command(&mut docs);
        reply(
            &mut docs,
            Reply::Documents {
                documents: vec![one],
                revision: 2,
                next: None,
            },
        );
        settings_read(&mut docs, 10, "new");
        docs.receive(&Event::RootsChanged);
        reply(
            &mut docs,
            Reply::Settings {
                values: BTreeMap::new(),
            },
        );
        assert!(docs.settings.is_empty());
        assert!(matches!(command(&mut docs), Command::Roots { .. }));
    }

    #[test]
    fn location_scopes_require_read_grants_and_exact_environment_but_not_execution() {
        let mut dependency = root(20, "one");
        dependency
            .capabilities
            .retain(|c| *c != api::Capability::Process);
        let mut denied = root(30, "one");
        denied
            .capabilities
            .retain(|c| *c != api::Capability::FilesRead);
        let mut docs = listed(
            vec![dependency, root(10, "one"), denied, root(40, "two")],
            vec![],
        );
        let env = api::EnvironmentKey::new("provider", "one").unwrap();
        assert_eq!(docs.read_scopes(&env, 10), vec![10, 20]);
        docs.refresh();
        assert!(docs.read_scopes(&env, 10).is_empty());
    }
    fn paths(docs: &mut Documents, scope: u64) {
        assert!(
            matches!(command(docs), Command::Request { scope: found, operation: api::Operation::ResourceUri { .. }, .. } if found == scope)
        );
        answer(
            docs,
            api::Answer::Uri {
                uri: "file:///opaque%5C%E4%B8%AD".into(),
            },
        );
        assert!(
            matches!(command(docs), Command::Request { scope: found, operation: api::Operation::ResourceUri { .. }, .. } if found == scope)
        );
        answer(
            docs,
            api::Answer::Uri {
                uri: "file:///root".into(),
            },
        );
    }
    #[test]
    fn project_discovery_uses_scoped_provider_paths_and_edits_reuse_the_result() {
        let mut descriptor = document(1, "one", "code");
        let mut docs = listed(vec![root(10, "one")], vec![descriptor.clone()]).with_projects(
            vec!["Cargo.toml".into()],
            true,
            true,
        );
        command(&mut docs);
        answer(
            &mut docs,
            api::Answer::Uri {
                uri: "file:///source".into(),
            },
        );
        assert!(matches!(
            command(&mut docs),
            Command::Request {
                scope: 10,
                operation: api::Operation::ParentPath { .. },
                ..
            }
        ));
        answer(
            &mut docs,
            api::Answer::Parent {
                path: Some(api::ProviderPath::new("opaque\\project").unwrap()),
            },
        );
        assert!(matches!(command(&mut docs), Command::Request {
            operation: api::Operation::ChildPath { parent, name }, .. }
            if parent.as_str() == "opaque\\project" && name == "Cargo.toml"));
        answer(
            &mut docs,
            api::Answer::Path {
                path: api::ProviderPath::new("opaque-marker").unwrap(),
            },
        );
        command(&mut docs);
        answer(
            &mut docs,
            api::Answer::Info {
                file: api::FileInfo {
                    path: api::ProviderPath::new("opaque-marker").unwrap(),
                    name: "Cargo.toml".into(),
                    kind: api::FileKind::File,
                    size: 0,
                    revision: None,
                    readonly: false,
                },
            },
        );
        assert!(matches!(
            command(&mut docs),
            Command::Request {
                operation: api::Operation::ParentPath { .. },
                ..
            }
        ));
        answer(&mut docs, api::Answer::Parent { path: None });
        assert!(
            matches!(command(&mut docs), Command::Request { operation: api::Operation::ResourceUri { path }, .. }
            if path.as_str() == "opaque\\project")
        );
        answer(
            &mut docs,
            api::Answer::Uri {
                uri: "file:///project".into(),
            },
        );
        command(&mut docs);
        let events = reply(
            &mut docs,
            Reply::DocumentChunk {
                stamp: descriptor.stamp,
                text: "code".into(),
                next: None,
            },
        );
        let DocumentEvent::Ready(ready) = &events[0] else {
            panic!("ready")
        };
        assert_eq!(ready.project.as_str(), "opaque\\project");
        assert_eq!(ready.root_uri, "file:///project");
        descriptor.stamp.version += 1;
        docs.receive(&Event::DocumentsChanged { revision: 2 });
        command(&mut docs);
        reply(
            &mut docs,
            Reply::Documents {
                documents: vec![descriptor],
                revision: 2,
                next: None,
            },
        );
        command(&mut docs);
        answer(
            &mut docs,
            api::Answer::Uri {
                uri: "file:///source".into(),
            },
        );
        assert!(
            matches!(command(&mut docs), Command::Request { operation: api::Operation::ResourceUri { path }, .. }
            if path.as_str() == "opaque\\project")
        );
    }
    #[test]
    fn same_path_and_uri_in_two_environments_never_share_scope_or_snapshot() {
        let first = document(1, "one", "a😀中");
        let second = document(2, "two", "other");
        let mut docs = listed(
            vec![root(20, "two"), root(10, "one")],
            vec![first.clone(), second.clone()],
        );
        for (descriptor, scope, text) in [(first, 10, "a😀中"), (second, 20, "other")] {
            paths(&mut docs, scope);
            assert!(
                matches!(command(&mut docs), Command::ReadDocument { stamp, offset: 0, .. } if stamp == descriptor.stamp)
            );
            let events = reply(
                &mut docs,
                Reply::DocumentChunk {
                    stamp: descriptor.stamp,
                    text: text.into(),
                    next: None,
                },
            );
            let DocumentEvent::Ready(ready) = events.into_iter().next().unwrap() else {
                panic!("ready")
            };
            assert_eq!(ready.root.scope, scope);
            assert_eq!(ready.descriptor.resource, descriptor.resource);
            assert_eq!(ready.text, text);
            assert_eq!(ready.uri, "file:///opaque%5C%E4%B8%AD");
        }
        assert_eq!(docs.loaded.len(), 2);
    }
    #[test]
    fn edits_during_chunk_reads_discard_partial_text_and_rediscover() {
        let descriptor = document(1, "one", "a😀中");
        let mut docs = listed(vec![root(10, "one")], vec![descriptor.clone()]);
        paths(&mut docs, 10);
        command(&mut docs);
        assert!(
            reply(
                &mut docs,
                Reply::DocumentChunk {
                    stamp: descriptor.stamp,
                    text: "a".into(),
                    next: Some(1)
                }
            )
            .is_empty()
        );
        assert!(matches!(
            command(&mut docs),
            Command::ReadDocument { offset: 1, .. }
        ));
        for revision in 2..102 {
            docs.receive(&Event::DocumentsChanged { revision });
        }
        assert!(
            reply(
                &mut docs,
                Reply::DocumentChunk {
                    stamp: descriptor.stamp,
                    text: "😀中".into(),
                    next: None
                }
            )
            .is_empty()
        );
        assert!(docs.loaded.is_empty());
        assert!(matches!(
            command(&mut docs),
            Command::Documents { after: None, .. }
        ));
    }
    #[test]
    fn trust_revocation_closes_buffers_without_restarting_a_process() {
        let descriptor = document(1, "one", "x");
        let mut docs = listed(vec![root(10, "one")], vec![descriptor.clone()]);
        paths(&mut docs, 10);
        command(&mut docs);
        reply(
            &mut docs,
            Reply::DocumentChunk {
                stamp: descriptor.stamp,
                text: "x".into(),
                next: None,
            },
        );
        docs.receive(&Event::RootsChanged);
        command(&mut docs);
        let mut untrusted = root(10, "one");
        untrusted
            .capabilities
            .retain(|c| *c != api::Capability::Process);
        let events = reply(
            &mut docs,
            Reply::Roots {
                roots: vec![untrusted],
                next: None,
            },
        );
        assert!(events.iter().any(|e| matches!(e, DocumentEvent::Closed(1))));
        command(&mut docs);
        reply(
            &mut docs,
            Reply::Documents {
                documents: vec![descriptor],
                revision: 2,
                next: None,
            },
        );
        assert!(
            docs.drive(&mut |_| panic!("untrusted resource must not start path resolution"))
                .is_empty()
        );
    }
    #[test]
    fn conflict_and_rejected_mailbox_restart_without_mixed_or_duplicate_chunks() {
        let mut docs = Documents::new(vec!["rust".into()]);
        assert_eq!(docs.drive(&mut |_| Err("full".into())).len(), 1);
        assert!(matches!(command(&mut docs), Command::Roots { .. }));
        reply(
            &mut docs,
            Reply::Roots {
                roots: vec![root(10, "one")],
                next: None,
            },
        );
        command(&mut docs);
        docs.receive(&Event::Reply {
            ticket: 1,
            result: Err(api::Failure::new(api::FailureKind::Conflict, "changed")),
        });
        assert!(matches!(
            command(&mut docs),
            Command::Documents {
                after: None,
                revision: None,
                ..
            }
        ));
    }
}

//! In-memory host for protocol/lifecycle tests; production uses the WIT imports.
use crate::loom::extension::types::*;
use serde_json::Value;
use std::{cell::RefCell, collections::BTreeMap};
#[derive(Debug)]
#[allow(dead_code)] // Capture the complete WIT answer, even when a test inspects only some fields.
pub enum Answer {
    Completions(u64, Vec<CompletionItem>, bool),
    Diagnostics(u64, Vec<Diagnostic>),
    Edits(u64, Vec<TextEdit>),
    Hover(u64, Option<HoverAnswer>),
    Symbols(u64, Vec<DocumentSymbol>),
    Hints(u64, Vec<InlayHint>),
    Language(u64, LanguageAnswer),
}
#[derive(Default)]
pub struct State {
    pub roots: Vec<RootRef>,
    pub documents: BTreeMap<u64, (DocumentRef, String)>,
    pub snapshots: BTreeMap<u64, (DocumentRef, String)>,
    pub settings: BTreeMap<String, String>,
    pub messages: Vec<(u64, Value)>,
    pub answers: Vec<Answer>,
    pub configurations: BTreeMap<String, ProviderConfiguration>,
    pub nodes: BTreeMap<u64, UiDescription>,
    pub ready_hover: Vec<bool>,
    pub spawned: Vec<(u64, ProcessRequest)>,
    pub killed: Vec<u64>,
    pub watches: BTreeMap<u64, WatchRequest>,
    pub timers: BTreeMap<u64, u32>,
    pub operations: Vec<(u64, WorkspaceOperation)>,
    pub diagnostics: Vec<(String, u64, Vec<Diagnostic>)>,
    pub logs: Vec<(u8, String)>,
    pub blocked: bool,
    pub fail_watch: bool,
    pub sequence: u64,
    pub time: u64,
    pub read_bytes: usize,
    pub files: std::collections::BTreeSet<(u64, String)>,
}
thread_local! {pub static STATE:RefCell<State>=RefCell::new(State::default());}
pub fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}
fn next(s: &mut State) -> u64 {
    s.sequence += 1;
    s.sequence
}
fn error(text: &str) -> HostError {
    HostError {
        kind: ErrorKind::ExtensionFailed,
        message: text.into(),
        committed: false,
    }
}
pub fn log(level: u8, message: &str) {
    state(|s| s.logs.push((level, message.into())));
}
pub fn monotonic_nanoseconds() -> u64 {
    state(|s| {
        s.time += 1000;
        s.time
    })
}
pub fn get_setting(key: &str) -> Option<String> {
    state(|s| s.settings.get(key).cloned())
}
pub fn list_roots() -> Vec<RootRef> {
    state(|s| s.roots.clone())
}
pub fn read_file(root: u64, path: &str, _: u32) -> Result<FileContent, HostError> {
    if path == "go.mod" || state(|s| s.files.contains(&(root, path.to_owned()))) {
        Ok(FileContent {
            text: String::new(),
            truncated: false,
        })
    } else {
        Err(error("missing marker"))
    }
}
pub fn capture_document(document: u64) -> Result<DocumentSnapshot, HostError> {
    state(|s| {
        let (reference, text) = s
            .documents
            .get(&document)
            .cloned()
            .ok_or(error("closed document"))?;
        let snapshot = next(s);
        let length = text.len() as u64;
        s.snapshots.insert(snapshot, (reference.clone(), text));
        Ok(DocumentSnapshot {
            document: reference,
            snapshot,
            length,
        })
    })
}
pub fn read_range(token: u64, range: ByteRange) -> Result<String, HostError> {
    state(|s| {
        let value = s
            .snapshots
            .get(&token)
            .and_then(|(_, t)| t.get(range.start as usize..range.end as usize))
            .map(str::to_owned)
            .ok_or(error("expired or invalid snapshot"));
        if let Ok(text) = &value {
            s.read_bytes += text.len();
        }
        value
    })
}
pub fn release_snapshot(token: u64) {
    state(|s| {
        s.snapshots.remove(&token);
    });
}
pub fn process_spawn(request: &ProcessRequest) -> Result<u64, HostError> {
    Ok(state(|s| {
        let id = next(s);
        s.spawned.push((id, request.clone()));
        id
    }))
}
pub fn process_write(process: u64, bytes: &[u8]) -> Result<u32, HostError> {
    state(|s| {
        if s.blocked {
            return Ok(0);
        }
        s.messages
            .push((process, serde_json::from_slice(bytes).unwrap()));
        Ok(bytes.len() as u32)
    })
}
pub fn process_kill(process: u64) {
    state(|s| s.killed.push(process));
}
pub fn timer_start(milliseconds: u32) -> Result<u64, HostError> {
    Ok(state(|s| {
        let id = next(s);
        s.timers.insert(id, milliseconds);
        id
    }))
}
pub fn timer_cancel(id: u64) {
    state(|s| {
        s.timers.remove(&id);
    });
}
pub fn watch_files(request: &WatchRequest) -> Result<u64, HostError> {
    state(|s| {
        if s.fail_watch {
            return Err(error("watch refused"));
        }
        let id = next(s);
        s.watches.insert(id, request.clone());
        Ok(id)
    })
}
pub fn unwatch_files(id: u64) {
    state(|s| {
        s.watches.remove(&id);
    });
}
pub fn perform_workspace_operation(operation: &WorkspaceOperation) -> Result<u64, HostError> {
    Ok(state(|s| {
        let id = next(s);
        s.operations.push((id, operation.clone()));
        id
    }))
}
pub fn configure_provider(value: &ProviderConfiguration) -> Result<(), HostError> {
    state(|s| {
        s.configurations
            .insert(value.provider.clone(), value.clone());
    });
    Ok(())
}
pub fn publish_diagnostics(
    _: &str,
    path: &str,
    version: u64,
    items: &[Diagnostic],
) -> Result<(), HostError> {
    state(|s| s.diagnostics.push((path.into(), version, items.to_vec())));
    Ok(())
}
pub fn provide_diagnostics(request: u64, items: &[Diagnostic]) -> Result<(), HostError> {
    state(|s| s.answers.push(Answer::Diagnostics(request, items.to_vec())));
    Ok(())
}
pub fn provide_completions(
    request: u64,
    items: &[CompletionItem],
    incomplete: bool,
) -> Result<(), HostError> {
    state(|s| {
        s.answers
            .push(Answer::Completions(request, items.to_vec(), incomplete))
    });
    Ok(())
}
pub fn provide_edits(request: u64, items: &[TextEdit]) -> Result<(), HostError> {
    state(|s| s.answers.push(Answer::Edits(request, items.to_vec())));
    Ok(())
}
pub fn provide_hover(request: u64, value: Option<&HoverAnswer>) -> Result<(), HostError> {
    state(|s| s.answers.push(Answer::Hover(request, value.cloned())));
    Ok(())
}
pub fn provide_symbols(request: u64, items: &[DocumentSymbol]) -> Result<(), HostError> {
    state(|s| s.answers.push(Answer::Symbols(request, items.to_vec())));
    Ok(())
}
pub fn provide_inlay_hints(request: u64, items: &[InlayHint]) -> Result<(), HostError> {
    state(|s| s.answers.push(Answer::Hints(request, items.to_vec())));
    Ok(())
}
pub fn provide_language(request: u64, value: &LanguageAnswer) -> Result<(), HostError> {
    state(|s| s.answers.push(Answer::Language(request, value.clone())));
    Ok(())
}
pub fn ui_create(_: &UiParent, value: &UiDescription) -> Result<u64, HostError> {
    Ok(state(|s| {
        let id = next(s);
        s.nodes.insert(id, value.clone());
        id
    }))
}
pub fn ui_update(id: u64, value: &UiDescription) -> Result<(), HostError> {
    state(|s| {
        if value.label.as_deref() == Some(&format!("{}: Ready", crate::NAME)) {
            s.ready_hover.push(
                s.configurations
                    .get(&format!("{}.hover", crate::ID))
                    .is_some_and(|c| c.enabled),
            );
        }
        s.nodes.insert(id, value.clone());
    });
    Ok(())
}
pub fn ui_remove(id: u64) -> Result<(), HostError> {
    state(|s| {
        s.nodes.remove(&id);
    });
    Ok(())
}
pub mod tree {
    pub use crate::loom::ui::tree::{Placement, Relation, Target};
    pub fn place(_: u64, _: &Placement) -> Result<(), String> {
        Ok(())
    }
}

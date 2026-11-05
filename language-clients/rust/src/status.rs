//! What the client says about its language service: a state and a line, per session.
//! Loom draws the icon, the words and the list; nothing here describes a widget.
#[cfg(test)]
use crate::host::status as service;
#[cfg(not(test))]
use crate::loom::service::status as service;
use std::collections::BTreeMap;

/// How a session is doing: `rank` 0 stopped, 1 ready, 2 working, 3 error.
#[derive(Clone, Default)]
pub struct Status {
    pub rank: u8,
    pub detail: String,
}

/// The sessions reported to Loom, each under a short key of its own.
#[derive(Default)]
pub struct Widgets {
    keys: BTreeMap<String, String>,
    next: u64,
}

impl Widgets {
    /// Report every session in `states` (with the documents it serves) and withdraw
    /// the ones that are gone.
    pub fn update(&mut self, states: BTreeMap<String, (Status, Vec<String>)>) {
        let gone: Vec<String> = self
            .keys
            .keys()
            .filter(|key| !states.contains_key(*key))
            .cloned()
            .collect();
        for key in gone {
            if let Some(id) = self.keys.remove(&key) {
                withdraw(&id);
            }
        }
        for (key, (status, contexts)) in states {
            let id = self
                .keys
                .entry(key)
                .or_insert_with(|| {
                    self.next += 1;
                    format!("session-{}", self.next)
                })
                .clone();
            report(&id, &status, &contexts);
        }
    }

    pub fn dispose(&mut self) {
        for (_, id) in std::mem::take(&mut self.keys) {
            withdraw(&id);
        }
    }
}

fn compact(value: &str, limit: usize) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > limit {
        text.chars()
            .take(limit - 1)
            .chain(std::iter::once('…'))
            .collect()
    } else {
        text
    }
}

fn state_of(status: &Status, detail: &str) -> service::State {
    match status.rank {
        1 => service::State::Ready,
        2 if detail == "Starting" || detail.starts_with("Restarting") => service::State::Starting,
        2 => service::State::Working,
        3 => service::State::Error,
        _ => service::State::Stopped,
    }
}

fn report(key: &str, status: &Status, contexts: &[String]) {
    let detail = compact(&status.detail, 80);
    let mut contexts = contexts.to_vec();
    contexts.sort();
    contexts.dedup();
    let result = service::publish(&service::Report {
        key: key.into(),
        state: state_of(status, &detail),
        detail: (!detail.is_empty()).then_some(detail),
        contexts: Some(contexts),
    });
    if let Err(error) = result {
        crate::host::log(1, &format!("Service status: {error}"));
    }
}

fn withdraw(key: &str) {
    if let Err(error) = service::withdraw(key) {
        crate::host::log(1, &format!("Service status: {error}"));
    }
}

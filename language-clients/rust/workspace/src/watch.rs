//! Plugin-private subscription ownership. Provider paths are never interpreted.
use crate::{Command, Event, Reply, api};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) enum Change {
    Paths(api::ScopeId, Option<Vec<api::ProviderPath>>),
    Error(String),
}
struct Watch {
    path: api::ProviderPath,
    wanted: bool,
    handle: Option<u64>,
    sequence: u64,
    pending: bool,
    failed: bool,
}
#[derive(Clone, Copy)]
struct Pending {
    scope: api::ScopeId,
    start: bool,
}
#[derive(Default)]
pub(crate) struct Watches {
    watches: BTreeMap<api::ScopeId, Watch>,
    tickets: BTreeMap<u64, Pending>,
    requests: BTreeMap<u64, Pending>,
}
impl Watches {
    pub fn sync(&mut self, roots: &[api::PresentedRoot]) -> Vec<Change> {
        let selected: BTreeSet<_> = roots
            .iter()
            .filter(|root| {
                root.capabilities.contains(&api::Capability::Watch)
                    && root.capabilities.contains(&api::Capability::Process)
            })
            .map(|root| root.scope)
            .collect();
        for (scope, watch) in &mut self.watches {
            watch.wanted = selected.contains(scope);
            if !watch.pending {
                watch.failed = false;
            }
        }
        let mut errors = vec![];
        for root in roots.iter().filter(|root| selected.contains(&root.scope)) {
            if self.watches.contains_key(&root.scope) {
                continue;
            }
            if self.watches.len() >= 16 {
                errors.push(Change::Error(
                    "Project monitoring supports at most sixteen roots".into(),
                ));
                break;
            }
            let api::ResourceRef::Provided { path, .. } = &root.resource else {
                continue;
            };
            self.watches.insert(
                root.scope,
                Watch {
                    path: path.clone(),
                    wanted: true,
                    handle: None,
                    sequence: 0,
                    pending: false,
                    failed: false,
                },
            );
        }
        errors
    }
    pub fn needs_drive(&self) -> bool {
        self.watches
            .values()
            .any(|watch| !watch.pending && !watch.failed && watch.wanted != watch.handle.is_some())
    }
    pub fn drive(
        &mut self,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<Change> {
        self.watches
            .retain(|_, watch| watch.wanted || watch.pending || watch.handle.is_some());
        let mut events = vec![];
        for (&scope, watch) in &mut self.watches {
            if watch.pending || watch.failed || watch.wanted == watch.handle.is_some() {
                continue;
            }
            let start = watch.wanted;
            let operation = if start {
                api::Operation::Watch {
                    path: watch.path.clone(),
                }
            } else {
                api::Operation::Unwatch {
                    watch: watch.handle.unwrap(),
                }
            };
            match submit(Command::Request {
                scope,
                operation,
                timeout_ms: 15_000,
            }) {
                Ok(ticket) => {
                    watch.pending = true;
                    self.tickets.insert(ticket, Pending { scope, start });
                }
                Err(error) => events.push(Change::Error(error)),
            }
        }
        events
    }
    pub fn receive(&mut self, event: &Event) -> Vec<Change> {
        let completed = match event {
            Event::Reply { ticket, result } => {
                let Some(pending) = self.tickets.remove(ticket) else {
                    return vec![];
                };
                match result {
                    Ok(Reply::Request { id }) => {
                        self.requests.insert(*id, pending);
                        return vec![];
                    }
                    Err(error) => (pending, Err(error.clone())),
                    _ => (
                        pending,
                        Err(api::Failure::new(
                            api::FailureKind::InvalidRequest,
                            "Invalid watch receipt",
                        )),
                    ),
                }
            }
            Event::Delivery {
                event: api::Delivery::Complete { request, result },
            } => {
                let Some(pending) = self.requests.remove(request) else {
                    return vec![];
                };
                (pending, result.clone())
            }
            Event::Delivery {
                event: api::Delivery::Notification { scope, event, .. },
            } => {
                let Some(watch) = self.watches.get_mut(scope) else {
                    return vec![];
                };
                return match event {
                    api::Notification::Changed {
                        watch: handle,
                        sequence,
                        paths,
                        rescan,
                    } if watch.handle == Some(*handle) && *sequence > watch.sequence => {
                        let reset = *rescan || *sequence != watch.sequence + 1;
                        watch.sequence = *sequence;
                        if watch.wanted {
                            vec![Change::Paths(*scope, (!reset).then(|| paths.clone()))]
                        } else {
                            vec![]
                        }
                    }
                    api::Notification::WatchStopped {
                        watch: handle,
                        error,
                    } if watch.handle == Some(*handle) => {
                        watch.handle = None;
                        watch.failed = true;
                        vec![
                            Change::Paths(*scope, None),
                            Change::Error(format!(
                                "Project monitoring stopped: {}. Reload the root to retry.",
                                error.message
                            )),
                        ]
                    }
                    _ => vec![],
                };
            }
            _ => return vec![],
        };
        let (pending, result) = completed;
        let Some(watch) = self.watches.get_mut(&pending.scope) else {
            return vec![];
        };
        watch.pending = false;
        match (pending.start, result) {
            (true, Ok(api::Answer::Started { handle })) => {
                watch.handle = Some(handle);
                watch.sequence = 0;
                vec![]
            }
            (false, Ok(api::Answer::Ok)) => {
                watch.handle = None;
                vec![]
            }
            (_, result) => {
                watch.failed = true;
                vec![Change::Error(match result {
                    Err(error) => error.message,
                    _ => "Invalid watch response".into(),
                })]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(scope: u64) -> api::PresentedRoot {
        api::PresentedRoot {
            scope,
            resource: api::ResourceRef::provided(
                api::EnvironmentKey::new("provider", &scope.to_string()).unwrap(),
                "opaque:root",
            )
            .unwrap(),
            name: "test".into(),
            capabilities: vec![api::Capability::Watch, api::Capability::Process],
        }
    }
    fn finish(watches: &mut Watches, ticket: u64, handle: u64) {
        watches.receive(&Event::Reply {
            ticket,
            result: Ok(Reply::Request { id: ticket + 100 }),
        });
        watches.receive(&Event::Delivery {
            event: api::Delivery::Complete {
                request: ticket + 100,
                result: Ok(api::Answer::Started { handle }),
            },
        });
    }
    #[test]
    fn scoped_sequences_and_late_start_cleanup_do_not_cross_connections() {
        let mut watches = Watches::default();
        watches.sync(&[root(1), root(2)]);
        let mut ticket = 0;
        watches.drive(&mut |_| {
            ticket += 1;
            Ok(ticket)
        });
        watches.sync(&[root(2)]);
        finish(&mut watches, 1, 7);
        finish(&mut watches, 2, 7);
        let mut stopped = vec![];
        watches.drive(&mut |command| {
            stopped.push(command);
            Ok(3)
        });
        assert!(matches!(
            &stopped[..],
            [Command::Request {
                scope: 1,
                operation: api::Operation::Unwatch { watch: 7 },
                ..
            }]
        ));
        let changed = |scope, sequence| Event::Delivery {
            event: api::Delivery::Notification {
                environment: scope,
                scope,
                event: api::Notification::Changed {
                    watch: 7,
                    sequence,
                    paths: vec![api::ProviderPath::new("opaque:file").unwrap()],
                    rescan: false,
                },
            },
        };
        assert!(matches!(
            &watches.receive(&changed(2, 1))[..],
            [Change::Paths(2, Some(_))]
        ));
        assert!(watches.receive(&changed(2, 1)).is_empty());
        assert!(watches.receive(&changed(1, 1)).is_empty());
        assert!(matches!(
            &watches.receive(&changed(2, 4))[..],
            [Change::Paths(2, None)]
        ));
        assert!(watches.receive(&changed(2, 3)).is_empty());
    }
}

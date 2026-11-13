//! Private language-plugin workspace transport; no host or SSH implementation.
//! The caller submits through its own guest mailbox and feeds later events back.
use api::session::{Command, Event, Reply};
pub use loog_workspace_api as api;
use std::collections::{BTreeMap, VecDeque};

mod framing;
pub use framing::Frames;
mod documents;
mod locations;
mod project;
pub mod settings;
mod watch;
pub use documents::{DocumentEvent, Documents, ReadyDocument, Settings};
pub use locations::{LocationResult, LocationUris};

const WINDOW: usize = 256 * 1024;
const CHUNK: usize = 16 * 1024;
const MAX_PROCESSES: usize = 4;

#[cfg(test)]
mod tests;

/// Plugin-private identity, distinct from host-local and provider handle spaces.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Process(pub u64);

#[derive(Debug)]
pub enum ProcessEvent {
    Output {
        process: Process,
        stderr: bool,
        bytes: Vec<u8>,
    },
    /// Space returned to the plugin's bounded input queue, not provider credit.
    Writable { process: Process, bytes: usize },
    Exited {
        process: Process,
        code: Option<i32>,
        error: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum Purpose {
    Spawn,
    Input(usize),
    Close,
    Kill,
}
#[derive(Clone, Copy)]
struct Pending {
    process: Process,
    purpose: Purpose,
    cancelled: bool,
}
struct Session {
    scope: api::ScopeId,
    handle: Option<u64>,
    credit: usize,
    input: VecDeque<u8>,
    writing: bool,
    closing: bool,
    closed: bool,
    stopping: bool,
    kill_sent: bool,
    failed: bool,
}

#[derive(Default)]
pub struct Processes {
    next: u64,
    sessions: BTreeMap<Process, Session>,
    tickets: BTreeMap<u64, Pending>,
    requests: BTreeMap<api::RequestId, Pending>,
}

impl Processes {
    pub fn can_start(&self) -> bool {
        self.sessions.len() < MAX_PROCESSES
    }
    /// A restart can await teardown instead of losing its prepared document.
    /// Retiring processes still count against the same hard resource limit.
    pub fn waiting_for_capacity(&self) -> bool {
        !self.can_start() && self.sessions.values().any(|session| session.stopping)
    }
    /// Teardown whose submission was rejected before admission needs another
    /// drive. Accepted operations wake their consumer through mailbox events.
    pub fn needs_drive(&self) -> bool {
        self.sessions.values().any(|s| s.stopping && !s.kill_sent)
    }
    pub fn start(
        &mut self,
        scope: api::ScopeId,
        operation: api::Operation,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Result<Process, String> {
        if !self.can_start() {
            return Err("At most four language processes may run".into());
        }
        if !matches!(operation, api::Operation::Spawn { .. }) {
            return Err("Expected a process spawn".into());
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or("Process identity space exhausted")?;
        let process = Process(self.next);
        let ticket = submit(Command::Request {
            scope,
            operation,
            timeout_ms: 30_000,
        })?;
        self.tickets.insert(
            ticket,
            Pending {
                process,
                purpose: Purpose::Spawn,
                cancelled: false,
            },
        );
        self.sessions.insert(
            process,
            Session {
                scope,
                handle: None,
                credit: 0,
                input: VecDeque::new(),
                writing: false,
                closing: false,
                closed: false,
                stopping: false,
                kill_sent: false,
                failed: false,
            },
        );
        Ok(process)
    }

    /// Accept bounded bytes before or after spawn completes. The caller retains
    /// any unaccepted suffix and resumes only after Writable; no polling needed.
    pub fn write(
        &mut self,
        process: Process,
        bytes: &[u8],
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Result<usize, String> {
        let session = self
            .sessions
            .get_mut(&process)
            .filter(|s| !s.stopping && !s.closing)
            .ok_or("Process input is closed")?;
        let previous = session.input.len();
        let count = bytes.len().min(WINDOW - session.input.len());
        session.input.extend(&bytes[..count]);
        if let Err(error) = self.flush(process, submit) {
            // submit() rejected this operation, so none of the appended bytes
            // was sent. Restore the queue before telling the caller to retry.
            self.sessions
                .get_mut(&process)
                .unwrap()
                .input
                .truncate(previous);
            return Err(error);
        }
        Ok(count)
    }

    pub fn close_input(
        &mut self,
        process: Process,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(&process)
            .ok_or("Process is unavailable")?;
        session.closing = true;
        self.flush(process, submit)
    }

    pub fn kill(
        &mut self,
        process: Process,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Result<(), String> {
        let Some(session) = self.sessions.get_mut(&process) else {
            return Ok(());
        };
        if session.kill_sent {
            return Ok(());
        }
        session.stopping = true;
        session.input.clear();
        if let Some(handle) = session.handle {
            let ticket = submit(Command::Request {
                scope: session.scope,
                operation: api::Operation::Kill { process: handle },
                timeout_ms: 30_000,
            })?;
            session.kill_sent = true;
            self.tickets.insert(
                ticket,
                Pending {
                    process,
                    purpose: Purpose::Kill,
                    cancelled: false,
                },
            );
        } else {
            for (&request, pending) in &mut self.requests {
                if pending.process == process
                    && matches!(pending.purpose, Purpose::Spawn)
                    && !pending.cancelled
                {
                    submit(Command::Cancel { request })?;
                    pending.cancelled = true;
                }
            }
        }
        Ok(())
    }

    pub fn contains(&self, process: Process) -> bool {
        self.sessions.contains_key(&process)
    }

    fn flush(
        &mut self,
        process: Process,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Result<(), String> {
        let Some(session) = self.sessions.get_mut(&process) else {
            return Ok(());
        };
        let Some(handle) = session.handle else {
            return Ok(());
        };
        if session.writing || session.stopping {
            return Ok(());
        }
        let (operation, purpose) = if !session.input.is_empty() && session.credit > 0 {
            let count = session.input.len().min(session.credit).min(CHUNK);
            (
                api::Operation::Input {
                    process: handle,
                    bytes: session.input.iter().take(count).copied().collect(),
                },
                Purpose::Input(count),
            )
        } else if session.closing && session.input.is_empty() && !session.closed {
            (
                api::Operation::CloseInput { process: handle },
                Purpose::Close,
            )
        } else {
            return Ok(());
        };
        let ticket = submit(Command::Request {
            scope: session.scope,
            operation,
            timeout_ms: 30_000,
        })?;
        match purpose {
            Purpose::Input(count) => {
                session.credit -= count;
                session.writing = true;
            }
            Purpose::Close => session.closed = true,
            _ => unreachable!(),
        }
        self.tickets.insert(
            ticket,
            Pending {
                process,
                purpose,
                cancelled: false,
            },
        );
        Ok(())
    }

    fn remove(&mut self, process: Process) {
        self.sessions.remove(&process);
        self.tickets.retain(|_, pending| pending.process != process);
        self.requests
            .retain(|_, pending| pending.process != process);
    }

    fn failed(
        &mut self,
        process: Process,
        error: String,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
        events: &mut Vec<ProcessEvent>,
    ) {
        // Never replay an input whose result is unknown. Best-effort termination
        // supplements host-owned retirement/lease cleanup of the provider handle.
        let _ = self.kill(process, submit);
        if let Some(session) = self.sessions.get_mut(&process) {
            if !session.failed {
                session.failed = true;
                events.push(ProcessEvent::Exited {
                    process,
                    code: None,
                    error: Some(error),
                });
            }
        }
    }

    /// Call once per guest dispatch as well as after receiving transport events.
    /// Only a submission rejected before acceptance is retried. Input operations
    /// with failed/unknown results are never replayed.
    pub fn drive(
        &mut self,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<ProcessEvent> {
        let mut events = Vec::new();
        for process in self.sessions.keys().copied().collect::<Vec<_>>() {
            if self.sessions[&process].stopping {
                let _ = self.kill(process, submit);
            } else if let Err(error) = self.flush(process, submit) {
                self.failed(process, error, submit, &mut events);
            }
        }
        events
    }

    pub fn receive(
        &mut self,
        event: Event,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<ProcessEvent> {
        let mut events = Vec::new();
        match event {
            Event::Reply { ticket, result } => {
                let Some(pending) = self.tickets.remove(&ticket) else {
                    return events;
                };
                match result {
                    Ok(Reply::Request { id }) => {
                        self.requests.insert(id, pending);
                        if matches!(pending.purpose, Purpose::Spawn)
                            && self
                                .sessions
                                .get(&pending.process)
                                .is_some_and(|s| s.stopping)
                        {
                            if let Err(error) = submit(Command::Cancel { request: id }) {
                                self.failed(pending.process, error, submit, &mut events);
                            } else if let Some(pending) = self.requests.get_mut(&id) {
                                pending.cancelled = true;
                            }
                        }
                    }
                    Err(error) if matches!(pending.purpose, Purpose::Spawn) => {
                        self.remove(pending.process);
                        events.push(ProcessEvent::Exited {
                            process: pending.process,
                            code: None,
                            error: Some(error.message),
                        });
                    }
                    Err(error) => self.failed(pending.process, error.message, submit, &mut events),
                    _ => self.failed(
                        pending.process,
                        "Invalid process request receipt".into(),
                        submit,
                        &mut events,
                    ),
                }
            }
            Event::Delivery {
                event: api::Delivery::Complete { request, result },
            } => {
                let Some(pending) = self.requests.remove(&request) else {
                    return events;
                };
                let Some(session) = self.sessions.get_mut(&pending.process) else {
                    return events;
                };
                let completed = match (pending.purpose, result) {
                    (Purpose::Spawn, Ok(api::Answer::Started { handle })) => {
                        session.handle = Some(handle);
                        if session.stopping {
                            // Cancellation can lose a race with successful spawn.
                            self.kill(pending.process, submit)
                        } else {
                            Ok(())
                        }
                    }
                    (Purpose::Input(count), Ok(api::Answer::Ok)) => {
                        if !session.stopping {
                            session.input.drain(..count);
                            events.push(ProcessEvent::Writable {
                                process: pending.process,
                                bytes: count,
                            });
                        }
                        session.writing = false;
                        Ok(())
                    }
                    (Purpose::Close | Purpose::Kill, Ok(api::Answer::Ok)) => Ok(()),
                    (Purpose::Spawn, Err(error)) => {
                        let notified = session.failed;
                        self.remove(pending.process);
                        if !notified {
                            events.push(ProcessEvent::Exited {
                                process: pending.process,
                                code: None,
                                error: Some(error.message),
                            });
                        }
                        Ok(())
                    }
                    (_, Err(error)) => Err(error.message),
                    _ => Err("Invalid process operation result".into()),
                };
                if let Err(error) = completed.and_then(|_| self.flush(pending.process, submit)) {
                    self.failed(pending.process, error, submit, &mut events);
                }
            }
            Event::Delivery {
                event: api::Delivery::Notification { scope, event, .. },
            } => {
                let handle = match &event {
                    api::Notification::Writable { handle, .. }
                    | api::Notification::Output { handle, .. }
                    | api::Notification::Exited { handle, .. } => *handle,
                    _ => return events,
                };
                let Some(process) = self
                    .sessions
                    .iter()
                    .find(|(_, s)| s.scope == scope && s.handle == Some(handle))
                    .map(|(id, _)| *id)
                else {
                    return events;
                };
                match event {
                    api::Notification::Writable { bytes, .. } => {
                        let session = self.sessions.get_mut(&process).unwrap();
                        if session.credit.saturating_add(bytes as usize) > WINDOW {
                            self.failed(
                                process,
                                "Provider input credit exceeds its window".into(),
                                submit,
                                &mut events,
                            );
                        } else {
                            session.credit += bytes as usize;
                            if let Err(error) = self.flush(process, submit) {
                                self.failed(process, error, submit, &mut events);
                            }
                        }
                    }
                    api::Notification::Output { stderr, bytes, .. } => {
                        if !self.sessions[&process].failed {
                            events.push(ProcessEvent::Output {
                                process,
                                stderr,
                                bytes,
                            });
                        }
                    }
                    api::Notification::Exited { code, .. } => {
                        let notified = self.sessions[&process].failed;
                        self.remove(process);
                        if !notified {
                            events.push(ProcessEvent::Exited {
                                process,
                                code,
                                error: None,
                            });
                        }
                    }
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
        events.extend(self.drive(submit));
        events
    }
}

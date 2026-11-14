//! Server applyEdit requests finish only after the host's buffer receipt.
//! Accepted mutations are never retried, including after process replacement.
use super::*;
use api::session::{DocumentVersion, Reply};

pub struct Response {
    pub process: Process,
    pub id: Value,
    pub result: Value,
}
struct Origin {
    process: Process,
    id: Value,
    stamp: DocumentVersion,
    title: String,
}
struct Mapping {
    origin: Origin,
    documents: Vec<rename::Document>,
}
struct Submitted {
    origin: Origin,
    cancelled: bool,
    cancel_sent: bool,
}
#[derive(Default)]
pub struct Edits {
    next: u64,
    locations: LocationUris,
    mapping: BTreeMap<u64, Mapping>,
    submitted: BTreeMap<u64, Submitted>,
}
fn response(origin: Origin, result: Result<(), String>) -> Response {
    Response {
        process: origin.process,
        id: origin.id,
        result: match result {
            Ok(()) => json!({"applied":true}),
            Err(reason) => json!({"applied":false,"failureReason":reason}),
        },
    }
}
impl Edits {
    pub fn start(
        &mut self,
        process: Process,
        id: Value,
        stamp: DocumentVersion,
        title: String,
        environment: api::EnvironmentKey,
        scopes: Vec<u64>,
        documents: Vec<rename::Document>,
    ) -> Result<(), String> {
        if self.mapping.len() + self.submitted.len() >= 4
            || self
                .mapping
                .values()
                .any(|m| m.origin.process == process && m.origin.id == id)
            || self
                .submitted
                .values()
                .any(|s| s.origin.process == process && s.origin.id == id)
            || !(id.as_i64().is_some() || id.as_str().is_some_and(|s| s.len() <= 128))
            || title.is_empty()
            || title.len() > 256
            || title.contains(['\0', '\r', '\n'])
        {
            return Err("Server edit request budget or identity is invalid".into());
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or("Edit identifier space exhausted")?;
        self.locations.start_with_scopes(
            self.next,
            stamp,
            scopes,
            environment,
            rename::uris(&documents),
        )?;
        self.mapping.insert(
            self.next,
            Mapping {
                origin: Origin {
                    process,
                    id,
                    stamp,
                    title,
                },
                documents,
            },
        );
        Ok(())
    }
    pub fn pending(&self, process: Process) -> bool {
        self.mapping.values().any(|m| m.origin.process == process)
            || self
                .submitted
                .values()
                .any(|s| s.origin.process == process && !s.cancelled)
    }
    pub fn cancel_process(&mut self, process: Process) {
        self.mapping.retain(|key, mapping| {
            if mapping.origin.process == process {
                self.locations.cancel(*key);
                false
            } else {
                true
            }
        });
        for submitted in self
            .submitted
            .values_mut()
            .filter(|s| s.origin.process == process)
        {
            submitted.cancelled = true;
        }
    }
    pub fn cancel_all(&mut self) {
        self.locations.cancel_all();
        self.mapping.clear();
        for submitted in self.submitted.values_mut() {
            submitted.cancelled = true;
        }
    }
    pub fn needs_drive(&self) -> bool {
        self.locations.needs_drive()
            || self
                .submitted
                .values()
                .any(|s| s.cancelled && !s.cancel_sent)
    }
    pub fn receive(
        &mut self,
        event: &Event,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<Response> {
        if let Event::Reply { ticket, result } = event {
            if let Some(submitted) = self.submitted.remove(ticket) {
                if submitted.cancelled {
                    return vec![];
                }
                let result = match result {
                    Ok(Reply::EditsApplied { .. }) => Ok(()),
                    Ok(_) => Err("Unexpected edit completion receipt".into()),
                    Err(reason) => Err(reason.message.clone()),
                };
                return vec![response(submitted.origin, result)];
            }
        }
        let results = self.locations.receive(event);
        self.resolved(results, submit)
    }
    pub fn drive(
        &mut self,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<Response> {
        for (ticket, pending) in &mut self.submitted {
            if pending.cancelled && !pending.cancel_sent {
                pending.cancel_sent =
                    submit(Command::CancelWorkspaceEdit { ticket: *ticket }).is_ok();
            }
        }
        let results = self.locations.drive(submit);
        self.resolved(results, submit)
    }
    fn resolved(
        &mut self,
        results: Vec<LocationResult>,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<Response> {
        let mut responses = vec![];
        for result in results {
            let Some(mapping) = self.mapping.remove(&result.request) else {
                continue;
            };
            let origin = mapping.origin;
            let ticket = result
                .locations
                .and_then(|locations| rename::resolved(mapping.documents, locations))
                .and_then(|documents| {
                    submit(Command::ApplyWorkspaceEdit {
                        origin: origin.stamp,
                        title: origin.title.clone(),
                        documents,
                    })
                });
            match ticket {
                Ok(ticket) => {
                    self.submitted.insert(
                        ticket,
                        Submitted {
                            origin,
                            cancelled: false,
                            cancel_sent: false,
                        },
                    );
                }
                Err(reason) => responses.push(response(origin, Err(reason))),
            }
        }
        responses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn start(edits: &mut Edits, process: u64, id: i64) {
        edits
            .start(
                Process(process),
                json!(id),
                DocumentVersion {
                    document: 1,
                    lifecycle: 2,
                    version: 3,
                },
                "Server edit".into(),
                api::EnvironmentKey::new("memory", "one").unwrap(),
                vec![1],
                vec![],
            )
            .unwrap();
    }
    #[test]
    fn submission_is_not_success_and_a_receipt_is_consumed_exactly_once() {
        let mut edits = Edits::default();
        start(&mut edits, 7, 4);
        assert!(
            edits
                .drive(&mut |cmd| {
                    assert!(matches!(cmd, Command::ApplyWorkspaceEdit { .. }));
                    Ok(10)
                })
                .is_empty()
        );
        assert!(edits.pending(Process(7)));
        assert!(
            edits
                .start(
                    Process(7),
                    json!(4),
                    DocumentVersion {
                        document: 1,
                        lifecycle: 2,
                        version: 3
                    },
                    "Duplicate".into(),
                    api::EnvironmentKey::new("memory", "one").unwrap(),
                    vec![1],
                    vec![]
                )
                .is_err()
        );
        let receipt = Event::Reply {
            ticket: 10,
            result: Ok(Reply::EditsApplied {
                transaction: 8,
                changed: 2,
            }),
        };
        let answer = edits.receive(&receipt, &mut |_| panic!("never replay"));
        assert_eq!(answer.len(), 1);
        assert_eq!(answer[0].id, json!(4));
        assert_eq!(answer[0].result["applied"], true);
        assert!(
            edits
                .receive(&receipt, &mut |_| panic!("never replay"))
                .is_empty()
        );
        assert!(!edits.pending(Process(7)));
        start(&mut edits, 7, 5);
        edits.drive(&mut |_| Ok(11));
        let answer = edits.receive(
            &Event::Reply {
                ticket: 11,
                result: Err(api::Failure::new(
                    api::FailureKind::Conflict,
                    "stale target",
                )),
            },
            &mut |_| unreachable!(),
        );
        assert_eq!(
            answer[0].result,
            json!({"applied":false,"failureReason":"stale target"})
        );
    }
    #[test]
    fn stopping_cancels_accepted_edits_with_retry_only_for_unaccepted_cancellation() {
        let mut edits = Edits::default();
        start(&mut edits, 9, 1);
        edits.drive(&mut |_| Ok(20));
        edits.cancel_process(Process(9));
        assert!(!edits.pending(Process(9)));
        assert!(edits.needs_drive());
        edits.drive(&mut |cmd| {
            assert!(matches!(cmd, Command::CancelWorkspaceEdit { ticket: 20 }));
            Err("busy".into())
        });
        assert!(edits.needs_drive());
        edits.drive(&mut |_| Ok(21));
        assert!(!edits.needs_drive());
        assert!(
            edits
                .receive(
                    &Event::Reply {
                        ticket: 20,
                        result: Ok(Reply::EditsApplied {
                            transaction: 8,
                            changed: 1
                        })
                    },
                    &mut |_| unreachable!()
                )
                .is_empty()
        );
    }
    #[test]
    fn unresolved_and_unacknowledged_requests_share_one_admission_budget() {
        let mut edits = Edits::default();
        for id in 0..4 {
            start(&mut edits, 1, id);
        }
        let mut ticket = 100;
        edits.drive(&mut |_| {
            ticket += 1;
            Ok(ticket)
        });
        assert!(
            edits
                .start(
                    Process(2),
                    json!(5),
                    DocumentVersion {
                        document: 1,
                        lifecycle: 2,
                        version: 3
                    },
                    "Server edit".into(),
                    api::EnvironmentKey::new("memory", "one").unwrap(),
                    vec![1],
                    vec![]
                )
                .is_err()
        );
        edits.cancel_all();
        assert!(!edits.pending(Process(1)));
    }
}

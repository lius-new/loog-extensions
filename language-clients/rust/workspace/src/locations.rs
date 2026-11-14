//! Resolve language-server URIs through the matching provider, with bounded
//! queues and cancellation even when a submission receipt arrives late.
use super::*;
use api::session::{DocumentVersion, ResourceLocation, ResourceRange};

pub struct LocationResult {
    pub request: u64,
    pub stamp: DocumentVersion,
    pub locations: Result<Vec<ResourceLocation>, String>,
}
struct Pending {
    ticket: Option<u64>,
    request: Option<u64>,
    cancelled: bool,
}
struct Job {
    stamp: DocumentVersion,
    scopes: Vec<u64>,
    scope_index: usize,
    attempts: usize,
    environment: api::EnvironmentKey,
    uris: VecDeque<(String, ResourceRange)>,
    resolved: BTreeMap<String, api::ProviderPath>,
    locations: Vec<ResourceLocation>,
    bytes: usize,
    output_bytes: usize,
    error: Option<String>,
    pending: Option<Pending>,
    cancelled: bool,
}
#[derive(Default)]
pub struct LocationUris {
    jobs: BTreeMap<u64, Job>,
}
impl LocationUris {
    pub fn start(
        &mut self,
        request: u64,
        stamp: DocumentVersion,
        scope: u64,
        environment: api::EnvironmentKey,
        uris: Vec<(String, ResourceRange)>,
    ) -> Result<(), String> {
        self.start_with_scopes(request, stamp, vec![scope], environment, uris)
    }
    pub fn start_with_scopes(
        &mut self,
        request: u64,
        stamp: DocumentVersion,
        scopes: Vec<u64>,
        environment: api::EnvironmentKey,
        uris: Vec<(String, ResourceRange)>,
    ) -> Result<(), String> {
        let bytes = uris.iter().map(|(uri, _)| uri.len() + 128).sum::<usize>();
        if self.jobs.contains_key(&request)
            || scopes.is_empty()
            || scopes.len() > 128
            || scopes
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != scopes.len()
            || self.jobs.len() >= 32
            || uris.len() > 256
            || bytes > 128 * 1024
            || self.jobs.values().map(|j| j.bytes).sum::<usize>() + bytes > 512 * 1024
            || uris.iter().any(|(uri, range)| {
                uri.is_empty() || uri.len() > 16 * 1024 || range.start > range.end
            })
        {
            return Err("Language location mapping budget or range is invalid".into());
        }
        self.jobs.insert(
            request,
            Job {
                stamp,
                scopes,
                scope_index: 0,
                attempts: 0,
                environment,
                uris: uris.into(),
                resolved: BTreeMap::new(),
                locations: vec![],
                bytes,
                output_bytes: 0,
                error: None,
                pending: None,
                cancelled: false,
            },
        );
        Ok(())
    }
    pub fn cancel(&mut self, request: u64) {
        if let Some(job) = self.jobs.get_mut(&request) {
            job.cancelled = true;
        }
    }
    pub fn cancel_all(&mut self) {
        for job in self.jobs.values_mut() {
            job.cancelled = true;
        }
    }
    pub fn needs_drive(&self) -> bool {
        self.jobs.values().any(|job| {
            job.pending.as_ref().is_none_or(|pending| {
                (pending.ticket.is_none() && pending.request.is_none())
                    || (job.cancelled && pending.request.is_some() && !pending.cancelled)
            })
        })
    }
    pub fn drive(
        &mut self,
        submit: &mut impl FnMut(Command) -> Result<u64, String>,
    ) -> Vec<LocationResult> {
        let mut events = vec![];
        for id in self.jobs.keys().copied().collect::<Vec<_>>() {
            let job = self.jobs.get_mut(&id).unwrap();
            if job.cancelled {
                if let Some(pending) = &mut job.pending {
                    if let Some(request) = pending.request
                        && !pending.cancelled
                    {
                        pending.cancelled = submit(Command::Cancel { request }).is_ok();
                    } else if pending.ticket.is_none() && pending.request.is_none() {
                        self.jobs.remove(&id);
                    }
                } else {
                    self.jobs.remove(&id);
                }
                continue;
            }
            while job.pending.is_none() {
                let Some((uri, range)) = job.uris.front() else {
                    break;
                };
                if let Some(path) = job.resolved.get(uri) {
                    let location = ResourceLocation {
                        resource: api::ResourceRef::Provided {
                            environment: job.environment.clone(),
                            path: path.clone(),
                        },
                        range: *range,
                    };
                    let bytes = serde_json::to_vec(&location)
                        .expect("validated resource")
                        .len();
                    if job.output_bytes + bytes > 192 * 1024 {
                        job.error =
                            Some("Resolved language locations exceed their output budget".into());
                        break;
                    }
                    job.output_bytes += bytes;
                    job.locations.push(location);
                    job.uris.pop_front();
                } else {
                    job.pending = Some(Pending {
                        ticket: None,
                        request: None,
                        cancelled: false,
                    });
                }
            }
            if job.attempts >= 512
                && job
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.ticket.is_none() && pending.request.is_none())
            {
                job.pending = None;
                job.error = Some("Location scope attempts exceed their budget".into());
            }
            let Some(pending) = &mut job.pending else {
                let job = self.jobs.remove(&id).unwrap();
                events.push(LocationResult {
                    request: id,
                    stamp: job.stamp,
                    locations: job.error.map_or_else(|| Ok(job.locations), Err),
                });
                continue;
            };
            if pending.ticket.is_none() && pending.request.is_none() {
                pending.ticket = submit(Command::Request {
                    scope: job.scopes[job.scope_index],
                    operation: api::Operation::ResolveUri {
                        uri: job.uris.front().unwrap().0.clone(),
                    },
                    timeout_ms: 15_000,
                })
                .ok();
                if pending.ticket.is_some() {
                    job.attempts += 1;
                }
            }
        }
        events
    }
    pub fn receive(&mut self, event: &Event) -> Vec<LocationResult> {
        let Some(id) = self.jobs.iter().find_map(|(id, job)| {
            let pending = job.pending.as_ref()?;
            match event {
                Event::Reply { ticket, .. } if pending.ticket == Some(*ticket) => Some(*id),
                Event::Delivery {
                    event: api::Delivery::Complete { request, .. },
                } if pending.request == Some(*request) => Some(*id),
                _ => None,
            }
        }) else {
            return vec![];
        };
        let job = self.jobs.get_mut(&id).unwrap();
        if let Event::Reply {
            result: Ok(Reply::Request { id }),
            ..
        } = event
        {
            let pending = job.pending.as_mut().unwrap();
            pending.ticket = None;
            pending.request = Some(*id);
            return vec![];
        }
        if job.cancelled {
            self.jobs.remove(&id);
            return vec![];
        }
        let error = match event {
            Event::Delivery {
                event:
                    api::Delivery::Complete {
                        result: Ok(api::Answer::Path { path }),
                        ..
                    },
            } => {
                job.resolved
                    .insert(job.uris.front().unwrap().0.clone(), path.clone());
                job.scope_index = 0;
                job.pending = None;
                return vec![];
            }
            Event::Reply {
                result: Err(error), ..
            }
            | Event::Delivery {
                event:
                    api::Delivery::Complete {
                        result: Err(error), ..
                    },
            } => {
                if matches!(
                    error.kind,
                    api::FailureKind::PermissionDenied
                        | api::FailureKind::NotFound
                        | api::FailureKind::Unavailable
                ) && job.scope_index + 1 < job.scopes.len()
                    && job.attempts < 512
                {
                    job.scope_index += 1;
                    job.pending = None;
                    return vec![];
                }
                error.message.clone()
            }
            _ => "Unexpected location mapping response".into(),
        };
        let job = self.jobs.remove(&id).unwrap();
        vec![LocationResult {
            request: id,
            stamp: job.stamp,
            locations: Err(error),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp() -> DocumentVersion {
        DocumentVersion {
            document: 1,
            lifecycle: 2,
            version: 3,
        }
    }
    fn ranges() -> Vec<(String, ResourceRange)> {
        let range = ResourceRange {
            start: api::session::ResourcePosition {
                line: 2,
                character: 1,
            },
            end: api::session::ResourcePosition {
                line: 2,
                character: 3,
            },
            encoding: api::session::ColumnEncoding::Utf16,
        };
        vec![("file:///same%5C%E4%B8%AD".into(), range); 2]
    }
    fn reply(map: &mut LocationUris, ticket: u64, request: u64) {
        assert!(
            map.receive(&Event::Reply {
                ticket,
                result: Ok(Reply::Request { id: request })
            })
            .is_empty()
        );
    }
    #[test]
    fn identical_uris_use_their_scope_and_duplicate_targets_share_one_resolution() {
        let mut map = LocationUris::default();
        for (id, environment) in [(1, "one"), (2, "two")] {
            map.start(
                id,
                stamp(),
                id * 10,
                api::EnvironmentKey::new("provider", environment).unwrap(),
                ranges(),
            )
            .unwrap();
        }
        let mut sent = vec![];
        map.drive(&mut |command| {
            sent.push(command);
            Ok(sent.len() as u64)
        });
        assert!(matches!(sent[0], Command::Request { scope: 10, .. }));
        assert!(matches!(sent[1], Command::Request { scope: 20, .. }));
        reply(&mut map, 1, 101);
        reply(&mut map, 2, 102);
        for request in [101, 102] {
            map.receive(&Event::Delivery {
                event: api::Delivery::Complete {
                    request,
                    result: Ok(api::Answer::Path {
                        path: api::ProviderPath::new("opaque\\中").unwrap(),
                    }),
                },
            });
        }
        let done = map.drive(&mut |_| panic!("duplicate URI must be cached"));
        assert_eq!(done.len(), 2);
        let a = done[0].locations.as_ref().unwrap();
        let b = done[1].locations.as_ref().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], a[1]);
        assert_ne!(a[0].resource, b[0].resource);
        assert_eq!(a[0].range.encoding, api::session::ColumnEncoding::Utf16);
    }
    #[test]
    fn cancellation_waits_for_late_submission_receipt_and_never_publishes() {
        let mut map = LocationUris::default();
        map.start(
            1,
            stamp(),
            10,
            api::EnvironmentKey::new("provider", "one").unwrap(),
            ranges(),
        )
        .unwrap();
        map.drive(&mut |_| Err("busy".into()));
        assert!(map.needs_drive());
        map.drive(&mut |_| Ok(20));
        map.cancel(1);
        assert!(
            map.drive(&mut |_| panic!("still waiting for receipt"))
                .is_empty()
        );
        reply(&mut map, 20, 50);
        map.drive(&mut |command| {
            assert!(matches!(command, Command::Cancel { request: 50 }));
            Ok(21)
        });
        assert!(
            map.receive(&Event::Delivery {
                event: api::Delivery::Complete {
                    request: 50,
                    result: Ok(api::Answer::Path {
                        path: api::ProviderPath::new("late").unwrap()
                    })
                }
            })
            .is_empty()
        );
        assert!(map.jobs.is_empty());
    }
    #[test]
    fn denied_or_oversized_results_fail_without_a_partial_navigation_answer() {
        let mut map = LocationUris::default();
        let env = api::EnvironmentKey::new("provider", "one").unwrap();
        let mut oversized = ranges();
        oversized[0].0 = "x".repeat(16 * 1024 + 1);
        assert!(map.start(1, stamp(), 10, env.clone(), oversized).is_err());
        map.start(1, stamp(), 10, env, ranges()).unwrap();
        map.drive(&mut |_| Ok(1));
        reply(&mut map, 1, 2);
        let result = map.receive(&Event::Delivery {
            event: api::Delivery::Complete {
                request: 2,
                result: Err(api::Failure::new(
                    api::FailureKind::PermissionDenied,
                    "outside root",
                )),
            },
        });
        assert!(result[0].locations.is_err());
        assert!(map.jobs.is_empty());
    }
    #[test]
    fn provider_path_expansion_is_bounded_even_for_one_cached_uri() {
        let mut map = LocationUris::default();
        map.start(
            1,
            stamp(),
            10,
            api::EnvironmentKey::new("provider", "one").unwrap(),
            vec![ranges()[0].clone(); 256],
        )
        .unwrap();
        map.drive(&mut |_| Ok(1));
        reply(&mut map, 1, 2);
        map.receive(&Event::Delivery {
            event: api::Delivery::Complete {
                request: 2,
                result: Ok(api::Answer::Path {
                    path: api::ProviderPath::new("x".repeat(32 * 1024)).unwrap(),
                }),
            },
        });
        let events = map.drive(&mut |_| panic!("cached URI"));
        assert!(events[0].locations.is_err());
        assert!(map.jobs.is_empty());
    }

    #[test]
    fn another_authorized_scope_resolves_only_after_a_scoped_failure() {
        let mut map = LocationUris::default();
        let env = api::EnvironmentKey::new("provider", "one").unwrap();
        map.start_with_scopes(1, stamp(), vec![10, 20], env.clone(), ranges())
            .unwrap();
        map.drive(&mut |command| {
            assert!(matches!(command, Command::Request { scope: 10, .. }));
            Ok(1)
        });
        reply(&mut map, 1, 100);
        assert!(
            map.receive(&Event::Delivery {
                event: api::Delivery::Complete {
                    request: 100,
                    result: Err(api::Failure::new(
                        api::FailureKind::PermissionDenied,
                        "outside first root"
                    )),
                }
            })
            .is_empty()
        );
        map.drive(&mut |command| {
            assert!(matches!(command, Command::Request { scope: 20, .. }));
            Ok(2)
        });
        reply(&mut map, 2, 101);
        map.receive(&Event::Delivery {
            event: api::Delivery::Complete {
                request: 101,
                result: Ok(api::Answer::Path {
                    path: api::ProviderPath::new("dependency\\中").unwrap(),
                }),
            },
        });
        let result = map.drive(&mut |_| panic!("cached duplicate"));
        assert_eq!(
            result[0].locations.as_ref().unwrap()[0]
                .resource
                .environment(),
            Some(&env)
        );

        map.start_with_scopes(2, stamp(), vec![10, 20], env, ranges())
            .unwrap();
        map.drive(&mut |_| Ok(3));
        reply(&mut map, 3, 102);
        let result = map.receive(&Event::Delivery {
            event: api::Delivery::Complete {
                request: 102,
                result: Err(api::Failure::new(
                    api::FailureKind::InvalidRequest,
                    "malformed URI",
                )),
            },
        });
        assert!(result[0].locations.is_err());
        assert!(map.jobs.is_empty());
    }

    #[test]
    fn mapping_many_roots_and_targets_stops_at_the_total_attempt_limit() {
        let mut map = LocationUris::default();
        let env = api::EnvironmentKey::new("provider", "one").unwrap();
        let uris = (0..256)
            .map(|n| (format!("file:///target-{n}"), ranges()[0].1))
            .collect();
        map.start_with_scopes(1, stamp(), vec![10, 20, 30], env, uris)
            .unwrap();
        let mut count = 0u64;
        loop {
            let mut scope = 0;
            let events = map.drive(&mut |command| {
                let Command::Request { scope: value, .. } = command else {
                    panic!("request")
                };
                scope = value;
                count += 1;
                Ok(count)
            });
            if !events.is_empty() {
                assert!(events[0].locations.is_err());
                break;
            }
            assert!(count <= 512);
            reply(&mut map, count, count + 1000);
            let result = if scope == 30 {
                Ok(api::Answer::Path {
                    path: api::ProviderPath::new(format!("resolved-{count}")).unwrap(),
                })
            } else {
                Err(api::Failure::new(
                    api::FailureKind::NotFound,
                    "another root",
                ))
            };
            let events = map.receive(&Event::Delivery {
                event: api::Delivery::Complete {
                    request: count + 1000,
                    result,
                },
            });
            if !events.is_empty() {
                assert!(events[0].locations.is_err());
                break;
            }
        }
        assert_eq!(count, 512);
        assert!(map.jobs.is_empty());
    }
}

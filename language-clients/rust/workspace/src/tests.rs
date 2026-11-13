use super::*;

#[derive(Default)]
struct Mailbox {
    next: u64,
    commands: Vec<(u64, Command)>,
    full: bool,
}
impl Mailbox {
    fn submit(&mut self, command: Command) -> Result<u64, String> {
        if self.full {
            return Err("full".into());
        }
        self.next += 1;
        self.commands.push((self.next, command));
        Ok(self.next)
    }
    fn start(&mut self, driver: &mut Processes, scope: u64) -> Process {
        driver
            .start(
                scope,
                api::Operation::Spawn {
                    program: "language-server".into(),
                    args: vec![],
                    cwd: api::ProviderPath::new("opaque-root").unwrap(),
                    env: vec![],
                },
                &mut |c| self.submit(c),
            )
            .unwrap()
    }
    fn receive(&mut self, driver: &mut Processes, event: Event) -> Vec<ProcessEvent> {
        driver.receive(event, &mut |c| self.submit(c))
    }
    fn receipt(&mut self, driver: &mut Processes, ticket: u64, request: u64) -> Vec<ProcessEvent> {
        self.receive(
            driver,
            Event::Reply {
                ticket,
                result: Ok(Reply::Request { id: request }),
            },
        )
    }
    fn complete(
        &mut self,
        driver: &mut Processes,
        request: u64,
        result: Result<api::Answer, api::Failure>,
    ) -> Vec<ProcessEvent> {
        self.receive(
            driver,
            Event::Delivery {
                event: api::Delivery::Complete { request, result },
            },
        )
    }
    fn notify(
        &mut self,
        driver: &mut Processes,
        scope: u64,
        event: api::Notification,
    ) -> Vec<ProcessEvent> {
        self.receive(
            driver,
            Event::Delivery {
                event: api::Delivery::Notification {
                    environment: scope,
                    scope,
                    event,
                },
            },
        )
    }
    fn started(&mut self, driver: &mut Processes, scope: u64, handle: u64) -> Process {
        let process = self.start(driver, scope);
        let ticket = self.next;
        self.receipt(driver, ticket, ticket + 100);
        self.complete(driver, ticket + 100, Ok(api::Answer::Started { handle }));
        process
    }
}

#[test]
fn four_process_restart_waits_for_retirement_without_exceeding_the_limit() {
    let mut driver = Processes::default();
    let mut mailbox = Mailbox::default();
    let processes: Vec<_> = (1..=4)
        .map(|scope| mailbox.started(&mut driver, scope, scope))
        .collect();
    assert!(!driver.can_start());
    assert!(!driver.waiting_for_capacity());
    driver
        .kill(processes[0], &mut |c| mailbox.submit(c))
        .unwrap();
    assert!(driver.waiting_for_capacity());
    assert!(
        !driver.can_start(),
        "unconfirmed cleanup cannot free a slot"
    );
    mailbox.notify(
        &mut driver,
        1,
        api::Notification::Exited {
            handle: 1,
            code: None,
        },
    );
    assert!(driver.can_start());
    mailbox.started(&mut driver, 1, 5);
    assert!(!driver.can_start());
    assert!(!driver.waiting_for_capacity());
    assert_eq!(driver.sessions.len(), 4);
}

#[test]
fn bounded_input_waits_for_spawn_and_credit_and_closes_only_after_acknowledged_bytes() {
    let mut driver = Processes::default();
    let mut mailbox = Mailbox::default();
    let process = mailbox.start(&mut driver, 1);
    assert_eq!(
        driver
            .write(process, &vec![0xFF; WINDOW + 1], &mut |c| mailbox.submit(c))
            .unwrap(),
        WINDOW
    );
    assert_eq!(
        driver
            .write(process, &[1], &mut |c| mailbox.submit(c))
            .unwrap(),
        0
    );
    driver
        .close_input(process, &mut |c| mailbox.submit(c))
        .unwrap();
    assert_eq!(mailbox.commands.len(), 1);
    mailbox.receipt(&mut driver, 1, 100);
    mailbox.complete(&mut driver, 100, Ok(api::Answer::Started { handle: 7 }));
    assert_eq!(mailbox.commands.len(), 1);
    mailbox.notify(
        &mut driver,
        1,
        api::Notification::Writable {
            handle: 7,
            bytes: WINDOW as u32,
        },
    );
    let mut transferred = 0;
    while transferred < WINDOW {
        let (ticket, command) = mailbox.commands.last().unwrap().clone();
        let Command::Request {
            operation:
                api::Operation::Input {
                    process: handle,
                    bytes,
                },
            ..
        } = command
        else {
            panic!("input")
        };
        assert_eq!(handle, 7);
        assert_eq!(bytes, vec![0xFF; CHUNK]);
        mailbox.receipt(&mut driver, ticket, ticket + 100);
        // Credits can arrive before the operation receipt without resending it.
        let before = mailbox.commands.len();
        mailbox.notify(
            &mut driver,
            1,
            api::Notification::Writable {
                handle: 7,
                bytes: CHUNK as u32,
            },
        );
        assert_eq!(mailbox.commands.len(), before);
        let events = mailbox.complete(&mut driver, ticket + 100, Ok(api::Answer::Ok));
        assert!(
            matches!(&events[..], [ProcessEvent::Writable { process: p, bytes: CHUNK }] if *p == process)
        );
        transferred += CHUNK;
    }
    assert!(matches!(
        &mailbox.commands.last().unwrap().1,
        Command::Request {
            operation: api::Operation::CloseInput { process: 7 },
            ..
        }
    ));
    assert!(
        driver
            .write(process, &[1], &mut |c| mailbox.submit(c))
            .is_err()
    );
}

#[test]
fn cancel_before_spawn_receipt_kills_a_late_success_without_replaying_queued_input() {
    let mut driver = Processes::default();
    let mut mailbox = Mailbox::default();
    let process = mailbox.start(&mut driver, 1);
    driver
        .write(process, b"never send", &mut |c| mailbox.submit(c))
        .unwrap();
    driver.kill(process, &mut |c| mailbox.submit(c)).unwrap();
    mailbox.receipt(&mut driver, 1, 100);
    assert!(matches!(
        &mailbox.commands.last().unwrap().1,
        Command::Cancel { request: 100 }
    ));
    mailbox.complete(&mut driver, 100, Ok(api::Answer::Started { handle: 7 }));
    assert!(matches!(
        &mailbox.commands.last().unwrap().1,
        Command::Request {
            operation: api::Operation::Kill { process: 7 },
            ..
        }
    ));
    mailbox.notify(
        &mut driver,
        1,
        api::Notification::Writable {
            handle: 7,
            bytes: WINDOW as u32,
        },
    );
    assert!(!mailbox.commands.iter().any(|(_, c)| matches!(
        c,
        Command::Request {
            operation: api::Operation::Input { .. },
            ..
        }
    )));
    let events = mailbox.notify(
        &mut driver,
        1,
        api::Notification::Exited {
            handle: 7,
            code: None,
        },
    );
    assert!(matches!(&events[..], [ProcessEvent::Exited { process: p, .. }] if *p == process));
    assert!(!driver.contains(process));
}

#[test]
fn identical_provider_handles_are_routed_by_scope_and_late_output_is_ignored() {
    let mut driver = Processes::default();
    let mut mailbox = Mailbox::default();
    let first = mailbox.started(&mut driver, 1, 7);
    let second = mailbox.started(&mut driver, 2, 7);
    let output = || api::Notification::Output {
        handle: 7,
        stderr: true,
        bytes: vec![0, 255],
    };
    assert!(
        matches!(&mailbox.notify(&mut driver, 2, output())[..], [ProcessEvent::Output { process, stderr: true, bytes }] if *process == second && bytes == &[0, 255])
    );
    assert!(mailbox.notify(&mut driver, 3, output()).is_empty());
    mailbox.notify(
        &mut driver,
        1,
        api::Notification::Exited {
            handle: 7,
            code: Some(0),
        },
    );
    assert!(!driver.contains(first));
    assert!(driver.contains(second));
    assert!(mailbox.notify(&mut driver, 1, output()).is_empty());
}

#[test]
fn rejected_submission_accepts_no_bytes_and_unknown_input_results_terminate_without_replay() {
    let mut driver = Processes::default();
    let mut mailbox = Mailbox::default();
    let process = mailbox.started(&mut driver, 1, 7);
    mailbox.notify(
        &mut driver,
        1,
        api::Notification::Writable {
            handle: 7,
            bytes: WINDOW as u32,
        },
    );
    mailbox.full = true;
    assert!(
        driver
            .write(process, b"once", &mut |c| mailbox.submit(c))
            .is_err()
    );
    assert!(driver.sessions[&process].input.is_empty());
    mailbox.full = false;
    driver
        .write(process, b"once", &mut |c| mailbox.submit(c))
        .unwrap();
    let ticket = mailbox.next;
    mailbox.receipt(&mut driver, ticket, 100);
    let events = mailbox.complete(
        &mut driver,
        100,
        Err(api::Failure::new(
            api::FailureKind::UnknownOutcome,
            "lost input receipt",
        )),
    );
    assert!(matches!(
        &events[..],
        [ProcessEvent::Exited { error: Some(_), .. }]
    ));
    assert_eq!(
        mailbox
            .commands
            .iter()
            .filter(|(_, c)| matches!(
                c,
                Command::Request {
                    operation: api::Operation::Input { .. },
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(
        mailbox
            .notify(
                &mut driver,
                1,
                api::Notification::Exited {
                    handle: 7,
                    code: None
                }
            )
            .is_empty()
    );
    assert!(!driver.contains(process));
}

#[test]
fn teardown_retries_only_a_control_submission_rejected_before_acceptance() {
    let mut driver = Processes::default();
    let mut mailbox = Mailbox::default();
    let process = mailbox.started(&mut driver, 1, 7);
    mailbox.full = true;
    assert!(driver.kill(process, &mut |c| mailbox.submit(c)).is_err());
    assert!(driver.contains(process));
    mailbox.full = false;
    driver.drive(&mut |c| mailbox.submit(c));
    assert!(matches!(
        &mailbox.commands.last().unwrap().1,
        Command::Request {
            operation: api::Operation::Kill { process: 7 },
            ..
        }
    ));
    let count = mailbox.commands.len();
    driver.drive(&mut |c| mailbox.submit(c));
    assert_eq!(mailbox.commands.len(), count);
}

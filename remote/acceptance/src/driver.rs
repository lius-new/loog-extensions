//! Exercise the same private transport code later embedded in language guests.
//! The provider remains a real WASM component communicating through SSH.
use super::*;
use loog_language_workspace::{ProcessEvent, Processes};

pub(super) fn check(h: &mut Harness) {
    let scope = h.scope.unwrap();
    let root = h.router.scope(scope).unwrap().1.clone();
    let mut driver = Processes::default();
    let mut tickets = 0u64;
    let mut submissions = Vec::new();
    let enqueue = |tickets: &mut u64, queue: &mut Vec<_>, command| {
        *tickets += 1;
        queue.push((*tickets, command));
        Ok(*tickets)
    };
    let process = driver.start(scope, api::Operation::Spawn {
        program: "python3".into(),
        args: vec!["-u".into(), "-c".into(), "import sys; data=sys.stdin.buffer.read(); sys.stdout.buffer.write(data); sys.stdout.buffer.flush(); sys.stderr.write('传输完成')".into()],
        cwd: root,
        env: vec![],
    }, &mut |c| enqueue(&mut tickets, &mut submissions, c)).unwrap();
    let bytes: Vec<_> = (0..600_000).map(|i| (i % 256) as u8).collect();
    let mut sent = 0;
    let mut writable = true;
    let mut closed = false;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exited = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while !exited {
        assert!(Instant::now() < deadline, "language transport timed out");
        if writable && sent < bytes.len() {
            sent += driver
                .write(process, &bytes[sent..], &mut |c| {
                    enqueue(&mut tickets, &mut submissions, c)
                })
                .unwrap();
            writable = false;
        }
        if sent == bytes.len() && !closed {
            driver
                .close_input(process, &mut |c| enqueue(&mut tickets, &mut submissions, c))
                .unwrap();
            closed = true;
        }
        let mut events = Vec::new();
        for (ticket, command) in std::mem::take(&mut submissions) {
            let result = match command {
                Command::Request {
                    scope,
                    operation,
                    timeout_ms,
                } => h
                    .router
                    .submit(
                        &h.consumer,
                        scope,
                        operation,
                        h.clock.elapsed().as_millis() as u64,
                        timeout_ms as u64,
                    )
                    .map(|id| Reply::Request { id }),
                Command::Cancel { request } => {
                    h.router.cancel(&h.consumer, request).map(|_| Reply::Done)
                }
                _ => panic!("unexpected transport command"),
            };
            events.extend(driver.receive(Event::Reply { ticket, result }, &mut |c| {
                enqueue(&mut tickets, &mut submissions, c)
            }));
        }
        h.tick(vec![]);
        for event in h.router.poll(&h.consumer, 32) {
            events.extend(driver.receive(Event::Delivery { event }, &mut |c| {
                enqueue(&mut tickets, &mut submissions, c)
            }));
        }
        events.extend(driver.drive(&mut |c| enqueue(&mut tickets, &mut submissions, c)));
        for event in events {
            match event {
                ProcessEvent::Writable { process: p, .. } => {
                    assert_eq!(p, process);
                    writable = true;
                }
                ProcessEvent::Output {
                    process: p,
                    stderr: error,
                    bytes,
                } => {
                    assert_eq!(p, process);
                    if error {
                        stderr.extend(bytes);
                    } else {
                        stdout.extend(bytes);
                    }
                }
                ProcessEvent::Exited {
                    process: p,
                    code,
                    error,
                } => {
                    assert_eq!(p, process);
                    assert_eq!(error, None);
                    assert_eq!(code, Some(0));
                    exited = true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(stdout, bytes);
    assert_eq!(stderr, "传输完成".as_bytes());
    assert!(!driver.contains(process));
}

//! Opt-in, real WASM -> generic tool -> SSH -> bundled Agent acceptance.
//! Target/config/profile are supplied by the caller, never committed credentials.
use loog_extension_host as host;
mod driver;
mod language;
mod lifecycle;
use loog_workspace_api::{
    self as api,
    session::{Command, Event, Reply},
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Child, Command as Process, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

struct Storage(String);
impl host::StoragePort for Storage {
    fn load(&mut self, scope: &str) -> BTreeMap<String, String> {
        assert_eq!(scope, "user");
        [("profile".into(), self.0.clone())].into()
    }
    fn save(&mut self, scope: &str, _: &BTreeMap<String, String>) -> Result<(), String> {
        assert_eq!(scope, "user");
        Ok(())
    }
}
struct Harness {
    instance: host::Instance,
    router: api::Router,
    owner: api::Owner,
    consumer: api::Owner,
    scope: Option<u64>,
    child: Option<(u64, Child)>,
    incoming: Option<Receiver<host::Inbound>>,
    readers: Vec<std::thread::JoinHandle<()>>,
    clock: Instant,
    notifications: Vec<api::Notification>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Some((_, child)) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.incoming = None;
        for reader in self.readers.drain(..) {
            reader.join().unwrap();
        }
    }
}
impl Harness {
    fn tick(&mut self, mut batch: Vec<host::Inbound>) {
        batch.extend(self.instance.poll_timers().0);
        if let Some(incoming) = &self.incoming {
            batch.extend(incoming.try_iter().take(32));
        }
        if let Some((id, child)) = &mut self.child {
            if let Some(status) = child.try_wait().unwrap() {
                self.instance.process_ended(*id);
                batch.push(host::Inbound::ProcessExited(host::ProcessExit {
                    process: *id,
                    code: status.code(),
                    killed: false,
                    error: None,
                }));
                self.child = None;
            }
        }
        self.instance.dispatch(&batch).unwrap();
        for (level, text) in self.instance.take_logs() {
            assert!(level < 2, "plugin error: {text}");
        }
        for control in self.instance.take_process_controls() {
            match control {
                host::ProcessControl::LaunchTool {
                    process,
                    program,
                    args,
                } => {
                    assert!(self.child.is_none());
                    assert_eq!(program, "ssh");
                    let mut command = Process::new(program);
                    command.args([
                        "-F",
                        &std::env::var("LOOG_SSH_CONFIG").expect("SSH test config"),
                    ]);
                    command
                        .args(args)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        command.creation_flags(0x08000000);
                    }
                    let mut child = command.spawn().unwrap();
                    let streams: Vec<(Box<dyn Read + Send>, host::OutputChannel)> = vec![
                        (
                            Box::new(child.stdout.take().unwrap()),
                            host::OutputChannel::Stdout,
                        ),
                        (
                            Box::new(child.stderr.take().unwrap()),
                            host::OutputChannel::Stderr,
                        ),
                    ];
                    let (sender, receiver) = mpsc::sync_channel(32);
                    for (mut stream, channel) in streams {
                        let sender = sender.clone();
                        self.readers.push(std::thread::spawn(move || {
                            let mut bytes = [0; 8192];
                            while let Ok(count) = stream.read(&mut bytes) {
                                if count == 0 {
                                    break;
                                }
                                if sender
                                    .send(host::Inbound::ProcessOutput(host::ProcessOutput {
                                        process,
                                        channel,
                                        bytes: bytes[..count].to_vec(),
                                        text: None,
                                    }))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }));
                    }
                    self.incoming = Some(receiver);
                    self.child = Some((process, child));
                }
                host::ProcessControl::Write { process, bytes } => {
                    let (id, child) = self.child.as_mut().unwrap();
                    assert_eq!(*id, process);
                    child.stdin.as_mut().unwrap().write_all(&bytes).unwrap();
                    self.instance.process_writable(process, bytes.len());
                }
                host::ProcessControl::Kill { process } => {
                    if let Some((id, child)) = self.child.as_mut() {
                        if *id == process {
                            let _ = child.kill();
                        }
                    }
                }
                host::ProcessControl::Close { .. } => {
                    self.child.as_mut().unwrap().1.stdin.take();
                }
                _ => panic!("provider unexpectedly asked for a workspace process"),
            }
        }
        for submitted in self.instance.take_workspace_submissions() {
            let result = match submitted.command {
                Command::Register { descriptor } => self
                    .router
                    .register(self.owner.clone(), descriptor)
                    .map(|id| Reply::Environment { id }),
                Command::Mount { environment, root } => {
                    self.router.mount(&self.owner, environment, root).map(|id| {
                        // Explicit execution grant for this disposable test consumer.
                        self.router
                            .grant(
                                self.consumer.clone(),
                                id,
                                [
                                    api::Capability::FilesRead,
                                    api::Capability::FilesWrite,
                                    api::Capability::Watch,
                                    api::Capability::Process,
                                    api::Capability::Terminal,
                                ],
                            )
                            .unwrap();
                        self.scope = Some(id);
                        Reply::Scope { id }
                    })
                }
                Command::Unregister { environment } => {
                    self.scope = None;
                    self.router
                        .unregister(&self.owner, environment)
                        .map(|_| Reply::Done)
                }
                Command::Present { scope } => {
                    assert_eq!(self.scope, Some(scope));
                    Ok(Reply::Done)
                }
                Command::Remember { scope, .. } => {
                    assert_eq!(self.scope, Some(scope));
                    Ok(Reply::Done)
                }
                Command::Forget { .. } => Ok(Reply::Done),
                Command::Complete { request, result } => self
                    .router
                    .complete(&self.owner, request, result)
                    .map(|_| Reply::Done),
                Command::Notify { scope, event } => self
                    .router
                    .notify(&self.owner, scope, event)
                    .map(|_| Reply::Done),
                _ => panic!("unexpected provider command"),
            };
            self.instance
                .deliver_workspace(&Event::Reply {
                    ticket: submitted.ticket,
                    result,
                })
                .unwrap();
        }
        for event in self.router.poll(&self.owner, 32) {
            self.instance
                .deliver_workspace(&Event::Delivery { event })
                .unwrap();
        }
        self.router
            .expire(self.clock.elapsed().as_millis() as u64)
            .unwrap();
    }
    fn until(&mut self, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done(self) {
            assert!(Instant::now() < deadline, "acceptance timed out");
            self.tick(vec![]);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn request(&mut self, operation: api::Operation) -> Result<api::Answer, api::Failure> {
        let request = self.router.submit(
            &self.consumer,
            self.scope.unwrap(),
            operation,
            self.clock.elapsed().as_millis() as u64,
            20_000,
        )?;
        let mut result = None;
        self.until(|h| {
            for event in h.router.poll(&h.consumer, 32) {
                if let api::Delivery::Complete {
                    request: id,
                    result: value,
                } = event
                {
                    assert_eq!(id, request);
                    result = Some(value);
                } else if let api::Delivery::Notification { event, .. } = event {
                    h.notifications.push(event);
                }
            }
            result.is_some()
        });
        result.unwrap()
    }
    fn process_exit(&mut self, process: u64) -> Vec<api::Notification> {
        self.until(|h| {
            for event in h.router.poll(&h.consumer, 32) {
                if let api::Delivery::Notification { event, .. } = event { h.notifications.push(event); }
            }
            h.notifications.iter().any(|event| matches!(event,
                api::Notification::Exited { handle, .. } | api::Notification::TerminalExited { handle, .. } if *handle == process))
        });
        std::mem::take(&mut self.notifications)
    }
}
fn main() {
    let component_path = std::env::var("LOOG_REMOTE_COMPONENT").expect("built component path");
    let profile = std::env::var("LOOG_REMOTE_PROFILE").expect("non-secret target profile");
    let runtime = host::Runtime::new().unwrap();
    let component = runtime
        .compile(&std::fs::read(component_path).unwrap())
        .unwrap();
    let mut instance = runtime
        .instantiate(&component, host::Limits::default())
        .unwrap();
    instance.set_storage(Box::new(Storage(profile.clone())));
    let mut h = Harness {
        instance,
        router: api::Router::default(),
        owner: api::Owner {
            id: "loog.remote.ssh".into(),
            generation: 1,
        },
        consumer: api::Owner {
            id: "acceptance.editor".into(),
            generation: 1,
        },
        scope: None,
        child: None,
        incoming: None,
        readers: Vec::new(),
        clock: Instant::now(),
        notifications: Vec::new(),
    };
    if let Ok(cycles) = std::env::var("LOOG_REMOTE_LIFECYCLES") {
        lifecycle::run(&mut h, cycles.parse().expect("lifecycle count"), || {
            let mut instance = runtime
                .instantiate(&component, host::Limits::default())
                .unwrap();
            instance.set_storage(Box::new(Storage(profile.clone())));
            instance
        });
        return;
    }
    h.tick(vec![host::Inbound::RunCommand(host::CommandContext {
        call: 1,
        command: "loog.remote.ssh.connect".into(),
        snapshot: None,
        selections: vec![],
        selection_precondition: None,
    })]);
    let nodes = h.instance.take_ui_snapshot().unwrap();
    let button = nodes
        .iter()
        .find(|node| node.description.id == "connect")
        .unwrap()
        .node;
    h.tick(vec![host::Inbound::UiEvent(host::UiEvent {
        node: button,
        id: "ext.15:loog.remote.ssh.1.1.connect".into(),
        payload: host::UiEventPayload::Click,
    })]);
    h.until(|h| h.scope.is_some());
    // Let the provider consume its scope receipt before delivering operations.
    h.tick(vec![]);
    let path =
        api::ProviderPath::new(format!("acceptance-{}-中文%#?\\.bin", std::process::id())).unwrap();
    let root = h.router.scope(h.scope.unwrap()).unwrap().1.clone();
    let api::Answer::Path { path: absolute } = h
        .request(api::Operation::ChildPath {
            parent: root.clone(),
            name: path.as_str().into(),
        })
        .unwrap()
    else {
        panic!("child")
    };
    let api::Answer::Uri { uri } = h
        .request(api::Operation::ResourceUri {
            path: absolute.clone(),
        })
        .unwrap()
    else {
        panic!("uri")
    };
    assert!(uri.contains("%25%23%3F%5C"));
    assert!(
        matches!(h.request(api::Operation::ResolveUri { uri }).unwrap(), api::Answer::Path { path } if path == absolute)
    );
    assert!(
        matches!(h.request(api::Operation::ParentPath { path: absolute }).unwrap(), api::Answer::Parent { path: Some(path) } if path == root)
    );
    assert!(matches!(
        h.request(api::Operation::ParentPath { path: root })
            .unwrap(),
        api::Answer::Parent { path: None }
    ));
    let bytes = vec![0xff, 0xfe, b'A', 0, b'\r', 0, b'\n', 0];
    let saved = h
        .request(api::Operation::Write {
            path: path.clone(),
            bytes: bytes.clone(),
            expected_revision: None,
        })
        .unwrap();
    let api::Answer::Written {
        revision: Some(revision),
    } = saved
    else {
        panic!("missing save revision")
    };
    let read = h
        .request(api::Operation::Read {
            path: path.clone(),
            offset: 0,
            limit: 128,
        })
        .unwrap();
    assert!(
        matches!(read, api::Answer::Bytes { bytes: actual, revision: Some(ref token), eof: true } if actual == bytes && token == &revision)
    );
    h.request(api::Operation::Write {
        path: path.clone(),
        bytes: b"external".to_vec(),
        expected_revision: None,
    })
    .unwrap();
    let error = h
        .request(api::Operation::Write {
            path: path.clone(),
            bytes: b"stale".to_vec(),
            expected_revision: Some(revision),
        })
        .unwrap_err();
    assert_eq!(error.kind, api::FailureKind::Conflict);
    let large: Vec<u8> = (0..600_000).map(|index| (index % 256) as u8).collect();
    let api::Answer::Info { file } = h
        .request(api::Operation::Stat { path: path.clone() })
        .unwrap()
    else {
        panic!("stat")
    };
    let api::Answer::Started { handle } = h
        .request(api::Operation::BeginWrite {
            path: path.clone(),
            size: large.len() as u64,
            expected_revision: file.revision,
        })
        .unwrap()
    else {
        panic!("write handle")
    };
    for (index, chunk) in large.chunks(128 * 1024).enumerate() {
        h.request(api::Operation::AppendWrite {
            write: handle,
            offset: (index * 128 * 1024) as u64,
            bytes: chunk.to_vec(),
        })
        .unwrap();
    }
    assert!(
        matches!(h.request(api::Operation::Read { path: path.clone(), offset: 0, limit: 32 }).unwrap(),
        api::Answer::Bytes { bytes, .. } if bytes == b"external")
    );
    let api::Answer::Written { revision } = h
        .request(api::Operation::CommitWrite { write: handle })
        .unwrap()
    else {
        panic!("commit")
    };
    let mut received = Vec::new();
    loop {
        let api::Answer::Bytes {
            bytes,
            revision: observed,
            eof,
        } = h
            .request(api::Operation::Read {
                path: path.clone(),
                offset: received.len() as u64,
                limit: 128 * 1024,
            })
            .unwrap()
        else {
            panic!("read chunk")
        };
        assert_eq!(observed, revision);
        received.extend(bytes);
        if eof {
            break;
        }
    }
    assert_eq!(received, large);
    assert_eq!(
        h.request(api::Operation::AppendWrite {
            write: handle,
            offset: 0,
            bytes: vec![1]
        })
        .unwrap_err()
        .kind,
        api::FailureKind::PermissionDenied
    );
    let list = h
        .request(api::Operation::List {
            path: api::ProviderPath::new(".").unwrap(),
            cursor: None,
            limit: 32,
        })
        .unwrap();
    assert!(
        matches!(list, api::Answer::Directory { entries, .. } if entries.iter().any(|entry| entry.name == path.as_str()))
    );
    let escaped = h
        .request(api::Operation::Read {
            path: api::ProviderPath::new("/etc/passwd").unwrap(),
            offset: 0,
            limit: 16,
        })
        .unwrap_err();
    assert_eq!(escaped.kind, api::FailureKind::PermissionDenied);
    h.request(api::Operation::Remove {
        path,
        recursive: false,
    })
    .unwrap();
    let payload: Vec<u8> = (0..120_000).map(|index| (index % 256) as u8).collect();
    let api::Answer::Started { handle: process } = h.request(api::Operation::Spawn {
        program: "python3".into(),
        args: vec!["-u".into(), "-c".into(),
            "import os,sys; sys.stderr.write(os.environ['LOOG_TEST']); sys.stdout.buffer.write(sys.stdin.buffer.read())".into()],
        cwd: api::ProviderPath::new(".").unwrap(), env: vec![("LOOG_TEST".into(), "中文".into())],
    }).unwrap() else { panic!("process handle") };
    h.request(api::Operation::Input {
        process,
        bytes: payload.clone(),
    })
    .unwrap();
    h.request(api::Operation::CloseInput { process }).unwrap();
    let events = h.process_exit(process);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut credits = 0u64;
    for event in events {
        match event {
            api::Notification::Output {
                handle,
                bytes,
                stderr: error,
            } => {
                assert_eq!(handle, process);
                if error {
                    stderr.extend(bytes);
                } else {
                    stdout.extend(bytes);
                }
            }
            api::Notification::Writable { handle, bytes } => {
                assert_eq!(handle, process);
                credits += u64::from(bytes);
            }
            api::Notification::Exited { handle, code } => {
                assert_eq!(handle, process);
                assert_eq!(code, Some(0));
            }
            _ => panic!("unexpected notification"),
        }
    }
    assert_eq!(stdout, payload);
    assert_eq!(stderr, "中文".as_bytes());
    assert_eq!(credits, 256 * 1024 + payload.len() as u64);
    let api::Answer::Started { handle: process } = h
        .request(api::Operation::Spawn {
            program: "python3".into(),
            args: vec!["-c".into(), "import time; time.sleep(60)".into()],
            cwd: api::ProviderPath::new(".").unwrap(),
            env: Vec::new(),
        })
        .unwrap()
    else {
        panic!("process handle")
    };
    h.request(api::Operation::Kill { process }).unwrap();
    assert!(h.process_exit(process).iter().any(|event| matches!(event,
        api::Notification::Exited { code, .. } if *code != Some(0))));
    let api::Answer::Started { handle: terminal } = h.request(api::Operation::TerminalOpen {
        cwd: api::ProviderPath::new(".").unwrap(), program: Some("python3".into()),
        args: vec!["-u".into(), "-c".into(),
            "import os,sys; print('READY:%dx%d'%os.get_terminal_size(),flush=True); line=sys.stdin.readline(); print('RESULT:'+line.strip(),flush=True); print('SIZE:%dx%d'%os.get_terminal_size(),flush=True)".into()],
        cols: 80, rows: 24,
    }).unwrap() else { panic!("terminal handle") };
    h.until(|h| {
        for event in h.router.poll(&h.consumer, 32) {
            if let api::Delivery::Notification { event, .. } = event {
                h.notifications.push(event);
            }
        }
        let output: Vec<_> = h
            .notifications
            .iter()
            .filter_map(|event| match event {
                api::Notification::TerminalOutput { bytes, .. } => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        output
            .windows(b"READY:80x24".len())
            .any(|part| part == b"READY:80x24")
    });
    h.request(api::Operation::TerminalResize {
        terminal,
        cols: 100,
        rows: 40,
    })
    .unwrap();
    h.request(api::Operation::TerminalInput {
        terminal,
        bytes: "中文\n".as_bytes().to_vec(),
    })
    .unwrap();
    let events = h.process_exit(terminal);
    let mut output = Vec::new();
    for event in events {
        match event {
            api::Notification::TerminalOutput { handle, bytes } => {
                assert_eq!(handle, terminal);
                output.extend(bytes);
            }
            api::Notification::TerminalExited { handle, code } => {
                assert_eq!(handle, terminal);
                assert_eq!(code, Some(0));
            }
            api::Notification::TerminalWritable { handle, .. } => assert_eq!(handle, terminal),
            api::Notification::TerminalDirectory { handle, path } => {
                assert_eq!(handle, terminal);
                if let Some(path) = path {
                    assert_eq!(&path, h.router.scope(h.scope.unwrap()).unwrap().1);
                }
            }
            _ => panic!("unexpected terminal notification"),
        }
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("RESULT:中文") && output.contains("SIZE:100x40"));
    driver::check(&mut h);
    if let Ok(program) = std::env::var("LOOG_REMOTE_LANGUAGE_PROGRAM") {
        language::check(&mut h, program);
    }
    h.tick(vec![host::Inbound::RunCommand(host::CommandContext {
        call: 2,
        command: "loog.remote.ssh.disconnect".into(),
        snapshot: None,
        selections: vec![],
        selection_precondition: None,
    })]);
    h.until(|h| h.scope.is_none());
    println!(
        "PASS: real WASM/SSH handshake, provider URI/parent paths, raw bytes, staged 600KB save, conflict, listing, boundary, binary process streams, plugin language transport 600KB input/EOF, cancellation, PTY Unicode/resize/exit and disconnect"
    );
}

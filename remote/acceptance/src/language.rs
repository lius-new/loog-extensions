//! Real rust-analyzer acceptance for the plugin-private transport and framing.
//! This is a protocol test, not a claim that the native language UI is wired.
use super::*;
use loog_language_workspace::{Frames, Process, ProcessEvent, Processes};
use serde_json::{Value, json};
use std::collections::VecDeque;

struct Client {
    driver: Processes,
    process: Process,
    tickets: u64,
    submissions: Vec<(u64, Command)>,
    outgoing: VecDeque<Vec<u8>>,
    decoder: Frames,
    messages: Vec<Value>,
    errors: Vec<u8>,
    exited: bool,
}
impl Client {
    fn start(h: &Harness, program: String, cwd: api::ProviderPath) -> Self {
        let mut driver = Processes::default();
        let mut submissions = Vec::new();
        let process = driver
            .start(
                h.scope.unwrap(),
                api::Operation::Spawn {
                    program,
                    args: vec![],
                    cwd,
                    env: vec![],
                },
                &mut |command| {
                    submissions.push((1, command));
                    Ok(1)
                },
            )
            .unwrap();
        Self {
            driver,
            process,
            tickets: 1,
            submissions,
            outgoing: VecDeque::new(),
            decoder: Frames::new(32 * 1024 * 1024).unwrap(),
            messages: Vec::new(),
            errors: Vec::new(),
            exited: false,
        }
    }
    fn send(&mut self, value: Value) {
        let text = value.to_string();
        self.outgoing
            .push_back(format!("Content-Length: {}\r\n\r\n{text}", text.len()).into_bytes());
    }
    fn tick(&mut self, h: &mut Harness) {
        let mut submit = |command| {
            self.tickets += 1;
            self.submissions.push((self.tickets, command));
            Ok(self.tickets)
        };
        if let Some(frame) = self.outgoing.front_mut() {
            let count = self.driver.write(self.process, frame, &mut submit).unwrap();
            frame.drain(..count);
            if frame.is_empty() {
                self.outgoing.pop_front();
            }
        }
        let mut events = self.driver.drive(&mut submit);
        for (ticket, command) in std::mem::take(&mut self.submissions) {
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
                _ => panic!("unexpected language transport command"),
            };
            events.extend(
                self.driver
                    .receive(Event::Reply { ticket, result }, &mut |command| {
                        self.tickets += 1;
                        self.submissions.push((self.tickets, command));
                        Ok(self.tickets)
                    }),
            );
        }
        h.tick(vec![]);
        for event in h.router.poll(&h.consumer, 32) {
            events.extend(
                self.driver
                    .receive(Event::Delivery { event }, &mut |command| {
                        self.tickets += 1;
                        self.submissions.push((self.tickets, command));
                        Ok(self.tickets)
                    }),
            );
        }
        for event in events {
            match event {
                ProcessEvent::Output {
                    stderr: false,
                    bytes,
                    ..
                } => {
                    for message in self.decoder.push(&bytes).unwrap() {
                        let value: Value = serde_json::from_str(&message).unwrap();
                        if value.get("method").is_some() && value.get("id").is_some() {
                            let response = match value["method"].as_str().unwrap() {
                                "window/workDoneProgress/create"
                                | "client/registerCapability"
                                | "client/unregisterCapability" => {
                                    json!({"jsonrpc":"2.0","id":value["id"],"result":null})
                                }
                                "workspace/configuration" => {
                                    json!({"jsonrpc":"2.0","id":value["id"],"result":value["params"]["items"].as_array().unwrap().iter().map(|_| json!({})).collect::<Vec<_>>()})
                                }
                                _ => {
                                    json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Unsupported test-client operation"}})
                                }
                            };
                            self.send(response);
                        } else {
                            self.messages.push(value);
                        }
                    }
                }
                ProcessEvent::Output {
                    stderr: true,
                    bytes,
                    ..
                } => {
                    if self.errors.len() < 16 * 1024 {
                        self.errors
                            .extend(bytes.into_iter().take(16 * 1024 - self.errors.len()));
                    }
                }
                ProcessEvent::Exited { code, error, .. } => {
                    assert_eq!(error, None, "{}", String::from_utf8_lossy(&self.errors));
                    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&self.errors));
                    self.exited = true;
                }
                ProcessEvent::Writable { .. } => {}
            }
        }
        assert!(self.messages.len() <= 512, "language test event budget");
    }
    fn until(&mut self, h: &mut Harness, mut predicate: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(45);
        while !predicate(self) {
            assert!(
                !self.exited && Instant::now() < deadline,
                "language response timed out: {}",
                String::from_utf8_lossy(&self.errors)
            );
            self.tick(h);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn request(&mut self, h: &mut Harness, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        self.until(h, |client| {
            client.messages.iter().any(|message| message["id"] == id)
        });
        self.messages.remove(
            self.messages
                .iter()
                .position(|message| message["id"] == id)
                .unwrap(),
        )
    }
}

pub(super) fn check(h: &mut Harness, program: String) {
    let mounted = h.router.scope(h.scope.unwrap()).unwrap().1.clone();
    let api::Answer::Path { path: directory } = h
        .request(api::Operation::ChildPath {
            parent: mounted,
            name: format!("language-acceptance-{}", std::process::id()),
        })
        .unwrap()
    else {
        panic!("directory")
    };
    h.request(api::Operation::CreateDirectory {
        path: directory.clone(),
    })
    .unwrap();
    let api::Answer::Path { path } = h
        .request(api::Operation::ChildPath {
            parent: directory.clone(),
            name: "中文.rs".into(),
        })
        .unwrap()
    else {
        panic!("path")
    };
    let text = "// 中文😀\nfn value() -> i32 { 7 }\nfn main() { let result = value(); }\n";
    h.request(api::Operation::Write {
        path: path.clone(),
        bytes: text.as_bytes().to_vec(),
        expected_revision: None,
    })
    .unwrap();
    let api::Answer::Uri { uri } = h
        .request(api::Operation::ResourceUri { path: path.clone() })
        .unwrap()
    else {
        panic!("file uri")
    };
    let api::Answer::Uri { uri: root_uri } = h
        .request(api::Operation::ResourceUri {
            path: directory.clone(),
        })
        .unwrap()
    else {
        panic!("root uri")
    };
    let mut client = Client::start(h, program, directory.clone());
    // ProjectJson lets this focused test run without installing a system-wide
    // Rust toolchain. Compiler diagnostics and rustfmt need separate acceptance.
    // https://rust-analyzer.github.io/book/non_cargo_based_projects.html
    let initialized = client.request(h, 1, "initialize", json!({
        "processId":null, "rootUri":root_uri, "workspaceFolders":[{"uri":root_uri,"name":"acceptance"}],
        "capabilities":{"general":{"positionEncodings":["utf-8"]},"textDocument":{"publishDiagnostics":{"versionSupport":true}},"workspace":{"configuration":false}},
        "initializationOptions":{"linkedProjects":[{"crates":[{"root_module":path.as_str(),"edition":"2021","deps":[],"cfg":[],"env":{},"is_workspace_member":true}]}],"checkOnSave":false,"check":{"enable":false},"cargo":{"buildScripts":{"enable":false}},"procMacro":{"enable":false}}
    }));
    assert!(initialized.get("error").is_none(), "{initialized}");
    client.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    client.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"rust","version":1,"text":text}}}));
    let mut hover = Value::Null;
    for id in 10..30 {
        let answer = client.request(
            h,
            id,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":2,"character":26}}),
        );
        if answer["result"].get("contents").is_some() {
            hover = answer["result"].clone();
            break;
        }
        for _ in 0..20 {
            client.tick(h);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        hover.to_string().contains("value"),
        "hover: {hover}, stderr: {}",
        String::from_utf8_lossy(&client.errors)
    );
    let definition = client.request(
        h,
        40,
        "textDocument/definition",
        json!({"textDocument":{"uri":uri},"position":{"line":2,"character":26}}),
    );
    assert!(
        definition["result"].to_string().contains(&uri),
        "{definition}"
    );
    let incomplete = text.replace("value();", "val;");
    client.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":incomplete}]}}));
    let completion = client.request(
        h,
        41,
        "textDocument/completion",
        json!({"textDocument":{"uri":uri},"position":{"line":2,"character":27}}),
    );
    assert!(
        completion["result"].to_string().contains("value"),
        "{completion}"
    );
    client.messages.clear();
    client.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":"fn broken( {"}]}}));
    client.until(h, |client| {
        client.messages.iter().any(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
                && message["params"]["version"] == 3
                && message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
        })
    });
    assert!(
        client
            .request(h, 50, "shutdown", Value::Null)
            .get("error")
            .is_none()
    );
    client.send(json!({"jsonrpc":"2.0","method":"exit"}));
    client.until(h, |client| client.exited);
    h.request(api::Operation::Remove {
        path,
        recursive: false,
    })
    .unwrap();
    h.request(api::Operation::Remove {
        path: directory,
        recursive: false,
    })
    .unwrap();
    println!(
        "PASS: real remote rust-analyzer initialize, Unicode resource URI, hover, definition, completion, versioned syntax diagnostics and shutdown (ProjectJson; no Rust toolchain)"
    );
}

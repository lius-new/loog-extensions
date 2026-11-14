//! Drives the real binary over its pipes, as the client does.
#![allow(dead_code)]
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

pub struct Session {
    pub child: Child,
    pub input: Option<ChildStdin>,
    lines: Receiver<Value>,
    /// Events that arrived while a reply was awaited.
    pub events: Vec<Value>,
    next: u64,
}

impl Session {
    pub fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loog-remote-agent"))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in output.lines() {
                let Ok(line) = line else { break };
                if sender.send(serde_json::from_str(&line).unwrap()).is_err() { break; }
            }
        });
        Session { child, input: Some(input), lines, events: Vec::new(), next: 0 }
    }

    pub fn raw(&mut self, id: u64, method: &str, params: Value) -> Value {
        let line = json!({"id": id, "method": method, "params": params});
        writeln!(self.input.as_mut().unwrap(), "{line}").unwrap();
        loop {
            let value = self.lines.recv_timeout(Duration::from_secs(10)).expect("no reply from the agent");
            if value.get("event").is_some() { self.events.push(value["event"].clone()); continue; }
            return value;
        }
    }

    pub fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.raw(id, method, params)
    }

    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    pub fn kind(&mut self, method: &str, params: Value) -> String {
        self.call(method, params)["error"]["kind"].as_str().unwrap_or("none").to_owned()
    }

    /// The next event satisfying `wanted`, from those already seen or those to come.
    pub fn event(&mut self, wanted: impl Fn(&Value) -> bool) -> Value {
        match self.try_event(wanted, Duration::from_secs(10)) {
            Some(event) => event,
            None => panic!("no matching event; saw {:?}", self.events),
        }
    }

    pub fn try_event(&mut self, wanted: impl Fn(&Value) -> bool, wait: Duration) -> Option<Value> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(at) = self.events.iter().position(&wanted) { return Some(self.events.remove(at)); }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(value) if value.get("event").is_some() => self.events.push(value["event"].clone()),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    /// Output of a process or terminal until it exits: (stdout, stderr, exit code).
    pub fn collect(&mut self, handle: &Value) -> (Vec<u8>, Vec<u8>, i64) {
        use base64::Engine;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        loop {
            let event = self.event(|event| event["handle"] == *handle);
            match event["kind"].as_str().unwrap() {
                "output" | "terminal_output" => {
                    let data = base64::engine::general_purpose::STANDARD.decode(event["data"].as_str().unwrap()).unwrap();
                    if event["stderr"] == true { err.extend(data) } else { out.extend(data) }
                }
                "exited" | "terminal_exited" => return (out, err, event["code"].as_i64().unwrap()),
                _ => {}
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

pub fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loog-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

pub fn mount(session: &mut Session, dir: &Path) -> u64 {
    session.ok("mount", json!({"path": dir}))["root"].as_u64().unwrap()
}

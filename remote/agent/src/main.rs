//! Plugin-owned, bounded filesystem agent for Linux targets. No application
//! imports and no UI policy; the client speaks one JSON object per line on
//! stdin/stdout. Statically linked, so the target needs no interpreter.
#[cfg(not(target_os = "linux"))]
compile_error!("The agent targets Linux");

mod agent;
mod error;
mod uri;
mod process;
mod scan;
mod search;
mod vfs;
mod watch;
mod writes;

use agent::{Agent, LEASE_SECONDS, MAX_FRAME};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static STOPPED: AtomicBool = AtomicBool::new(false);
extern "C" fn stopped(_: libc::c_int) { STOPPED.store(true, Ordering::SeqCst); }

/// Started by the bootstrap from a private directory: remove the binary and the
/// directory at once. The running image stays valid until the process exits.
fn remove_bootstrap(directory: &str) {
    let Ok(exe) = std::fs::read_link("/proc/self/exe") else { return };
    let dir = std::path::Path::new(directory);
    let name = dir.file_name().and_then(|name| name.to_str()).unwrap_or("");
    if name.starts_with("loog-agent-") && exe.parent() == Some(dir) {
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_dir(dir);
    }
}

fn set_nonblocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

struct Output { queue: VecDeque<Vec<u8>>, bytes: usize }

impl Output {
    fn emit(&mut self, mut response: Value) -> Result<(), String> {
        let mut encoded = serde_json::to_vec(&response).unwrap();
        if encoded.len() + 1 > MAX_FRAME {
            response = json!({"id": response.get("id"), "error": {"kind": "limit_exceeded", "message": "response frame budget exceeded"}});
            encoded = serde_json::to_vec(&response).unwrap();
        }
        encoded.push(b'\n');
        if self.bytes + encoded.len() > 4 * MAX_FRAME { return Err("agent output budget exceeded".into()); }
        self.bytes += encoded.len();
        self.queue.push_back(encoded);
        Ok(())
    }
    /// Write what the pipe takes. False once the client has gone.
    fn flush(&mut self) -> bool {
        for _ in 0..32 {
            let Some(front) = self.queue.front_mut() else { break };
            let count = unsafe { libc::write(1, front.as_ptr().cast(), front.len()) };
            if count < 0 {
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EAGAIN) | Some(libc::EINTR) => break,
                    _ => return false,
                }
            }
            if count == 0 { break; }
            let count = count as usize;
            self.bytes -= count;
            if count < front.len() { front.drain(..count); } else { self.queue.pop_front(); }
        }
        true
    }
}

fn run(lease: Duration) -> Result<(), String> {
    let mut agent = Agent::new();
    let mut output = Output { queue: VecDeque::new(), bytes: 0 };
    let mut pending: Vec<u8> = Vec::new();
    let mut last_input = Instant::now();
    set_nonblocking(0);
    set_nonblocking(1);
    while !STOPPED.load(Ordering::SeqCst) {
        if last_input.elapsed() > lease { return Ok(()); }
        let mut chunk = [0u8; 65536];
        let count = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), chunk.len()) };
        let fresh = if count > 0 { count as usize } else if count == 0 { return Ok(()) } else {
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => 0,
                _ => return Ok(()),
            }
        };
        pending.extend_from_slice(&chunk[..fresh]);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            if end + 1 > MAX_FRAME { return Err("agent frame budget exceeded".into()); }
            let line: Vec<u8> = pending.drain(..=end).collect();
            let response = match serde_json::from_slice::<Value>(&line) {
                Ok(value) if value.is_object() => agent.handle(&value),
                Ok(_) => json!({"id": null, "error": {"kind": "invalid_request", "message": "request must be an object"}}),
                Err(error) => json!({"id": null, "error": {"kind": "invalid_request", "message": error.to_string()}}),
            };
            if response.get("result").is_some() || !response["id"].is_null() { last_input = Instant::now(); }
            output.emit(response)?;
        }
        if pending.len() >= MAX_FRAME { return Err("agent frame budget exceeded".into()); }
        for event in agent.watch_events() { output.emit(event)?; }
        // Stop reading target output while the client is behind, so a fast
        // process cannot grow the queue without bound.
        let read_output = output.bytes < 2 * MAX_FRAME;
        for event in agent.process_events(read_output) { output.emit(event)?; }
        if !output.flush() { return Ok(()); }
        let mut descriptors = vec![
            libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: 1, events: if output.queue.is_empty() { 0 } else { libc::POLLOUT }, revents: 0 },
        ];
        descriptors.extend(agent.descriptors(read_output));
        let wait = if fresh > 0 { 0 } else { 100 };
        unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as libc::nfds_t, wait) };
    }
    Ok(())
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        // The bootstrap runs this first, to learn whether the directory allows execution.
        Some("--probe") => return,
        // A search is a disposable child of this same binary: see `search`.
        Some("--search") => {
            if let Err(message) = scan::run() {
                eprintln!("{message}");
                std::process::exit(1);
            }
            return;
        }
        Some("--ephemeral") if arguments.len() == 2 => remove_bootstrap(&arguments[1]),
        Some(_) => { eprintln!("unknown agent argument"); std::process::exit(2); }
        None => {}
    }
    unsafe {
        for signal in [libc::SIGHUP, libc::SIGTERM, libc::SIGINT] {
            libc::signal(signal, stopped as extern "C" fn(libc::c_int) as usize);
        }
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    if let Err(message) = run(Duration::from_secs(LEASE_SECONDS)) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

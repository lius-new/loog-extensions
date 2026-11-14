//! Searches run in a disposable child of this same binary (`--search`): its own
//! session, a 256 MiB address-space cap, and only the root directory descriptor.
//! The agent bounds what the child may publish and reaps it however it ends.
use crate::error::{Result, error};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const REQUEST_LIMIT: usize = 256 * 1024;
const PAGE_LIMIT: usize = 768 * 1024;
const TIME_LIMIT: Duration = Duration::from_secs(130);
pub const IDLE_LIMIT: Duration = Duration::from_secs(30);

pub struct Search {
    pub root: u64,
    pub touched: Instant,
    child: Child,
    stdin: Option<OwnedFd>,
    stdout: OwnedFd,
    stderr: OwnedFd,
    input: Vec<u8>,
    pending: Vec<u8>,
    diagnostics: Vec<u8>,
    pages: VecDeque<(Value, usize)>,
    queued: usize,
    progress: Value,
    complete: bool,
    eof: bool,
    started: Instant,
}

fn nonblocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

impl Search {
    pub fn start(root: u64, directory: &OwnedFd, path: &str, options: &Value) -> Result<Search> {
        let input = serde_json::to_vec(options).unwrap_or_default();
        if input.len() > REQUEST_LIMIT { return Err(error("failed", "Search request exceeds budget")); }
        let inherited = directory.as_raw_fd();
        let mut command = Command::new("/proc/self/exe");
        command.args(["--search", &inherited.to_string(), path]).env_clear()
            .env("PATH", "/usr/bin:/bin").env("LANG", "C.UTF-8")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        unsafe {
            command.pre_exec(move || {
                libc::setsid();
                // The descriptor is close-on-exec in the agent; only this child keeps it.
                libc::fcntl(inherited, libc::F_SETFD, 0);
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let stdin: OwnedFd = child.stdin.take().unwrap().into();
        let stdout: OwnedFd = child.stdout.take().unwrap().into();
        let stderr: OwnedFd = child.stderr.take().unwrap().into();
        for fd in [&stdin, &stdout, &stderr] { nonblocking(fd.as_raw_fd()); }
        Ok(Search {
            root, touched: Instant::now(), child, stdin: Some(stdin), stdout, stderr, input,
            pending: Vec::new(), diagnostics: Vec::new(), pages: VecDeque::new(), queued: 0,
            progress: json!({"searched": 0, "skipped_binary": 0, "skipped_encoding": 0, "skipped_large": 0,
                "errors": 0, "first_error": null, "limit_hit": false}),
            complete: false, eof: false, started: Instant::now(),
        })
    }

    /// Stop the child. It is handed back while it has not been collected yet, so
    /// the agent reaps it later without waiting here.
    pub fn close(mut self) -> Option<Child> {
        if matches!(self.child.try_wait(), Ok(None)) {
            unsafe { libc::killpg(self.child.id() as i32, libc::SIGKILL) };
        }
        match self.child.try_wait() { Ok(None) => Some(self.child), _ => None }
    }

    fn pump(&mut self) -> Result<()> {
        if let Some(stdin) = &self.stdin {
            let count = unsafe { libc::write(stdin.as_raw_fd(), self.input.as_ptr().cast(), self.input.len()) };
            if count >= 0 {
                self.input.drain(..count as usize);
                if self.input.is_empty() { self.stdin = None; }
            } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::EAGAIN) {
                self.stdin = None;
            }
        }
        if self.queued < 256 * 1024 && !self.eof {
            let mut buffer = [0u8; 65536];
            let count = unsafe { libc::read(self.stdout.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
            if count == 0 { self.eof = true; }
            if count > 0 { self.pending.extend_from_slice(&buffer[..count as usize]); }
            while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                if line.len() - 1 > 256 * 1024 { return Err(error("failed", "Search worker publication exceeds budget")); }
                let page: Value = serde_json::from_slice(&line).map_err(|_| error("failed", "Invalid search worker output"))?;
                let size = serde_json::to_vec(&page).map(|bytes| bytes.len()).unwrap_or(usize::MAX);
                if size > PAGE_LIMIT { return Err(error("failed", "Search publication exceeds transport budget")); }
                self.pages.push_back((page, size));
                self.queued += size;
            }
            if self.pending.len() > 256 * 1024 { return Err(error("failed", "Search worker frame exceeds budget")); }
        }
        let mut buffer = [0u8; 4096];
        let count = unsafe { libc::read(self.stderr.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if count > 0 {
            let room = 4096 - self.diagnostics.len();
            self.diagnostics.extend_from_slice(&buffer[..(count as usize).min(room)]);
        }
        if self.started.elapsed() > TIME_LIMIT { return Err(error("failed", "Search exceeded its time budget")); }
        if self.eof && self.pages.is_empty() && !self.complete && !matches!(self.child.try_wait(), Ok(None)) {
            let text = String::from_utf8_lossy(&self.diagnostics).trim().to_owned();
            return Err(error("failed", if text.is_empty() { "Search worker stopped before completion".into() } else { text }));
        }
        Ok(())
    }

    pub fn next(&mut self, limit: &Value) -> Result<Value> {
        let limit = limit.as_u64().filter(|limit| (1..=32).contains(limit)).ok_or_else(|| error("failed", "Invalid search page size"))? as usize;
        self.touched = Instant::now();
        self.pump()?;
        let (mut files, mut used) = (Vec::new(), 0usize);
        while files.len() < limit {
            let Some((_, size)) = self.pages.front() else { break };
            if used > 0 && used + size > PAGE_LIMIT { break; }
            let (mut page, size) = self.pages.pop_front().unwrap();
            self.queued -= size;
            used += size;
            if let Some(list) = page["files"].as_array_mut() { files.append(list); }
            self.progress = page["progress"].take();
            self.complete = page["complete"].as_bool().unwrap_or(false);
        }
        Ok(json!({"files": files, "progress": self.progress, "complete": self.complete}))
    }
}

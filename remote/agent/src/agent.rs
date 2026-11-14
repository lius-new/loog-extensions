//! Request handling: one `Agent` per connection, one JSON object per line.
use crate::error::{AgentError, Result, error};
use crate::process::Processes;
use crate::search::{self, Search};
use crate::watch::Watch;
use crate::writes::{self, Writes};
use crate::{uri, vfs};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::os::fd::AsRawFd;
use std::path::Path;

pub const PROTOCOL: u64 = 2;
pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_RECEIPTS: usize = 256;
pub const LEASE_SECONDS: u64 = 45;

/// Methods whose repetition would repeat a side effect. Their replies are
/// retained so a duplicate id is answered, never executed twice.
const MUTATING: &[&str] = &[
    "write", "mkdir", "rename", "relocate", "create_file", "remove",
    "write_begin", "write_append", "write_commit", "write_abort",
    "watch", "unwatch", "watch_ready",
    "spawn", "process_ready", "process_input", "process_close_input", "process_kill",
    "terminal_open", "terminal_ready", "terminal_input", "terminal_resize", "terminal_close",
    "search", "search_next", "search_close",
];

#[derive(Default)]
pub struct Agent {
    roots: HashMap<u64, vfs::Root>,
    next_root: u64,
    receipts: HashMap<u64, ([u8; 32], Value)>,
    receipt_order: VecDeque<u64>,
    receipt_bytes: usize,
    last_mutation: u64,
    writes: Writes,
    watches: std::collections::BTreeMap<u64, Watch>,
    next_watch: u64,
    processes: Processes,
    searches: std::collections::BTreeMap<u64, Search>,
    /// Closed searches whose child has not been collected yet.
    retired: Vec<std::process::Child>,
    next_search: u64,
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params.get(key).and_then(Value::as_str).ok_or_else(|| error("invalid_request", format!("missing {key}")))
}

impl Agent {
    pub fn new() -> Self { Self::default() }

    fn root_of<'a>(roots: &'a HashMap<u64, vfs::Root>, params: &Value) -> Result<(u64, &'a vfs::Root)> {
        params.get("root").and_then(Value::as_u64).and_then(|id| roots.get(&id).map(|root| (id, root)))
            .ok_or_else(|| error("unavailable", "workspace root is not mounted"))
    }

    fn mount(&mut self, path: &str) -> Result<Value> {
        // A lone "~" stands for the login directory; no other path is expanded.
        let wanted = if path == "~" { home().ok_or_else(|| error("not_found", "workspace directory does not exist"))? } else { path.to_owned() };
        let root = vfs::utf8(&vfs::realpath(&vfs::normalize(Path::new(&wanted))))?;
        if !Path::new(&root).is_dir() { return Err(error("not_found", "workspace directory does not exist")); }
        if let Some((handle, _)) = self.roots.iter().find(|(_, existing)| existing.path == root) {
            return Ok(json!({"root": handle, "path": root}));
        }
        if self.roots.len() >= 128 { return Err(error("limit_exceeded", "too many workspace roots")); }
        let fd = vfs::openat(libc::AT_FDCWD, root.as_ref(), vfs::DIRECTORY_FLAGS, 0)?;
        self.next_root += 1;
        self.roots.insert(self.next_root, vfs::Root { path: root.clone(), fd });
        Ok(json!({"root": self.next_root, "path": root}))
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value> {
        self.writes.expire();
        self.reap_searches();
        let idle: Vec<u64> = self.searches.iter().filter(|(_, search)| search.touched.elapsed() > search::IDLE_LIMIT).map(|(id, _)| *id).collect();
        for id in idle { self.retire(id); }
        match method {
            "hello" => return Ok(json!({"protocol": PROTOCOL, "platform": "linux", "machine": machine(),
                "hostname": hostname(),
                "lease_seconds": LEASE_SECONDS,
                "capabilities": ["files_read", "files_write", "watch", "process", "terminal", "search"],
                "max_chunk": vfs::MAX_CHUNK,
                "write_guarantee": "revision-check-then-atomic-replace"})),
            "ping" => return Ok(json!({})),
            "mount" => return self.mount(text(params, "path")?),
            // Reading a folder to choose it needs no root: it is what a shell would show.
            "browse" => {
                let path = text(params, "path")?;
                let wanted = if path == "~" { home().ok_or_else(|| error("not_found", "no login folder"))? } else { path.to_owned() };
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(vfs::MAX_BROWSE as u64)
                    .clamp(1, vfs::MAX_BROWSE as u64) as usize;
                return vfs::browse(&wanted, limit);
            }
            _ => {}
        }
        let (root_id, root) = Self::root_of(&self.roots, params)?;
        match method {
            "parent_path" => {
                let path = vfs::resolve(root, &params["path"], true)?;
                let parent = if path == root.path { None } else { Path::new(&path).parent().and_then(Path::to_str).map(str::to_owned) };
                Ok(json!({"path": parent}))
            }
            "resolve_path" | "relative_path" => {
                let base = vfs::resolve(root, &params["base"], true)?;
                let leaf = vfs::open_parent(root, base.clone())?;
                vfs::openat(leaf.fd.as_raw_fd(), &leaf.name, vfs::DIRECTORY_FLAGS, 0)?;
                if method == "resolve_path" {
                    let relative = params["relative"].as_str()
                        .filter(|text| !text.is_empty() && !text.contains('\0') && text.len() <= 32 * 1024)
                        .ok_or_else(|| error("invalid_request", "invalid relative resource path"))?;
                    let joined = Path::new(&base).join(relative);
                    let value = Value::String(vfs::utf8(&joined)?);
                    return Ok(json!({"path": vfs::resolve(root, &value, true)?}));
                }
                let path = vfs::resolve(root, &params["path"], true)?;
                let Ok(relative) = Path::new(&path).strip_prefix(&base) else { return Ok(json!({"components": null})) };
                let parts: Vec<String> = relative.components().map(|part| vfs::utf8(part.as_os_str().as_ref())).collect::<Result<_>>()?;
                if parts.len() > 128 || parts.iter().map(String::len).sum::<usize>() > 32 * 1024 {
                    return Err(error("limit_exceeded", "relative resource path exceeds its budget"));
                }
                Ok(json!({"components": parts}))
            }
            "resource_uri" => Ok(json!({"uri": uri::encode(&vfs::resolve(root, &params["path"], true)?)})),
            "resolve_uri" => {
                let path = uri::decode(text(params, "uri")?)?;
                Ok(json!({"path": vfs::resolve(root, &Value::String(path), true)?}))
            }
            "child_path" => {
                let name = text(params, "name")?;
                if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
                    return Err(error("invalid_request", "enter a single file or folder name"));
                }
                if name.len() > 255 { return Err(error("invalid_request", "name is too long")); }
                let leaf = vfs::parent(root, &params["parent"], true)?;
                vfs::openat(leaf.fd.as_raw_fd(), &leaf.name, vfs::DIRECTORY_FLAGS, 0)?;
                Ok(json!({"path": vfs::utf8(&Path::new(&leaf.path).join(name))?}))
            }
            "stat" => {
                let leaf = vfs::parent(root, &params["path"], false)?;
                vfs::info(leaf.fd.as_raw_fd(), &leaf.name, &leaf.path, true)
            }
            "list" => {
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(512).clamp(1, 512) as usize;
                let leaf = vfs::parent(root, &params["path"], true)?;
                vfs::list(&leaf, limit, params.get("cursor").and_then(Value::as_str))
            }
            "read" => {
                let offset = params.get("offset").and_then(Value::as_i64).unwrap_or(0);
                if offset < 0 { return Err(error("invalid_request", "negative read offset")); }
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(vfs::MAX_CHUNK as u64).clamp(1, vfs::MAX_CHUNK as u64) as usize;
                let leaf = vfs::parent(root, &params["path"], true)?;
                vfs::read(&leaf, offset as u64, limit)
            }
            "write" => writes::write(&vfs::parent(root, &params["path"], true)?, params),
            "create_file" => writes::create_file(&vfs::parent(root, &params["path"], false)?),
            "mkdir" => writes::mkdir(&vfs::parent(root, &params["path"], false)?),
            "remove" => writes::remove(root, &vfs::parent(root, &params["path"], false)?,
                params.get("recursive").and_then(Value::as_bool).unwrap_or(false)),
            "rename" | "relocate" => writes::rename(root, params, method == "relocate"),
            "write_begin" => self.writes.begin(root_id, &vfs::parent(root, &params["path"], true)?, params),
            "write_append" | "write_commit" | "write_abort" => self.writes.step(method, root_id, params),
            "search" => {
                if self.searches.len() + self.retired.len() >= 4 {
                    return Err(error("limit_exceeded", "too many concurrent searches"));
                }
                let leaf = vfs::parent(root, &params["path"], true)?;
                let directory = vfs::openat(leaf.fd.as_raw_fd(), &leaf.name, vfs::DIRECTORY_FLAGS, 0)?;
                let search = Search::start(root_id, &directory, &leaf.path, &params["options"])?;
                self.next_search += 1;
                self.searches.insert(self.next_search, search);
                Ok(json!({"search": self.next_search}))
            }
            "search_next" | "search_close" => {
                let id = params["search"].as_u64().unwrap_or(0);
                let Some(search) = self.searches.get_mut(&id) else {
                    if method == "search_close" { return Ok(json!({})); }
                    return Err(error("unavailable", "search is no longer available"));
                };
                if search.root != root_id { return Err(error("permission_denied", "search belongs to another root")); }
                if method == "search_close" { self.retire(id); return Ok(json!({})); }
                search.next(&params["limit"])
            }
            "spawn" => self.processes.spawn(root_id, root, params, false),
            "terminal_open" => self.processes.spawn(root_id, root, params, true),
            "terminal_ready" | "terminal_input" | "terminal_resize" | "terminal_close"
            | "process_ready" | "process_input" | "process_close_input" | "process_kill" =>
                self.processes.control(method, root_id, params),
            "watch" => {
                if self.watches.len() >= 16 { return Err(error("limit_exceeded", "too many filesystem subscriptions")); }
                let watch = Watch::new(root_id, root, &params["path"])?;
                self.next_watch += 1;
                self.watches.insert(self.next_watch, watch);
                Ok(json!({"watch": self.next_watch}))
            }
            "unwatch" | "watch_ready" => {
                let id = params["watch"].as_u64().unwrap_or(0);
                if let Some(watch) = self.watches.get_mut(&id) {
                    if watch.root != root_id { return Err(error("permission_denied", "watch belongs to another root")); }
                    if method == "unwatch" { self.watches.remove(&id); } else { watch.ready = true; }
                }
                Ok(json!({}))
            }
            _ => Err(error("unsupported", "unknown agent operation")),
        }
    }

    pub fn handle(&mut self, request: &Value) -> Value {
        let id = request.get("id").and_then(Value::as_u64).filter(|id| *id >= 1);
        let Some(id) = id else {
            return json!({"id": request.get("id"), "error": {"kind": "invalid_request", "message": "invalid request id"}});
        };
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let mutating = MUTATING.contains(&method);
        let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(request).unwrap_or_default()).into();
        if mutating {
            if let Some((previous, response)) = self.receipts.get(&id) {
                return if *previous == digest { response.clone() } else {
                    json!({"id": id, "error": {"kind": "conflict", "message": "request id reused with different content"}})
                };
            }
            // Mutations arrive in increasing order; an evicted receipt is unknown,
            // never permission to replay its operation.
            if id <= self.last_mutation {
                return json!({"id": id, "error": {"kind": "unknown_outcome", "message": "mutation receipt is no longer retained"}});
            }
        }
        let empty = json!({});
        let response = match self.dispatch(method, request.get("params").unwrap_or(&empty)) {
            Ok(result) => json!({"id": id, "result": result}),
            Err(AgentError { kind, message }) => json!({"id": id, "error": {"kind": kind, "message": message}}),
        };
        if mutating {
            self.last_mutation = id;
            self.receipt_bytes += response.to_string().len();
            self.receipts.insert(id, (digest, response.clone()));
            self.receipt_order.push_back(id);
            while self.receipt_order.len() > MAX_RECEIPTS || self.receipt_bytes > 4 * MAX_FRAME {
                let Some(old) = self.receipt_order.pop_front() else { break };
                if let Some((_, retired)) = self.receipts.remove(&old) { self.receipt_bytes -= retired.to_string().len(); }
            }
        }
        response
    }
}

fn home() -> Option<String> {
    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) { return home.into_string().ok(); }
    let mut buffer = vec![0u8; 4096];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let code = unsafe { libc::getpwuid_r(libc::getuid(), &mut entry, buffer.as_mut_ptr().cast(), buffer.len(), &mut found) };
    if code != 0 || found.is_null() { return None; }
    unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) }.to_str().ok().map(str::to_owned)
}

fn machine() -> String {
    let mut data: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut data) };
    unsafe { std::ffi::CStr::from_ptr(data.machine.as_ptr()) }.to_string_lossy().into_owned()
}

/// The machine's own name, for showing a connection that was made by address.
fn hostname() -> String {
    let mut data: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut data) };
    unsafe { std::ffi::CStr::from_ptr(data.nodename.as_ptr()) }.to_string_lossy().into_owned()
}

impl Agent {
    /// Filesystem changes since the last round. A subscription that cannot go on
    /// reports that it stopped, once, and is removed.
    pub fn watch_events(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        let ids: Vec<u64> = self.watches.keys().copied().collect();
        for id in ids {
            let Some(watch) = self.watches.get_mut(&id) else { continue };
            let Some(root) = self.roots.get(&watch.root) else { continue };
            match watch.poll(root) {
                Ok(Some(mut event)) => { event["watch"] = id.into(); out.push(json!({"event": event})); }
                Ok(None) => {}
                Err(failure) => {
                    let kind = failure.kind;
                    self.watches.remove(&id);
                    out.push(json!({"event": {"kind": "watch_stopped", "watch": id, "error": {
                        "kind": kind, "message": "Filesystem monitoring stopped; refresh to retry"}}}));
                }
            }
        }
        out
    }

    fn retire(&mut self, id: u64) {
        if let Some(child) = self.searches.remove(&id).and_then(Search::close) { self.retired.push(child); }
    }

    /// Collect searches that ended; the agent itself reaps every child it started.
    pub fn reap_searches(&mut self) {
        self.retired.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
    }

    pub fn process_events(&mut self, read_output: bool) -> Vec<Value> {
        self.reap_searches();
        self.processes.events(&self.roots, read_output)
    }

    /// Descriptors the loop should wait on, and for which of them it wants to write.
    pub fn descriptors(&self, read_output: bool) -> Vec<libc::pollfd> {
        let mut out: Vec<libc::pollfd> = self.watches.values().filter(|watch| watch.ready)
            .map(|watch| libc::pollfd { fd: watch.raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
        for process in self.processes.map.values().filter(|process| process.ready) {
            if read_output {
                out.extend(process.output_fds().into_iter().map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }));
            }
            if process.wants_write() {
                out.push(libc::pollfd { fd: process.stdin_fd().unwrap(), events: libc::POLLOUT, revents: 0 });
            }
        }
        out
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let ids: Vec<u64> = self.searches.keys().copied().collect();
        for id in ids { self.retire(id); }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !self.retired.is_empty() && std::time::Instant::now() < deadline {
            self.reap_searches();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.processes.close_all();
        self.watches.clear();
        self.writes.abort_all();
    }
}

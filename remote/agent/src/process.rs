//! Target processes and terminals. Argument arrays, separate binary pipes and a
//! new session; no implicit shell and no local platform or path substitutions.
use crate::error::{Result, error};
use crate::vfs::MAX_CHUNK;
use base64::Engine;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::{CString, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const INPUT_LIMIT: usize = 256 * 1024;
pub const OUTPUT_CHUNK: usize = 16 * 1024;

pub struct Session {
    pub root: u64,
    pub ready: bool,
    pub terminal: bool,
    pub closing_input: bool,
    pub directory_check: Instant,
    pub directory_token: Option<Option<String>>,
    pid: i32,
    returncode: Option<i32>,
    initial_credit: bool,
    input: VecDeque<Vec<u8>>,
    input_bytes: usize,
    stop_deadline: Option<Instant>,
    exit_deadline: Option<Instant>,
    stdin: Option<i32>,
    outputs: Vec<(i32, bool)>,
    streams: HashMap<i32, OwnedFd>,
}

pub fn terminal_size(cols: &Value, rows: &Value) -> Result<libc::winsize> {
    let valid = |value: &Value| value.as_u64().filter(|size| (1..=1000).contains(size));
    match (valid(cols), valid(rows)) {
        (Some(cols), Some(rows)) => Ok(libc::winsize { ws_row: rows as u16, ws_col: cols as u16, ws_xpixel: 0, ws_ypixel: 0 }),
        _ => Err(error("invalid_request", "invalid terminal dimensions")),
    }
}

fn nonblocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

fn into_fd(stream: impl Into<OwnedFd>) -> OwnedFd { stream.into() }

impl Session {
    fn adopt(root: u64, terminal: bool, pid: i32, stdin: Option<OwnedFd>, outputs: Vec<(OwnedFd, bool)>) -> Session {
        let mut session = Session {
            root, ready: false, terminal, closing_input: false,
            directory_check: Instant::now(), directory_token: None, pid, returncode: None,
            initial_credit: true, input: VecDeque::new(), input_bytes: 0, stop_deadline: None,
            exit_deadline: None, stdin: None, outputs: Vec::new(), streams: HashMap::new(),
        };
        if let Some(fd) = stdin { session.stdin = Some(fd.as_raw_fd()); nonblocking(fd.as_raw_fd()); session.streams.insert(fd.as_raw_fd(), fd); }
        for (fd, stderr) in outputs {
            nonblocking(fd.as_raw_fd());
            session.outputs.push((fd.as_raw_fd(), stderr));
            session.streams.insert(fd.as_raw_fd(), fd);
        }
        session
    }

    pub fn start(root: u64, program: &str, args: &[String], directory: i32,
                 environment: &BTreeMap<OsString, OsString>, size: Option<libc::winsize>) -> Result<Session> {
        match size {
            Some(size) => Self::pty(root, program, args, directory, environment, size),
            None => {
                let mut command = Command::new(program);
                command.args(args).env_clear().envs(environment)
                    .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
                unsafe {
                    command.pre_exec(move || {
                        libc::setsid();
                        if libc::fchdir(directory) != 0 { return Err(std::io::Error::last_os_error()); }
                        Ok(())
                    });
                }
                let mut child = command.spawn()?;
                let pid = child.id() as i32;
                let stdin = child.stdin.take().map(into_fd);
                let stdout = into_fd(child.stdout.take().unwrap());
                let stderr = into_fd(child.stderr.take().unwrap());
                Ok(Self::adopt(root, false, pid, stdin, vec![(stdout, false), (stderr, true)]))
            }
        }
    }

    fn pty(root: u64, program: &str, args: &[String], directory: i32,
           environment: &BTreeMap<OsString, OsString>, size: libc::winsize) -> Result<Session> {
        let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
        if master < 0 { return Err(std::io::Error::last_os_error().into()); }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let mut name = [0 as libc::c_char; 128];
        unsafe {
            if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0
                || libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let program = CString::new(program).map_err(|_| error("invalid_request", "invalid process arguments"))?;
        let mut argv = vec![program.clone()];
        for arg in args { argv.push(CString::new(arg.as_str()).map_err(|_| error("invalid_request", "invalid process arguments"))?); }
        let mut pointers: Vec<*const libc::c_char> = argv.iter().map(|arg| arg.as_ptr()).collect();
        pointers.push(std::ptr::null());
        let variables: Vec<(CString, CString)> = environment.iter()
            .filter_map(|(key, value)| Some((CString::new(key.as_bytes()).ok()?, CString::new(value.as_bytes()).ok()?))).collect();
        let pid = unsafe { libc::fork() };
        if pid < 0 { return Err(std::io::Error::last_os_error().into()); }
        if pid == 0 {
            unsafe {
                libc::setsid();
                let slave = libc::open(name.as_ptr(), libc::O_RDWR);
                libc::ioctl(slave, libc::TIOCSCTTY as _, 0);
                for target in 0..3 { libc::dup2(slave, target); }
                if slave > 2 { libc::close(slave); }
                libc::ioctl(0, libc::TIOCSWINSZ as _, &size);
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                if slave >= 0 && libc::fchdir(directory) == 0 {
                    libc::clearenv();
                    for (key, value) in &variables { libc::setenv(key.as_ptr(), value.as_ptr(), 1); }
                    libc::execvp(program.as_ptr(), pointers.as_ptr());
                }
                // Never run the copied parent's cleanup in the fork.
                let message = b"Could not start terminal program\r\n";
                libc::write(2, message.as_ptr().cast(), message.len());
                libc::_exit(127);
            }
        }
        let outputs = vec![(master.try_clone()?, false)];
        let mut session = Self::adopt(root, true, pid, None, outputs);
        // One descriptor serves input and output, as on a terminal master.
        session.stdin = Some(master.as_raw_fd());
        session.streams.insert(master.as_raw_fd(), master);
        Ok(session)
    }

    pub fn stdin_fd(&self) -> Option<i32> { self.stdin.filter(|fd| self.streams.contains_key(fd)) }
    pub fn output_fds(&self) -> Vec<i32> { self.outputs.iter().map(|(fd, _)| *fd).collect() }
    pub fn wants_write(&self) -> bool { !self.input.is_empty() && self.stdin_fd().is_some() }
    pub fn pid(&self) -> i32 { self.pid }

    fn close_stream(&mut self, fd: i32) { self.streams.remove(&fd); }

    pub fn reap(&mut self) -> Option<i32> {
        if self.returncode.is_none() {
            let mut status = 0;
            let done = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if done == self.pid {
                self.returncode = Some(if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -libc::WTERMSIG(status) });
            }
        }
        self.returncode
    }

    /// Reclaim the child's process group and, for an interactive shell, the
    /// foreground jobs it started in other groups of the same session. A
    /// process that deliberately creates another session is outside this lease.
    pub fn stop(&mut self, force: bool) {
        self.ready = true;
        let mut groups = vec![self.pid];
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if !name.as_bytes().iter().all(u8::is_ascii_digit) { continue; }
                let Ok(text) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
                let Some((_, rest)) = text.rsplit_once(')') else { continue };
                let fields: Vec<&str> = rest.split_whitespace().collect();
                if fields.get(3).and_then(|session| session.parse::<i32>().ok()) == Some(self.pid) {
                    if let Some(group) = fields.get(2).and_then(|group| group.parse::<i32>().ok()) {
                        if !groups.contains(&group) { groups.push(group); }
                    }
                }
            }
        }
        for group in groups {
            unsafe { libc::killpg(group, if force { libc::SIGKILL } else { libc::SIGTERM }) };
        }
        self.closing_input = true;
        self.input.clear();
        self.input_bytes = 0;
        if !force { self.stop_deadline = Some(Instant::now() + Duration::from_secs(1)); }
    }

    pub fn close(&mut self) {
        self.stop(true);
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.reap().is_none() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(1)); }
        self.streams.clear();
    }

    pub fn write(&mut self, data: Vec<u8>) -> Result<()> {
        if self.closing_input || self.stdin_fd().is_none() || self.reap().is_some() {
            return Err(error("unavailable", "process input is closed"));
        }
        if data.len() > MAX_CHUNK || self.input_bytes + data.len() > INPUT_LIMIT {
            return Err(error("limit_exceeded", "process input window is full"));
        }
        if !data.is_empty() { self.input_bytes += data.len(); self.input.push_back(data); }
        Ok(())
    }

    /// Move bytes both ways. The flag says the process has ended and is gone.
    pub fn poll(&mut self, read_output: bool) -> (Vec<Value>, bool) {
        let mut events = Vec::new();
        if self.stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            self.stop(true);
            self.stop_deadline = None;
        }
        if !self.ready { return (events, false); }
        if self.initial_credit {
            events.push(json!({"kind": "writable", "bytes": INPUT_LIMIT}));
            self.initial_credit = false;
        }
        let mut freed = 0usize;
        for _ in 0..16 {
            let Some(fd) = self.stdin_fd() else { break };
            let Some(front) = self.input.front() else { break };
            let count = unsafe { libc::write(fd, front.as_ptr().cast(), front.len()) };
            if count < 0 {
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EAGAIN) | Some(libc::EINTR) => break,
                    _ => { self.input.clear(); self.input_bytes = 0; self.closing_input = true; break }
                }
            }
            if count == 0 { break; }
            let count = count as usize;
            let chunk = self.input.pop_front().unwrap();
            if count < chunk.len() { self.input.push_front(chunk[count..].to_vec()); }
            freed += count;
            self.input_bytes -= count;
        }
        if freed > 0 { events.push(json!({"kind": "writable", "bytes": freed})); }
        if !self.terminal && self.closing_input && self.input.is_empty() {
            if let Some(fd) = self.stdin_fd() { self.close_stream(fd); }
        }
        if read_output {
            for (fd, stderr) in self.outputs.clone() {
                let mut buffer = [0u8; OUTPUT_CHUNK];
                let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
                let data: &[u8] = if count > 0 { &buffer[..count as usize] } else if count == 0 { &[] } else {
                    match std::io::Error::last_os_error().raw_os_error() {
                        Some(libc::EAGAIN) | Some(libc::EINTR) => continue,
                        // A terminal master reports EIO once the child side is gone.
                        _ => &[],
                    }
                };
                if data.is_empty() {
                    self.outputs.retain(|(other, _)| *other != fd);
                    self.close_stream(fd);
                } else {
                    events.push(json!({"kind": "output", "stderr": stderr,
                        "data": base64::engine::general_purpose::STANDARD.encode(data)}));
                }
            }
        }
        if let Some(code) = self.reap() {
            if !read_output && self.exit_deadline.is_some() { self.exit_deadline = Some(Instant::now() + Duration::from_secs(2)); }
            if self.exit_deadline.is_some() && events.iter().any(|event| event["kind"] == "output") {
                self.exit_deadline = Some(Instant::now() + Duration::from_secs(2));
            }
            if self.exit_deadline.is_none() {
                // A background descendant must not outlive the process lease.
                self.stop(false);
                self.exit_deadline = Some(Instant::now() + Duration::from_secs(2));
            }
            if self.outputs.is_empty() || Instant::now() >= self.exit_deadline.unwrap() {
                self.close();
                events.push(json!({"kind": "exited", "code": code}));
                return (events, true);
            }
        }
        (events, false)
    }
}

#[derive(Default)]
pub struct Processes {
    pub map: BTreeMap<u64, Session>,
    next: u64,
}

fn valid(text: &str) -> bool { !text.contains('\0') && text.len() <= 8192 }

impl Processes {
    pub fn spawn(&mut self, root_id: u64, root: &crate::vfs::Root, params: &Value, terminal: bool) -> Result<Value> {
        use crate::vfs;
        if self.map.len() >= 16 { return Err(error("limit_exceeded", "too many running processes")); }
        let bad_arguments = || error("invalid_request", "invalid process arguments");
        let program = match params.get("program") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return Err(bad_arguments()),
        };
        let size = if terminal { Some(terminal_size(&params["cols"], &params["rows"])?) } else { None };
        let program = match program {
            None if terminal => std::env::var("SHELL").ok().filter(|shell| !shell.is_empty()).unwrap_or_else(|| "/bin/sh".into()),
            Some(program) => program,
            None => return Err(bad_arguments()),
        };
        let empty = Vec::new();
        let args = match params.get("args") { None => &empty, Some(Value::Array(list)) => list, Some(_) => return Err(bad_arguments()) };
        let args: Vec<String> = args.iter().map(|arg| arg.as_str().filter(|text| valid(text)).map(str::to_owned)).collect::<Option<_>>()
            .filter(|list: &Vec<String>| list.len() <= 128).ok_or_else(bad_arguments)?;
        if program.is_empty() || !valid(&program) { return Err(bad_arguments()); }
        let bad_environment = || error("invalid_request", "invalid process environment");
        let pairs = match params.get("env") { None => &empty, Some(Value::Array(list)) => list, Some(_) => return Err(bad_environment()) };
        if pairs.len() > 128 { return Err(bad_environment()); }
        let mut overrides = Vec::new();
        for pair in pairs {
            let pair = pair.as_array().filter(|pair| pair.len() == 2).ok_or_else(bad_environment)?;
            let (Some(key), Some(value)) = (pair[0].as_str(), pair[1].as_str()) else { return Err(bad_environment()) };
            if !valid(key) || !valid(value) || key.is_empty() || key.contains('=') { return Err(bad_environment()); }
            overrides.push((key.to_owned(), value.to_owned()));
        }
        if serde_json::to_vec(&json!([program, args, pairs])).unwrap_or_default().len() > 128 * 1024 {
            return Err(error("limit_exceeded", "process arguments exceed the budget"));
        }
        let mut environment: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
        for (key, value) in overrides { environment.insert(key.into(), value.into()); }
        if terminal { environment.insert("TERM".into(), "xterm-256color".into()); }
        let parent = vfs::parent(root, &params["cwd"], true)?;
        let excluded = match params.get("excluded_roots") {
            None => &empty,
            Some(Value::Array(list)) if list.len() <= 128 => list,
            Some(_) => return Err(error("invalid_request", "invalid execution boundaries")),
        };
        for value in excluded {
            let text = value.as_str().filter(|text| !text.contains('\0') && text.starts_with('/'))
                .ok_or_else(|| error("invalid_request", "invalid execution boundary"))?;
            let boundary = vfs::realpath(std::path::Path::new(text));
            if boundary != std::path::Path::new(&root.path) && boundary.starts_with(&root.path)
                && std::path::Path::new(&parent.path).starts_with(&boundary) {
                return Err(error("permission_denied", "program directory is in an untrusted nested root"));
            }
        }
        let directory = vfs::openat(parent.fd.as_raw_fd(), &parent.name, vfs::DIRECTORY_FLAGS, 0)?;
        let session = Session::start(root_id, &program, &args, directory.as_raw_fd(), &environment, size)?;
        self.next += 1;
        self.map.insert(self.next, session);
        Ok(json!({if terminal { "terminal" } else { "process" }: self.next}))
    }

    pub fn control(&mut self, method: &str, root_id: u64, params: &Value) -> Result<Value> {
        let terminal = method.starts_with("terminal_");
        let id = params[if terminal { "terminal" } else { "process" }].as_u64().unwrap_or(0);
        let Some(process) = self.map.get_mut(&id) else {
            if matches!(method, "terminal_ready" | "terminal_close" | "process_ready" | "process_kill") { return Ok(json!({})); }
            return Err(error("unavailable", if terminal { "terminal has exited" } else { "process has exited" }));
        };
        if process.root != root_id || process.terminal != terminal {
            return Err(error("permission_denied", if terminal { "terminal belongs to another scope" } else { "process belongs to another root" }));
        }
        let data = |params: &Value| -> Result<Vec<u8>> {
            let text = params["data"].as_str().ok_or_else(|| error("invalid_request", "missing data"))?;
            base64::engine::general_purpose::STANDARD.decode(text).map_err(|_| error("invalid_request", "invalid base64 data"))
        };
        match method {
            "terminal_ready" | "process_ready" => process.ready = true,
            "terminal_input" | "process_input" => process.write(data(params)?)?,
            "process_close_input" => process.closing_input = true,
            "terminal_resize" => {
                let size = terminal_size(&params["cols"], &params["rows"])?;
                if let Some(fd) = process.stdin_fd() { unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &size) }; }
            }
            _ => process.stop(false),
        }
        Ok(json!({}))
    }

    /// Output, credit and exit events, and the terminal's working directory.
    pub fn events(&mut self, roots: &HashMap<u64, crate::vfs::Root>, read_output: bool) -> Vec<Value> {
        let mut out = Vec::new();
        let ids: Vec<u64> = self.map.keys().copied().collect();
        for id in ids {
            let process = self.map.get_mut(&id).unwrap();
            let (events, ended) = process.poll(read_output);
            for mut event in events {
                if process.terminal {
                    let kind = match event["kind"].as_str() {
                        Some("output") => "terminal_output",
                        Some("writable") => "terminal_writable",
                        _ => "terminal_exited",
                    };
                    event["kind"] = kind.into();
                    event.as_object_mut().unwrap().remove("stderr");
                }
                event["handle"] = id.into();
                out.push(json!({"event": event}));
            }
            if process.terminal && process.ready && !ended && Instant::now() >= process.directory_check {
                process.directory_check = Instant::now() + Duration::from_millis(500);
                // Provider-owned Linux lookup; no shell hooks or host path parsing.
                let directory = roots.get(&process.root).and_then(|root| {
                    let link = std::fs::read_link(format!("/proc/{}/cwd", process.pid())).ok()?;
                    let path = crate::vfs::resolve(root, &Value::String(link.to_str()?.to_owned()), true).ok()?;
                    (path.len() <= 32768).then_some(path)
                });
                if process.directory_token.as_ref() != Some(&directory) {
                    process.directory_token = Some(directory.clone());
                    out.push(json!({"event": {"kind": "terminal_directory", "handle": id, "path": directory}}));
                }
            }
            if ended { self.map.remove(&id); }
        }
        out
    }

    pub fn close_all(&mut self) {
        for process in self.map.values_mut() { process.stop(true); }
        for process in self.map.values_mut() { process.close(); }
        self.map.clear();
    }
}

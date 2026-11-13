use ignore::gitignore::{Gitignore, GitignoreBuilder};
use loog_text_core::{
    encoding,
    search::{MAX_MATCHES, Matcher, hash},
};
use loog_workspace_api::{SearchFile, SearchOptions, SearchPage};
use std::{
    collections::HashSet,
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const MAX_FILE: usize = 4 * 1024 * 1024;
const MAX_IGNORE: usize = 256 * 1024;
const MAX_IGNORE_TOTAL: usize = 1024 * 1024;
const MAX_IGNORE_LINES: usize = 8192;
const MAX_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_READ: usize = 1024 * 1024 * 1024;

struct Scanner {
    root: PathBuf,
    options: SearchOptions,
    matcher: Matcher,
    buffers: HashSet<String>,
    page: SearchPage,
    matched: usize,
    files: usize,
    entries: usize,
    read: usize,
    ignore_bytes: usize,
    ignore_lines: usize,
    output: usize,
    started: Instant,
    published: Instant,
    sink: std::io::BufWriter<std::io::Stdout>,
}

pub(super) fn run() -> Result<(), String> {
    // Bound compiled ignore/regex allocations as well as scanned input. The
    // worker is disposable; exceeding this limit is an explicit failed search.
    let memory = libc::rlimit {
        rlim_cur: 256 * 1024 * 1024,
        rlim_max: 256 * 1024 * 1024,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &memory) } != 0 {
        return Err("Could not bound search worker memory".into());
    }
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 2 {
        return Err("Expected inherited root descriptor and display identity".into());
    }
    let fd: i32 = arguments[0]
        .parse()
        .map_err(|_| "Invalid root descriptor")?;
    if fd < 3 {
        return Err("Invalid root descriptor".into());
    }
    // Agent owns the parent descriptor. This process owns its inherited copy.
    let directory = unsafe { File::from_raw_fd(fd) };
    if !directory.metadata().map_err(|e| e.to_string())?.is_dir() {
        return Err("Search root is not a directory".into());
    }
    let root = PathBuf::from(&arguments[1]);
    if !root.is_absolute() {
        return Err("Search root identity is not absolute".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("Search request exceeds budget".into());
    }
    let options: SearchOptions = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if options.buffers.len() > 128 {
        return Err("Too many unsaved search buffers".into());
    }
    let matcher = Matcher::new(&options.spec)?;
    let buffers = options
        .buffers
        .iter()
        .map(|path| path.as_str().to_owned())
        .collect();
    let mut scanner = Scanner {
        root,
        options,
        matcher,
        buffers,
        page: SearchPage::default(),
        matched: 0,
        files: 0,
        entries: 0,
        read: 0,
        ignore_bytes: 0,
        ignore_lines: 0,
        output: 0,
        started: Instant::now(),
        published: Instant::now(),
        sink: std::io::BufWriter::new(std::io::stdout()),
    };
    scanner.walk(directory, PathBuf::new(), &[], 0)?;
    scanner.page.complete = true;
    scanner.publish()?;
    Ok(())
}

fn open(parent: &File, name: &str, directory: bool) -> std::io::Result<File> {
    let name = CString::new(name).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

impl Scanner {
    fn limited(&mut self) -> bool {
        self.page.progress.limit_hit |= self.entries >= MAX_ENTRIES
            || self.read >= MAX_READ
            || self.output >= MAX_OUTPUT
            || self.started.elapsed() >= Duration::from_secs(120);
        self.page.progress.limit_hit
    }
    fn error(&mut self, error: impl ToString) {
        self.page.progress.errors += 1;
        if self.page.progress.first_error.is_none() {
            self.page.progress.first_error = Some(error.to_string().chars().take(512).collect());
        }
    }
    fn publish(&mut self) -> Result<(), String> {
        let encoded = serde_json::to_vec(&self.page).map_err(|e| e.to_string())?;
        if encoded.len() > 256 * 1024 {
            return Err("Search publication exceeds budget".into());
        }
        self.sink
            .write_all(&encoded)
            .and_then(|_| self.sink.write_all(b"\n"))
            .and_then(|_| self.sink.flush())
            .map_err(|e| e.to_string())?;
        self.output += encoded.len() + 1;
        self.page.files.clear();
        self.published = Instant::now();
        Ok(())
    }
    fn ignore_file(
        &mut self,
        directory: &File,
        name: &str,
        base: &Path,
        rules: &mut Vec<Arc<Gitignore>>,
    ) {
        let file = match open(directory, name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.error(format!("{name}: {error}"));
                return;
            }
        };
        if !file.metadata().is_ok_and(|meta| meta.is_file()) {
            return;
        }
        let mut bytes = Vec::new();
        if let Err(error) = file.take(MAX_IGNORE as u64 + 1).read_to_end(&mut bytes) {
            self.error(error);
            return;
        }
        if bytes.len() > MAX_IGNORE {
            self.error("Ignore file exceeds 256 KiB");
            return;
        }
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                self.error("Ignore file is not UTF-8");
                return;
            }
        };
        self.ignore_bytes += text.len();
        self.ignore_lines += text.lines().count();
        if self.ignore_bytes > MAX_IGNORE_TOTAL || self.ignore_lines > MAX_IGNORE_LINES {
            self.page.progress.limit_hit = true;
            self.error("Search ignore rules exceed the cumulative 1 MiB / 8192 line budget");
            return;
        }
        let mut builder = GitignoreBuilder::new(self.root.join(base));
        for line in text.lines() {
            if let Err(error) = builder.add_line(None, line) {
                self.error(error);
            }
        }
        match builder.build() {
            Ok(rule) => rules.push(Arc::new(rule)),
            Err(error) => self.error(error),
        }
    }
    fn walk(
        &mut self,
        directory: File,
        relative: PathBuf,
        inherited: &[Arc<Gitignore>],
        depth: usize,
    ) -> Result<(), String> {
        if depth > 64 {
            self.page.progress.limit_hit = true;
            return Ok(());
        }
        let mut rules = inherited.to_vec();
        if !self.options.spec.include_ignored {
            if depth == 0
                && let Ok(git) = open(&directory, ".git", true)
                && let Ok(info) = open(&git, "info", true)
            {
                self.ignore_file(&info, "exclude", &relative, &mut rules);
            }
            for name in [".gitignore", ".ignore", ".rgignore"] {
                self.ignore_file(&directory, name, &relative, &mut rules);
            }
        }
        let entries = match std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd())) {
            Ok(entries) => entries,
            Err(error) => {
                self.error(error);
                return Ok(());
            }
        };
        for entry in entries {
            if self.limited() {
                break;
            }
            self.entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.error(error);
                    continue;
                }
            };
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => {
                    self.error("Search skipped a filename that is not UTF-8");
                    continue;
                }
            };
            if !self.options.spec.include_hidden && name.starts_with('.') {
                continue;
            }
            let child = relative.join(&name);
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    self.error(error);
                    continue;
                }
            };
            if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
                continue;
            }
            let full = self.root.join(&child);
            if self.matcher.excludes(&child)
                || rules
                    .iter()
                    .rev()
                    .find_map(|rule| {
                        let found = rule.matched_path_or_any_parents(&full, kind.is_dir());
                        (!found.is_none()).then(|| found.is_ignore())
                    })
                    .unwrap_or(false)
            {
                continue;
            }
            if kind.is_dir() {
                match open(&directory, &name, true) {
                    Ok(next) => self.walk(next, child, &rules, depth + 1)?,
                    Err(error) => self.error(error),
                }
            } else if self.matcher.includes(&child) {
                self.file(&directory, &name, &child)?;
            }
            if self.published.elapsed() >= Duration::from_millis(100) {
                self.publish()?;
            }
        }
        Ok(())
    }
    fn file(&mut self, directory: &File, name: &str, relative: &Path) -> Result<(), String> {
        let full = self
            .root
            .join(relative)
            .into_os_string()
            .into_string()
            .map_err(|_| "Invalid resource identity")?;
        let path = full.clone().try_into()?;
        let relative = relative
            .components()
            .map(|c| c.as_os_str().to_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        if relative.iter().map(String::len).sum::<usize>() > 32 * 1024 {
            self.error("Search path exceeds budget");
            return Ok(());
        }
        if self.buffers.contains(&full) {
            self.page.files.push(SearchFile {
                path,
                name: name.into(),
                relative,
                matches: Vec::new(),
                content_hash: 0,
                buffer: true,
            });
            return self.publish();
        }
        let file = match open(directory, name, false) {
            Ok(file) => file,
            Err(error) => {
                self.error(error);
                return Ok(());
            }
        };
        let meta = match file.metadata() {
            Ok(meta) => meta,
            Err(error) => {
                self.error(error);
                return Ok(());
            }
        };
        if !meta.is_file() {
            return Ok(());
        }
        if meta.len() > MAX_FILE as u64 {
            self.page.progress.skipped_large += 1;
            return Ok(());
        }
        let mut bytes = Vec::new();
        if let Err(error) = file.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes) {
            self.error(error);
            return Ok(());
        }
        self.read += bytes.len();
        if bytes.len() > MAX_FILE {
            self.page.progress.skipped_large += 1;
            return Ok(());
        }
        let text = match encoding::decode(&bytes, None, &self.options.encoding, MAX_FILE) {
            Ok(decoded) => decoded.text,
            Err(error) => {
                if error == "Binary file" {
                    self.page.progress.skipped_binary += 1;
                } else {
                    self.page.progress.skipped_encoding += 1;
                }
                return Ok(());
            }
        };
        self.page.progress.searched += 1;
        let remaining = MAX_MATCHES.saturating_sub(self.matched);
        let mut matches = self
            .matcher
            .matches(&text, &AtomicBool::new(false), remaining + 1);
        if matches.len() > remaining {
            self.page.progress.limit_hit = true;
            matches.truncate(remaining);
        }
        if matches.is_empty() {
            return Ok(());
        }
        self.files += 1;
        if self.files > 5000 {
            self.page.progress.limit_hit = true;
            return Ok(());
        }
        self.matched += matches.len();
        let content_hash = hash(&text);
        for chunk in matches.chunks(32) {
            self.page.files.push(SearchFile {
                path: path.clone(),
                name: name.into(),
                relative: relative.clone(),
                matches: chunk.to_vec(),
                content_hash,
                buffer: false,
            });
            self.publish()?;
            if self.output >= MAX_OUTPUT {
                self.page.progress.limit_hit = true;
                break;
            }
        }
        Ok(())
    }
}

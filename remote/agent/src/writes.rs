//! Mutations: atomic whole-file writes, staged writes, directories, renames and
//! removal. A revision check followed by replacement is not a strict
//! compare-and-swap against arbitrary external writers; `hello` says so.
use crate::agent::MAX_FRAME;
use crate::error::{AgentError, Result, error};
use crate::vfs::{self, MAX_CHUNK, MAX_FILE, Parent, Root};
use base64::Engine;
use serde_json::{Value, json};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CString, OsStr, OsString};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

const STAGE_LIMIT: usize = 16;
const STAGE_BYTES: u64 = 256 * 1024 * 1024;
pub const STAGE_IDLE: Duration = Duration::from_secs(120);

pub struct Stage {
    root: u64,
    parent: OwnedFd,
    name: OsString,
    temporary: Option<OsString>,
    fd: OwnedFd,
    size: u64,
    offset: u64,
    expected: Option<String>,
    mode: u32,
    touched: Instant,
}

#[derive(Default)]
pub struct Writes {
    stages: HashMap<u64, Stage>,
    next: u64,
}

fn decode(params: &Value, key: &str) -> Result<Vec<u8>> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| error("invalid_request", format!("missing {key}")))?;
    base64::engine::general_purpose::STANDARD.decode(text).map_err(|_| error("invalid_request", "invalid base64 data"))
}

fn last_error() -> AgentError { std::io::Error::last_os_error().into() }

fn fsync(fd: i32) -> std::io::Result<()> {
    if unsafe { libc::fsync(fd) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn write_all(fd: i32, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let count = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if count < 0 {
            let source = std::io::Error::last_os_error();
            if source.raw_os_error() == Some(libc::EINTR) { continue; }
            return Err(source.into());
        }
        if count == 0 { return Err(error("failed", "staged write made no progress")); }
        data = &data[count as usize..];
    }
    Ok(())
}

fn temporary_name() -> OsString {
    let mut bytes = [0u8; 16];
    unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(".loog-save-{hex}").into()
}

fn unlinkat(dir: i32, name: &OsStr, flags: i32) -> std::io::Result<()> {
    let name = CString::new(name.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    if unsafe { libc::unlinkat(dir, name.as_ptr(), flags) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn replace(dir: i32, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let (from, to) = (CString::new(from.as_bytes())?, CString::new(to.as_bytes())?);
    if unsafe { libc::renameat(dir, from.as_ptr(), dir, to.as_ptr()) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Checks shared by whole-file and staged writes. Returns the mode to apply.
fn prepare(parent: &Parent, expected: Option<&str>) -> Result<u32> {
    let current = match vfs::fstatat(parent.fd.as_raw_fd(), &parent.name, false) {
        Ok(data) => Some(data),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(source.into()),
    };
    if current.as_ref().is_some_and(|data| !vfs::is(data, libc::S_IFREG as u32)) {
        return Err(error("invalid_request", "resource is not a regular file"));
    }
    if let Some(expected) = expected {
        if current.is_none() || vfs::revision(parent)? != expected {
            return Err(error("conflict", "file differs from the saved revision"));
        }
    }
    let mode = current.map_or(0o600, |data| vfs::mode_of(&data) & 0o7777);
    if mode & 0o222 == 0 { return Err(error("permission_denied", "file is read-only")); }
    Ok(mode)
}

fn expected(params: &Value) -> Option<String> {
    params.get("expected_revision").and_then(Value::as_str).map(str::to_owned)
}

pub fn write(parent: &Parent, params: &Value) -> Result<Value> {
    let data = decode(params, "data")?;
    if data.len() > MAX_CHUNK { return Err(error("limit_exceeded", "write exceeds the chunk budget")); }
    let expected = expected(params);
    let mode = prepare(parent, expected.as_deref())?;
    let temporary = temporary_name();
    let fd = vfs::openat(parent.fd.as_raw_fd(), &temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, mode)?;
    let renamed = Cell::new(false);
    let result = (|| -> Result<Value> {
        write_all(fd.as_raw_fd(), &data)?;
        unsafe { libc::fchmod(fd.as_raw_fd(), mode) };
        fsync(fd.as_raw_fd())?;
        if let Some(expected) = &expected {
            if vfs::revision(parent)? != *expected { return Err(error("conflict", "file changed before replacement")); }
        }
        replace(parent.fd.as_raw_fd(), &temporary, &parent.name)?;
        renamed.set(true);
        let revision = vfs::signature(&vfs::fstat(fd.as_raw_fd())?);
        fsync(parent.fd.as_raw_fd()).map_err(|_| error("unknown_outcome", "file replaced but directory sync failed"))?;
        Ok(json!({"revision": revision}))
    })();
    if !renamed.get() { let _ = unlinkat(parent.fd.as_raw_fd(), &temporary, 0); }
    result
}

pub fn create_file(parent: &Parent) -> Result<Value> {
    let fd = vfs::openat(parent.fd.as_raw_fd(), &parent.name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW, 0o600)?;
    let confirm = || -> std::io::Result<()> { fsync(fd.as_raw_fd())?; fsync(parent.fd.as_raw_fd()) };
    confirm().map_err(|_| error("unknown_outcome", "file created but confirmation failed"))?;
    vfs::info(parent.fd.as_raw_fd(), &parent.name, &parent.path, true)
        .map_err(|_| error("unknown_outcome", "file created but confirmation failed"))
}

pub fn mkdir(parent: &Parent) -> Result<Value> {
    let name = CString::new(parent.name.as_bytes()).map_err(|_| error("invalid_request", "invalid resource path"))?;
    if unsafe { libc::mkdirat(parent.fd.as_raw_fd(), name.as_ptr(), 0o777) } != 0 { return Err(last_error()); }
    Ok(json!({}))
}

pub fn remove(root: &Root, parent: &Parent, recursive: bool) -> Result<Value> {
    if parent.path == root.path { return Err(error("permission_denied", "cannot remove a mounted root")); }
    let data = vfs::fstatat(parent.fd.as_raw_fd(), &parent.name, false)?;
    if vfs::is(&data, libc::S_IFDIR as u32) {
        if recursive { return Err(error("unsupported", "recursive deletion is not available")); }
        unlinkat(parent.fd.as_raw_fd(), &parent.name, libc::AT_REMOVEDIR)?;
    } else {
        unlinkat(parent.fd.as_raw_fd(), &parent.name, 0)?;
    }
    Ok(json!({}))
}

fn rename_noreplace(source: &Parent, target: &Parent) -> Result<()> {
    // Linux renameat2 RENAME_NOREPLACE checks and renames atomically.
    let (from, to) = (CString::new(source.name.as_bytes()).unwrap(), CString::new(target.name.as_bytes()).unwrap());
    let code = unsafe { libc::syscall(libc::SYS_renameat2, source.fd.as_raw_fd(), from.as_ptr(),
        target.fd.as_raw_fd(), to.as_ptr(), 1 as libc::c_uint) };
    if code == 0 { return Ok(()); }
    let source = std::io::Error::last_os_error();
    match source.raw_os_error() {
        Some(libc::EEXIST) => Err(error("conflict", "target already exists")),
        Some(libc::ENOSYS) | Some(libc::EINVAL) | Some(libc::EOPNOTSUPP) =>
            Err(error("unsupported", "atomic no-replace rename is unavailable")),
        _ => Err(source.into()),
    }
}

pub fn rename(root: &Root, params: &Value, relocate: bool) -> Result<Value> {
    let source = vfs::parent(root, &params["source"], false)?;
    let target = vfs::parent(root, &params["target"], false)?;
    if source.path == root.path || target.path == root.path {
        return Err(error("permission_denied", "cannot rename a mounted root"));
    }
    let mut entries = Vec::new();
    if relocate {
        let tracked = params["tracked"].as_array().filter(|list| list.len() <= 1024)
            .ok_or_else(|| error("limit_exceeded", "too many tracked resources"))?;
        // Path interpretation stays in the provider: no client-side prefix test
        // or platform separator is needed for descendants.
        let mut normalized = Vec::new();
        for value in tracked { normalized.push((value.clone(), vfs::resolve(root, value, false)?)); }
        for (value, old) in &normalized {
            if let Ok(suffix) = Path::new(old).strip_prefix(&source.path) {
                let new = if suffix.as_os_str().is_empty() { target.path.clone() } else {
                    vfs::utf8(&Path::new(&target.path).join(suffix))?
                };
                let name = Path::new(&new).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                entries.push(json!({"source": value, "target": new, "name": name}));
            }
        }
        let moved: HashSet<&str> = normalized.iter().filter(|(value, _)| entries.iter().any(|e| e["source"] == *value))
            .map(|(_, old)| old.as_str()).collect();
        let occupied: HashSet<&str> = normalized.iter().map(|(_, old)| old.as_str()).filter(|old| !moved.contains(old)).collect();
        if entries.iter().any(|entry| occupied.contains(entry["target"].as_str().unwrap_or(""))) {
            return Err(error("conflict", "target is already open as another resource"));
        }
        if serde_json::to_vec(&entries).unwrap_or_default().len() > MAX_FRAME / 2 {
            return Err(error("limit_exceeded", "rename mapping exceeds the result budget"));
        }
    }
    if !relocate && params.get("overwrite").and_then(Value::as_bool).unwrap_or(false) {
        let (from, to) = (CString::new(source.name.as_bytes()).unwrap(), CString::new(target.name.as_bytes()).unwrap());
        if unsafe { libc::renameat(source.fd.as_raw_fd(), from.as_ptr(), target.fd.as_raw_fd(), to.as_ptr()) } != 0 {
            return Err(last_error());
        }
    } else {
        rename_noreplace(&source, &target)?;
    }
    let synced = fsync(source.fd.as_raw_fd()).and_then(|_| fsync(target.fd.as_raw_fd()));
    synced.map_err(|_| error("unknown_outcome", "resource renamed but directory sync failed"))?;
    Ok(if relocate { json!({"entries": entries}) } else { json!({}) })
}

impl Writes {
    pub fn expire(&mut self) {
        let stale: Vec<u64> = self.stages.iter().filter(|(_, stage)| stage.touched.elapsed() > STAGE_IDLE).map(|(id, _)| *id).collect();
        for id in stale { self.abort(id); }
    }

    pub fn abort(&mut self, id: u64) {
        if let Some(stage) = self.stages.remove(&id) {
            if let Some(temporary) = &stage.temporary {
                let _ = unlinkat(stage.parent.as_raw_fd(), temporary, 0);
            }
        }
    }

    pub fn abort_all(&mut self) {
        let ids: Vec<u64> = self.stages.keys().copied().collect();
        for id in ids { self.abort(id); }
    }

    pub fn begin(&mut self, root_id: u64, parent: &Parent, params: &Value) -> Result<Value> {
        let size = params["size"].as_u64().filter(|size| *size <= MAX_FILE)
            .ok_or_else(|| error("limit_exceeded", "invalid staged file size"))?;
        if self.stages.len() >= STAGE_LIMIT || self.stages.values().map(|stage| stage.size).sum::<u64>() + size > STAGE_BYTES {
            return Err(error("limit_exceeded", "staged write budget exceeded"));
        }
        let expected = expected(params);
        let mode = prepare(parent, expected.as_deref())?;
        let temporary = temporary_name();
        let retained = vfs::dup(parent.fd.as_raw_fd())?;
        let fd = vfs::openat(retained.as_raw_fd(), &temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, mode)?;
        self.next += 1;
        self.stages.insert(self.next, Stage { root: root_id, parent: retained, name: parent.name.clone(),
            temporary: Some(temporary), fd, size, offset: 0, expected, mode, touched: Instant::now() });
        Ok(json!({"write": self.next}))
    }

    pub fn step(&mut self, method: &str, root_id: u64, params: &Value) -> Result<Value> {
        let id = params["write"].as_u64().unwrap_or(0);
        let Some(stage) = self.stages.get_mut(&id) else {
            if method == "write_abort" { return Ok(json!({})); }
            return Err(error("unavailable", "staged write is no longer available"));
        };
        if stage.root != root_id { return Err(error("permission_denied", "staged write belongs to another root")); }
        stage.touched = Instant::now();
        match method {
            "write_abort" => { self.abort(id); Ok(json!({})) }
            "write_append" => {
                let data = decode(params, "data")?;
                let offset = params["offset"].as_u64();
                if offset != Some(stage.offset) || data.len() > MAX_CHUNK || stage.offset + data.len() as u64 > stage.size {
                    return Err(error("invalid_request", "invalid staged write offset or size"));
                }
                write_all(stage.fd.as_raw_fd(), &data)?;
                stage.offset += data.len() as u64;
                Ok(json!({}))
            }
            "write_commit" => {
                if stage.offset != stage.size { return Err(error("invalid_request", "staged write is incomplete")); }
                let result = Self::commit(stage);
                if result.is_ok() { stage.temporary = None; }
                self.abort(id);
                result
            }
            _ => Err(error("unsupported", "unknown staged write operation")),
        }
    }

    fn commit(stage: &Stage) -> Result<Value> {
        unsafe { libc::fchmod(stage.fd.as_raw_fd(), stage.mode) };
        fsync(stage.fd.as_raw_fd())?;
        let target = Parent { fd: vfs::dup(stage.parent.as_raw_fd())?, name: stage.name.clone(), path: String::new() };
        if let Some(expected) = &stage.expected {
            let current = match vfs::revision(&target) {
                Ok(revision) => Some(revision),
                Err(AgentError { kind: "not_found", .. }) => None,
                Err(other) => return Err(other),
            };
            if current.as_deref() != Some(expected.as_str()) { return Err(error("conflict", "file changed before replacement")); }
        }
        replace(stage.parent.as_raw_fd(), stage.temporary.as_deref().unwrap(), &stage.name)?;
        let revision = vfs::signature(&vfs::fstat(stage.fd.as_raw_fd())?);
        fsync(stage.parent.as_raw_fd()).map_err(|_| error("unknown_outcome", "file replaced but directory sync failed"))?;
        Ok(json!({"revision": revision}))
    }
}

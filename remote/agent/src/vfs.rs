//! Path scope and descriptor-relative file access. Every operation descends from
//! a retained root descriptor with `O_NOFOLLOW`, so a path swapped to a symlink
//! after resolution is refused, never followed.
use crate::error::{AgentError, Result, error, not_utf8};
use serde_json::{Value, json};
use std::collections::BinaryHeap;
use std::ffi::{CString, OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

pub const MAX_FILE: u64 = 64 * 1024 * 1024;
pub const MAX_CHUNK: usize = 256 * 1024;
pub const DIRECTORY_FLAGS: i32 = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

pub fn openat(dir: i32, name: &OsStr, flags: i32, mode: u32) -> std::io::Result<OwnedFd> {
    let name = CString::new(name.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let fd = unsafe { libc::openat(dir, name.as_ptr(), flags | libc::O_CLOEXEC, mode) };
    if fd < 0 { Err(std::io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd) }) }
}

pub fn fstat(fd: i32) -> std::io::Result<libc::stat> {
    let mut data: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut data) } == 0 { Ok(data) } else { Err(std::io::Error::last_os_error()) }
}

pub fn fstatat(dir: i32, name: &OsStr, follow: bool) -> std::io::Result<libc::stat> {
    let name = CString::new(name.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let mut data: libc::stat = unsafe { std::mem::zeroed() };
    let flags = if follow { 0 } else { libc::AT_SYMLINK_NOFOLLOW };
    if unsafe { libc::fstatat(dir, name.as_ptr(), &mut data, flags) } == 0 { Ok(data) } else { Err(std::io::Error::last_os_error()) }
}

pub fn mode_of(data: &libc::stat) -> u32 { data.st_mode as u32 }
pub fn is(data: &libc::stat, kind: u32) -> bool { mode_of(data) & libc::S_IFMT as u32 == kind }
fn nanos(sec: i64, nsec: i64) -> i128 { sec as i128 * 1_000_000_000 + nsec as i128 }
pub fn mtime_ns(data: &libc::stat) -> i128 { nanos(data.st_mtime as i64, data.st_mtime_nsec as i64) }
pub fn ctime_ns(data: &libc::stat) -> i128 { nanos(data.st_ctime as i64, data.st_ctime_nsec as i64) }

/// An opaque observation token: inode identity plus size and nanosecond times.
/// It is not a content hash and not a strict compare-and-swap lock.
pub fn signature(data: &libc::stat) -> String {
    format!("{}:{}:{}:{}:{}", data.st_dev as u64, data.st_ino as u64, data.st_size, mtime_ns(data), ctime_ns(data))
}

/// Lexical normalization of an absolute path: no `.`, no `..`, no empty parts.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::Normal(name) => out.push(name),
            Component::ParentDir => { out.pop(); }
            _ => {}
        }
    }
    out
}

/// Resolve every existing link; a missing tail is kept as written (a file about
/// to be created). A cycle or an unreadable link stays unresolved.
pub fn realpath(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::from("/");
    let mut todo: Vec<OsString> = path.components().rev().filter_map(|part| match part {
        Component::Normal(name) => Some(name.to_owned()),
        Component::ParentDir => Some("..".into()),
        _ => None,
    }).collect();
    let mut links = 0;
    while let Some(part) = todo.pop() {
        if part == ".." { resolved.pop(); continue; }
        let next = resolved.join(&part);
        match std::fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                links += 1;
                let Ok(target) = std::fs::read_link(&next) else { resolved = next; continue };
                if links > 40 { resolved = next; continue }
                if target.is_absolute() { resolved = PathBuf::from("/"); }
                for part in target.components().rev() {
                    match part {
                        Component::Normal(name) => todo.push(name.to_owned()),
                        Component::ParentDir => todo.push("..".into()),
                        _ => {}
                    }
                }
            }
            _ => resolved = next,
        }
    }
    resolved
}

pub fn utf8(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(not_utf8)
}

/// A directory descriptor, the leaf name inside it, and the canonical path.
pub struct Parent {
    pub fd: OwnedFd,
    pub name: OsString,
    pub path: String,
}

pub struct Root {
    pub path: String,
    pub fd: OwnedFd,
}

pub fn resolve(root: &Root, value: &Value, follow: bool) -> Result<String> {
    let text = value.as_str().filter(|text| !text.is_empty() && !text.contains('\0'))
        .ok_or_else(|| error("invalid_request", "invalid resource path"))?;
    let candidate = if text.starts_with('/') { PathBuf::from(text) } else { Path::new(&root.path).join(text) };
    let candidate = normalize(&candidate);
    let resolved = if follow {
        realpath(&candidate)
    } else {
        let parent = candidate.parent().unwrap_or(Path::new("/"));
        realpath(parent).join(candidate.file_name().unwrap_or_default())
    };
    if !resolved.starts_with(&root.path) {
        return Err(error("permission_denied", "resource is outside the workspace"));
    }
    utf8(&resolved)
}

pub fn parent(root: &Root, value: &Value, follow: bool) -> Result<Parent> {
    let path = resolve(root, value, follow)?;
    open_parent(root, path)
}

pub fn open_parent(root: &Root, path: String) -> Result<Parent> {
    let relative = Path::new(&path).strip_prefix(&root.path).map_err(|_| error("permission_denied", "resource is outside the workspace"))?;
    let parts: Vec<&OsStr> = relative.components().map(|part| part.as_os_str()).collect();
    let mut fd = dup(root.fd.as_raw_fd())?;
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        fd = openat(fd.as_raw_fd(), part, DIRECTORY_FLAGS, 0)?;
    }
    let name = parts.last().map_or(OsString::from("."), |name| (*name).to_owned());
    Ok(Parent { fd, name, path })
}

pub fn dup(fd: i32) -> std::io::Result<OwnedFd> {
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if copy < 0 { Err(std::io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(copy) }) }
}

/// Open a regular file for reading: never a link, never waits for a FIFO writer.
pub fn open_regular(parent: &Parent) -> Result<(OwnedFd, libc::stat)> {
    let fd = openat(parent.fd.as_raw_fd(), &parent.name, libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW, 0)?;
    let data = fstat(fd.as_raw_fd())?;
    if !is(&data, libc::S_IFREG as u32) { return Err(error("invalid_request", "resource is not a regular file")); }
    if data.st_size as u64 > MAX_FILE { return Err(error("limit_exceeded", "file exceeds the editable size budget")); }
    Ok((fd, data))
}

pub fn revision(parent: &Parent) -> Result<String> {
    let (_, data) = open_regular(parent)?;
    Ok(signature(&data))
}

pub fn info(parent_fd: i32, name: &OsStr, path: &str, include_revision: bool) -> Result<Value> {
    let data = fstatat(parent_fd, name, false)?;
    let kind = if is(&data, libc::S_IFLNK as u32) { "symlink" }
        else if is(&data, libc::S_IFDIR as u32) { "directory" }
        else if is(&data, libc::S_IFREG as u32) { "file" } else { "other" };
    let revision = if kind == "file" && include_revision {
        let (_, data) = open_regular(&Parent { fd: dup(parent_fd)?, name: name.to_owned(), path: path.into() })?;
        Some(signature(&data))
    } else { None };
    let base = Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(json!({"path": path, "name": base, "kind": kind, "size": data.st_size,
        "readonly": mode_of(&data) & 0o222 == 0, "revision": revision}))
}

/// One page of a directory, in name order, holding at most one page in memory.
pub fn list(parent: &Parent, limit: usize, cursor: Option<&str>) -> Result<Value> {
    let dir = openat(parent.fd.as_raw_fd(), &parent.name, DIRECTORY_FLAGS, 0)?;
    let reader = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
    let mut heap: BinaryHeap<Vec<u8>> = BinaryHeap::new();
    for entry in reader {
        let name = entry?.file_name().into_vec();
        if cursor.is_some_and(|cursor| name.as_slice() <= cursor.as_bytes()) { continue; }
        heap.push(name);
        if heap.len() > limit + 1 { heap.pop(); }
    }
    let names = heap.into_sorted_vec();
    let more = names.len() > limit;
    let page = &names[..names.len().min(limit)];
    let mut entries = Vec::new();
    for name in page {
        let name = OsString::from_vec(name.clone());
        let full = Path::new(&parent.path).join(&name);
        let text = utf8(&full)?;
        match info(dir.as_raw_fd(), &name, &text, false) {
            Err(AgentError { kind: "not_found", .. }) => continue,
            other => entries.push(other?),
        }
    }
    let next = if more { page.last().map(|name| String::from_utf8_lossy(name).into_owned()) } else { None };
    Ok(json!({"entries": entries, "cursor": next}))
}

pub fn read(parent: &Parent, offset: u64, limit: usize) -> Result<Value> {
    use base64::Engine;
    let (fd, before) = open_regular(parent)?;
    let mut data = vec![0u8; limit];
    let mut filled = 0;
    while filled < limit {
        let count = unsafe { libc::pread(fd.as_raw_fd(), data[filled..].as_mut_ptr().cast(), limit - filled, (offset + filled as u64) as i64) };
        if count < 0 { return Err(std::io::Error::last_os_error().into()); }
        if count == 0 { break; }
        filled += count as usize;
    }
    data.truncate(filled);
    let after = fstat(fd.as_raw_fd())?;
    if signature(&before) != signature(&after) { return Err(error("conflict", "file changed during read")); }
    Ok(json!({"data": base64::engine::general_purpose::STANDARD.encode(&data),
        "eof": offset + filled as u64 >= after.st_size as u64,
        "revision": signature(&after),
        "observation": [after.st_size, mtime_ns(&after) as i64, ctime_ns(&after) as i64]}))
}

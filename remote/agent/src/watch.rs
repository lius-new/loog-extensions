//! Bounded recursive inotify subscription, owned by one mounted root. Directory
//! descriptors prevent following links during registration. Directory moves and
//! kernel overflow rebuild the subscription and require a rescan.
//! https://man7.org/linux/man-pages/man7/inotify.7.html
use crate::error::{AgentError, Result, error, not_utf8};
use crate::vfs::{self, DIRECTORY_FLAGS, Root};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

// CLOSE_WRITE, ATTRIB, moves, creation/deletion and self movement, directories only.
const MASK: u32 = 0x0100_0FCC;
const OVERFLOW: u32 = 0x4000;
const IGNORED: u32 = 0x8000;
const UNMOUNT: u32 = 0x2000;
const DELETE_SELF: u32 = 0x400;
const MOVE_SELF: u32 = 0x800;
const IS_DIR: u32 = 0x4000_0000;
const TOPOLOGY: u32 = 0x40 | 0x80 | 0x100 | 0x200;

pub struct Watch {
    pub root: u64,
    pub ready: bool,
    path: String,
    fd: OwnedFd,
    paths: HashMap<i32, String>,
    sequence: u64,
}

fn missing(source: &AgentError) -> bool {
    source.kind == "not_found" || (source.kind == "failed" && source.message.contains("Not a directory"))
}

fn build(root: &Root, start: &str) -> Result<(OwnedFd, HashMap<i32, String>)> {
    let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if raw < 0 { return Err(error("failed", "cannot start filesystem monitoring")); }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut paths = HashMap::new();
    let (mut seen, mut stack, mut entries) = (HashSet::new(), vec![(start.to_owned(), 0usize)], 0usize);
    while let Some((path, depth)) = stack.pop() {
        if paths.len() >= 4096 || depth > 64 {
            return Err(error("limit_exceeded", "filesystem watch directory budget exceeded"));
        }
        let attempt = (|| -> Result<()> {
            let parent = vfs::parent(root, &Value::String(path.clone()), true)?;
            let directory = vfs::openat(parent.fd.as_raw_fd(), &parent.name, DIRECTORY_FLAGS, 0)?;
            let data = vfs::fstat(directory.as_raw_fd())?;
            if !seen.insert((data.st_dev as u64, data.st_ino as u64)) { return Ok(()); }
            // Register the opened inode, never a symlink.
            let link = CString::new(format!("/proc/self/fd/{}", directory.as_raw_fd())).unwrap();
            let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), link.as_ptr(), MASK) };
            if wd < 0 { return Err(std::io::Error::last_os_error().into()); }
            paths.insert(wd, parent.path.clone());
            for child in std::fs::read_dir(Path::new(&format!("/proc/self/fd/{}", directory.as_raw_fd())))? {
                let child = child?;
                entries += 1;
                if entries > 65536 { return Err(error("limit_exceeded", "filesystem watch entry budget exceeded")); }
                if child.file_type()?.is_dir() {
                    let name = child.file_name().into_string().map_err(|_| not_utf8())?;
                    if stack.len() >= 4096 { return Err(error("limit_exceeded", "filesystem watch directory budget exceeded")); }
                    stack.push((format!("{}/{}", parent.path.trim_end_matches('/'), name), depth + 1));
                }
            }
            Ok(())
        })();
        // A descendant can disappear or become a file between enumeration and
        // registration. Its watched parent reports the topology change; losing
        // the subscription root remains an explicit failure.
        match attempt {
            Err(source) if missing(&source) && path != start => {}
            other => other?,
        }
    }
    Ok((fd, paths))
}

impl Watch {
    pub fn new(root_id: u64, root: &Root, path: &Value) -> Result<Watch> {
        let path = vfs::resolve(root, path, true)?;
        let (fd, paths) = build(root, &path)?;
        Ok(Watch { root: root_id, ready: false, path, fd, paths, sequence: 0 })
    }

    pub fn raw_fd(&self) -> i32 { self.fd.as_raw_fd() }

    pub fn poll(&mut self, root: &Root) -> Result<Option<Value>> {
        if !self.ready { return Ok(None); }
        let (mut changed, mut rescan, mut rebuild, mut observed, mut bytes) = (BTreeSet::new(), false, false, false, 0usize);
        let mut drained = false;
        let mut buffer = vec![0u8; 65536];
        for _ in 0..16 {
            let count = unsafe { libc::read(self.fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
            if count <= 0 { drained = true; break; }
            let data = &buffer[..count as usize];
            let mut offset = 0;
            while offset + 16 <= data.len() {
                let word = |at: usize| u32::from_ne_bytes(data[offset + at..offset + at + 4].try_into().unwrap());
                let (wd, mask, length) = (word(0) as i32, word(4), word(12) as usize);
                let name_end = (offset + 16 + length).min(data.len());
                let name = data[offset + 16..name_end].split(|byte| *byte == 0).next().unwrap_or(&[]);
                offset = name_end;
                observed = true;
                if mask & (OVERFLOW | IGNORED | UNMOUNT | DELETE_SELF | MOVE_SELF) != 0 { rescan = true; rebuild = true; }
                let Some(parent) = self.paths.get(&wd) else { rescan = true; continue };
                if mask & IS_DIR != 0 && mask & TOPOLOGY != 0 { rescan = true; rebuild = true; }
                let path = if name.is_empty() { parent.clone() } else {
                    match std::str::from_utf8(name) {
                        Ok(name) => format!("{}/{}", parent.trim_end_matches('/'), name),
                        Err(_) => { rescan = true; continue }
                    }
                };
                for affected in [parent.clone(), path] {
                    if !changed.contains(&affected) {
                        bytes += affected.len();
                        if changed.len() < 512 && bytes <= 128 * 1024 { changed.insert(affected); } else { rescan = true; }
                    }
                }
            }
        }
        // More events were pending after the bounded reads: subscribe again.
        if !drained { rescan = true; rebuild = true; }
        if !observed { return Ok(None); }
        if rebuild {
            let (fd, paths) = build(root, &self.path)?;
            self.fd = fd;
            self.paths = paths;
        }
        self.sequence += 1;
        Ok(Some(json!({"kind": "changed", "sequence": self.sequence, "paths": changed, "rescan": rescan})))
    }
}

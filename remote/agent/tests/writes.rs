//! Mutations: atomic and staged writes, directories, renames and removal.
mod common;
use common::*;
use serde_json::json;
use std::path::Path;
use std::process::Command;

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn temporaries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with(".loog-save-")).collect()
}

#[test]
fn a_whole_file_write_replaces_atomically_and_checks_the_revision() {
    let dir = workspace("write");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let created = session.ok("write", json!({"root": root, "path": "f.txt", "data": b64(b"one")}));
    assert_eq!(std::fs::read(dir.join("f.txt")).unwrap(), b"one");
    let revision = created["revision"].as_str().unwrap().to_owned();
    assert_eq!(session.ok("stat", json!({"root": root, "path": "f.txt"}))["revision"], revision);
    let second = session.ok("write", json!({"root": root, "path": "f.txt", "data": b64(b"two"), "expected_revision": revision}));
    assert_ne!(second["revision"], revision);
    assert_eq!(session.kind("write", json!({"root": root, "path": "f.txt", "data": b64(b"x"), "expected_revision": revision})), "conflict");
    assert_eq!(std::fs::read(dir.join("f.txt")).unwrap(), b"two");
    std::fs::write(dir.join("f.txt"), "external edit that is longer").unwrap();
    let current = second["revision"].as_str().unwrap();
    assert_eq!(session.kind("write", json!({"root": root, "path": "f.txt", "data": b64(b"y"), "expected_revision": current})), "conflict");
    assert_eq!(session.kind("write", json!({"root": root, "path": "missing", "data": b64(b"y"), "expected_revision": "1:2:3:4:5"})), "conflict");
    assert!(temporaries(&dir).is_empty(), "a refused write leaves no temporary file");
}

#[test]
fn read_only_files_directories_and_internal_links_are_handled() {
    use std::os::unix::fs::PermissionsExt;
    let dir = workspace("modes");
    std::fs::write(dir.join("ro"), "r").unwrap();
    std::fs::set_permissions(dir.join("ro"), std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::write(dir.join("real"), "a").unwrap();
    std::fs::set_permissions(dir.join("real"), std::fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink("real", dir.join("link")).unwrap();
    std::fs::create_dir(dir.join("d")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    assert_eq!(session.kind("write", json!({"root": root, "path": "ro", "data": b64(b"x")})), "permission_denied");
    assert_eq!(session.kind("write", json!({"root": root, "path": "d", "data": b64(b"x")})), "invalid_request");
    session.ok("write", json!({"root": root, "path": "link", "data": b64(b"b")}));
    assert!(std::fs::symlink_metadata(dir.join("link")).unwrap().file_type().is_symlink(), "saving through an internal link keeps the link");
    assert_eq!(std::fs::read(dir.join("real")).unwrap(), b"b");
    assert_eq!(std::fs::metadata(dir.join("real")).unwrap().permissions().mode() & 0o777, 0o640, "the mode is preserved");
}

#[test]
fn a_staged_save_keeps_the_original_until_commit_and_bounds_offsets() {
    let dir = workspace("staged");
    std::fs::write(dir.join("f"), "original").unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let stat = session.ok("stat", json!({"root": root, "path": "f"}));
    let write = session.ok("write_begin", json!({"root": root, "path": "f", "size": 6, "expected_revision": stat["revision"]}))["write"].clone();
    session.ok("write_append", json!({"root": root, "write": write, "offset": 0, "data": b64(b"abc")}));
    assert_eq!(session.kind("write_append", json!({"root": root, "write": write, "offset": 0, "data": b64(b"x")})), "invalid_request");
    assert_eq!(session.kind("write_append", json!({"root": root, "write": write, "offset": 3, "data": b64(b"defg")})), "invalid_request");
    assert_eq!(session.kind("write_commit", json!({"root": root, "write": write})), "invalid_request", "incomplete");
    session.ok("write_append", json!({"root": root, "write": write, "offset": 3, "data": b64(b"def")}));
    assert_eq!(std::fs::read(dir.join("f")).unwrap(), b"original");
    assert_eq!(temporaries(&dir).len(), 1);
    let done = session.ok("write_commit", json!({"root": root, "write": write}));
    assert_eq!(std::fs::read(dir.join("f")).unwrap(), b"abcdef");
    assert!(done["revision"].is_string());
    assert!(temporaries(&dir).is_empty());
    assert_eq!(session.kind("write_commit", json!({"root": root, "write": write})), "unavailable");
    assert_eq!(session.ok("write_abort", json!({"root": root, "write": write})), json!({}));
}

#[test]
fn a_staged_conflict_abort_and_disconnect_remove_the_temporary_file() {
    let dir = workspace("staged-clean");
    std::fs::write(dir.join("f"), "original").unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let stat = session.ok("stat", json!({"root": root, "path": "f"}));
    let write = session.ok("write_begin", json!({"root": root, "path": "f", "size": 1, "expected_revision": stat["revision"]}))["write"].clone();
    session.ok("write_append", json!({"root": root, "write": write, "offset": 0, "data": b64(b"z")}));
    std::fs::write(dir.join("f"), "changed externally").unwrap();
    assert_eq!(session.kind("write_commit", json!({"root": root, "write": write})), "conflict");
    assert!(temporaries(&dir).is_empty(), "a conflict removes the stage");
    assert_eq!(std::fs::read(dir.join("f")).unwrap(), b"changed externally");

    let aborted = session.ok("write_begin", json!({"root": root, "path": "g", "size": 1}))["write"].clone();
    assert_eq!(temporaries(&dir).len(), 1);
    session.ok("write_abort", json!({"root": root, "write": aborted}));
    assert!(temporaries(&dir).is_empty());

    session.ok("write_begin", json!({"root": root, "path": "g", "size": 1}));
    assert_eq!(temporaries(&dir).len(), 1);
    session.input.take();
    session.child.wait().unwrap();
    assert!(temporaries(&dir).is_empty(), "disconnecting removes unfinished writes");
}

#[test]
fn hangup_and_termination_clean_uncommitted_writes() {
    for signal in ["HUP", "TERM"] {
        let dir = workspace(&format!("signal-{signal}"));
        let mut session = Session::start();
        let root = mount(&mut session, &dir);
        session.ok("write_begin", json!({"root": root, "path": "f", "size": 4}));
        assert_eq!(temporaries(&dir).len(), 1);
        Command::new("kill").args([format!("-{signal}"), session.child.id().to_string()]).status().unwrap();
        session.child.wait().unwrap();
        assert!(temporaries(&dir).is_empty(), "{signal}");
    }
}

#[test]
fn directories_files_and_removal_report_conflicts_and_protect_the_root() {
    let dir = workspace("tree");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let base = dir.to_str().unwrap();
    session.ok("mkdir", json!({"root": root, "path": "d"}));
    assert_eq!(session.kind("mkdir", json!({"root": root, "path": "d"})), "conflict");
    let created = session.ok("create_file", json!({"root": root, "path": "d/new.txt"}));
    assert_eq!(created["kind"], "file");
    assert_eq!(created["size"], 0);
    assert_eq!(session.kind("create_file", json!({"root": root, "path": "d/new.txt"})), "conflict");
    assert_eq!(session.kind("remove", json!({"root": root, "path": "d"})), "failed", "a folder with content stays");
    assert_eq!(session.kind("remove", json!({"root": root, "path": "d", "recursive": true})), "unsupported");
    session.ok("remove", json!({"root": root, "path": "d/new.txt"}));
    session.ok("remove", json!({"root": root, "path": "d"}));
    assert!(!dir.join("d").exists());
    assert_eq!(session.kind("remove", json!({"root": root, "path": "d"})), "not_found");
    assert_eq!(session.kind("remove", json!({"root": root, "path": base})), "permission_denied");
    assert_eq!(session.kind("rename", json!({"root": root, "source": base, "target": "x"})), "permission_denied");
}

#[test]
fn rename_never_replaces_unless_asked_and_relocate_maps_descendants_only() {
    let dir = workspace("rename");
    for name in ["a", "b", "dir2"] { std::fs::write(dir.join(name), name).unwrap(); }
    std::fs::create_dir(dir.join("dir")).unwrap();
    std::fs::write(dir.join("dir/inner"), "i").unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let base = dir.to_str().unwrap();
    assert_eq!(session.kind("rename", json!({"root": root, "source": "a", "target": "b"})), "conflict");
    assert_eq!(std::fs::read(dir.join("b")).unwrap(), b"b");
    session.ok("rename", json!({"root": root, "source": "a", "target": "b", "overwrite": true}));
    assert_eq!(std::fs::read(dir.join("b")).unwrap(), b"a");
    assert_eq!(session.kind("rename", json!({"root": root, "source": "nothing", "target": "z"})), "not_found");

    let tracked = json!([format!("{base}/dir"), format!("{base}/dir/inner"), format!("{base}/dir2")]);
    let moved = session.ok("relocate", json!({"root": root, "source": "dir", "target": "renamed", "tracked": tracked}));
    let entries = moved["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "the sibling dir2 shares only a prefix: {entries:?}");
    assert_eq!(entries[0]["target"], format!("{base}/renamed"));
    assert_eq!(entries[1]["target"], format!("{base}/renamed/inner"));
    assert_eq!(entries[1]["name"], "inner");
    assert!(dir.join("renamed/inner").exists());

    std::fs::create_dir(dir.join("src")).unwrap();
    let tracked = json!([format!("{base}/src"), format!("{base}/taken")]);
    assert_eq!(session.kind("relocate", json!({"root": root, "source": "src", "target": "taken", "tracked": tracked})), "conflict");
    assert!(dir.join("src").exists(), "a refused relocate changes nothing");
}

#[test]
fn non_utf8_names_fail_explicitly_and_a_fifo_is_rejected_without_waiting() {
    use std::os::unix::ffi::OsStrExt;
    let dir = workspace("names");
    std::fs::write(dir.join(std::ffi::OsStr::from_bytes(b"bad-\xff")), "x").unwrap();
    Command::new("mkfifo").arg(dir.join("pipe")).status().unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    assert_eq!(session.kind("list", json!({"root": root, "path": "."})), "unsupported");
    assert_eq!(session.kind("read", json!({"root": root, "path": "pipe"})), "invalid_request");
}

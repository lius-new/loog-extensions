//! Protocol behavior shared by every method.
mod common;
use common::*;
use serde_json::json;
use std::path::Path;

#[test]
fn hello_names_the_platform_and_the_protocol() {
    let mut session = Session::start();
    let hello = session.ok("hello", json!({}));
    assert_eq!(hello["protocol"], 2);
    assert_eq!(hello["platform"], "linux");
    assert!(hello["machine"].as_str().is_some_and(|machine| !machine.is_empty()));
}

#[test]
fn a_lone_tilde_mounts_the_login_directory_and_nothing_else_expands() {
    let mut session = Session::start();
    let home = std::env::var("HOME").unwrap();
    let mounted = session.ok("mount", json!({"path": "~"}));
    assert_eq!(Path::new(mounted["path"].as_str().unwrap()), std::fs::canonicalize(home).unwrap());
    assert_eq!(session.kind("mount", json!({"path": "~/nothing-here"})), "not_found");
}

#[test]
fn browse_lists_folders_and_files_outside_any_root_and_says_why_it_cannot() {
    let dir = workspace("browse");
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("b.txt"), "x").unwrap();
    std::os::unix::fs::symlink(dir.join("sub"), dir.join("link")).unwrap();
    std::os::unix::fs::symlink(dir.join("missing"), dir.join("broken")).unwrap();
    let mut session = Session::start();
    // No folder is mounted: choosing one comes before opening one.
    let listed = session.ok("browse", json!({"path": dir.to_str().unwrap()}));
    assert_eq!(Path::new(listed["path"].as_str().unwrap()), std::fs::canonicalize(&dir).unwrap());
    let entry = |name: &str| listed["entries"].as_array().unwrap().iter().find(|e| e["name"] == name).unwrap().clone();
    let names: Vec<_> = listed["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["b.txt", "broken", "link", "sub"], "in name order");
    assert_eq!((entry("sub")["kind"].as_str(), entry("sub")["link"].as_bool()), (Some("directory"), Some(false)));
    assert_eq!((entry("b.txt")["kind"].as_str(), entry("b.txt")["link"].as_bool()), (Some("file"), Some(false)));
    assert_eq!((entry("link")["kind"].as_str(), entry("link")["link"].as_bool()), (Some("directory"), Some(true)));
    assert_eq!((entry("broken")["kind"].as_str(), entry("broken")["link"].as_bool()), (Some("other"), Some(true)));
    assert!(listed["entries"][0].get("size").is_none(), "only names and kinds cross");
    assert_eq!(listed["more"], false);

    let cut = session.ok("browse", json!({"path": dir.to_str().unwrap(), "limit": 2}));
    assert_eq!(cut["entries"].as_array().unwrap().len(), 2);
    assert_eq!(cut["more"], true);

    // The login folder by its lone mark, nothing else expanded.
    let home = session.ok("browse", json!({"path": "~"}));
    assert_eq!(Path::new(home["path"].as_str().unwrap()), std::fs::canonicalize(std::env::var("HOME").unwrap()).unwrap());
    assert_eq!(session.kind("browse", json!({"path": dir.join("missing").to_str().unwrap()})), "not_found");
    assert_eq!(session.kind("browse", json!({"path": dir.join("b.txt").to_str().unwrap()})), "invalid_request");
    assert_eq!(session.kind("browse", json!({"path": "relative/path"})), "invalid_request");
    assert_eq!(session.kind("browse", json!({"path": "~/nothing-here"})), "invalid_request");
}

#[test]
fn stat_list_and_read_use_target_names_and_stable_cursors() {
    let dir = workspace("read");
    std::fs::create_dir(dir.join("sub")).unwrap();
    for name in ["b.txt", "a.txt", "c.txt"] { std::fs::write(dir.join(name), name).unwrap(); }
    std::fs::write(dir.join("中文 #1.txt"), "你好").unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);

    let first = session.ok("list", json!({"root": root, "path": ".", "limit": 2}));
    let names: Vec<_> = first["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(names, ["a.txt", "b.txt"]);
    let second = session.ok("list", json!({"root": root, "path": ".", "limit": 10, "cursor": first["cursor"]}));
    let rest: Vec<_> = second["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(rest, ["c.txt", "sub", "中文 #1.txt"]);
    assert!(second["cursor"].is_null());
    assert!(second["entries"][0]["revision"].is_null(), "listing does not hash file contents");

    let info = session.ok("stat", json!({"root": root, "path": "a.txt"}));
    assert_eq!(info["kind"], "file");
    assert_eq!(info["size"], 5);
    assert!(info["revision"].as_str().is_some());
    assert_eq!(session.ok("stat", json!({"root": root, "path": "sub"}))["kind"], "directory");

    let read = session.ok("read", json!({"root": root, "path": "中文 #1.txt", "offset": 3, "limit": 3}));
    use base64::Engine;
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(read["data"].as_str().unwrap()).unwrap(), "好".as_bytes());
    assert_eq!(read["eof"], true);
    assert_eq!(session.kind("read", json!({"root": root, "path": "sub"})), "invalid_request");
    assert_eq!(session.kind("stat", json!({"root": root, "path": "missing"})), "not_found");
}

#[test]
fn escapes_and_external_links_are_refused() {
    let dir = workspace("scope");
    let outside = workspace("outside");
    std::fs::write(outside.join("secret"), "x").unwrap();
    std::fs::write(dir.join("inside"), "y").unwrap();
    std::os::unix::fs::symlink(outside.join("secret"), dir.join("link")).unwrap();
    std::os::unix::fs::symlink("inside", dir.join("internal")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    assert_eq!(session.kind("read", json!({"root": root, "path": "../secret"})), "permission_denied");
    assert_eq!(session.kind("read", json!({"root": root, "path": outside.join("secret")})), "permission_denied");
    assert_eq!(session.kind("read", json!({"root": root, "path": "link"})), "permission_denied");
    assert_eq!(session.ok("stat", json!({"root": root, "path": "link"}))["kind"], "symlink");
    assert_eq!(session.ok("read", json!({"root": root, "path": "internal"}))["eof"], true);
    assert_eq!(session.kind("read", json!({"root": 99, "path": "inside"})), "unavailable");
}

#[test]
fn paths_uris_and_children_stay_in_the_provider_namespace() {
    let dir = workspace("paths");
    std::fs::create_dir(dir.join("a b")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let base = dir.to_str().unwrap();
    assert!(session.ok("parent_path", json!({"root": root, "path": base}))["path"].is_null());
    assert_eq!(session.ok("parent_path", json!({"root": root, "path": "a b"}))["path"], base);
    let child = session.ok("child_path", json!({"root": root, "parent": "a b", "name": "f.txt"}));
    assert_eq!(child["path"], format!("{base}/a b/f.txt"));
    assert_eq!(session.kind("child_path", json!({"root": root, "parent": ".", "name": "x/y"})), "invalid_request");
    let uri = session.ok("resource_uri", json!({"root": root, "path": "a b"}))["uri"].as_str().unwrap().to_owned();
    assert!(uri.ends_with("/a%20b"), "{uri}");
    assert_eq!(session.ok("resolve_uri", json!({"root": root, "uri": uri}))["path"], format!("{base}/a b"));
    assert_eq!(session.kind("resolve_uri", json!({"root": root, "uri": "file:///etc/passwd"})), "permission_denied");
    let parts = session.ok("relative_path", json!({"root": root, "base": base, "path": "a b"}));
    assert_eq!(parts["components"], json!(["a b"]));
}

#[test]
fn a_duplicate_mutation_is_answered_and_payload_reuse_is_rejected() {
    let mut session = Session::start();
    let first = session.raw(5, "mkdir", json!({"root": 1, "path": "x"}));
    assert_eq!(session.raw(5, "mkdir", json!({"root": 1, "path": "x"})), first);
    assert_eq!(session.raw(5, "mkdir", json!({"root": 1, "path": "y"}))["error"]["kind"], "conflict");
    assert_eq!(session.raw(4, "mkdir", json!({"root": 1, "path": "z"}))["error"]["kind"], "unknown_outcome");
}

#[test]
fn the_connection_ends_when_the_client_closes_it() {
    let mut session = Session::start();
    session.ok("ping", json!({}));
    session.input.take();
    assert!(session.child.wait().unwrap().success());
}

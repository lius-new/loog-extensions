//! Target-side search: the scanner runs as a disposable child of the agent.
mod common;
use common::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

fn options(query: &str, buffers: &[String], spec: Value) -> Value {
    let mut base = json!({"query": query, "include": "", "exclude": "", "case_sensitive": true,
        "whole_word": false, "regex": false, "include_hidden": false, "include_ignored": false});
    for (key, value) in spec.as_object().into_iter().flatten() { base[key] = value.clone(); }
    json!({"spec": base, "encoding": {"auto_detect": true}, "buffers": buffers})
}

fn run(session: &mut Session, root: u64, dir: &Path, options: Value) -> (Vec<Value>, Value) {
    let handle = session.ok("search", json!({"root": root, "path": dir, "options": options}))["search"].clone();
    let (mut files, deadline) = (Vec::new(), Instant::now() + Duration::from_secs(20));
    loop {
        let mut page = session.ok("search_next", json!({"root": root, "search": handle, "limit": 32}));
        files.extend(page["files"].as_array_mut().unwrap().drain(..));
        if page["complete"] == true {
            session.ok("search_close", json!({"root": root, "search": handle}));
            return (files, page["progress"].clone());
        }
        assert!(Instant::now() < deadline, "the search did not complete");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn file(dir: &Path, name: &str, data: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, data).unwrap();
    path.to_str().unwrap().to_owned()
}

fn names(files: &[Value]) -> BTreeSet<String> {
    files.iter().map(|file| file["name"].as_str().unwrap().to_owned()).collect()
}

fn utf16(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() { out.extend(unit.to_le_bytes()); }
    out
}

#[test]
fn target_names_encodings_ignore_rules_and_buffer_eligibility() {
    let dir = workspace("search-names");
    file(&dir, ".gitignore", b"ignored.txt\n*.skip\n!allowed.skip\n");
    for name in ["ignored.txt", "allowed.skip", "blocked.skip", ".hidden.txt", "src/private.txt"] { file(&dir, name, b"needle"); }
    file(&dir, "src/.ignore", b"private.txt\n");
    file(&dir, "src/中文\\name.txt", "中文 needle\r\nnext".as_bytes());
    file(&dir, "utf16.txt", &utf16("needle 中文"));
    let (gbk, _, _) = encoding_rs::GBK.encode("这是一个用于验证编码识别的简体中文文件。needle 中文内容。");
    file(&dir, "gbk.txt", &gbk);
    file(&dir, "binary", b"\x00\x01needle\x00");
    let dirty = file(&dir, "dirty.txt", b"\0do not decode disk");
    file(&dir, "large.txt", &vec![b'x'; 4 * 1024 * 1024 + 1]);
    std::os::unix::fs::symlink("/etc", dir.join("outside")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);

    let (files, progress) = run(&mut session, root, &dir, options("needle", &[dirty], json!({})));
    let expected: BTreeSet<String> = ["allowed.skip", "中文\\name.txt", "utf16.txt", "gbk.txt", "dirty.txt"].map(String::from).into();
    assert_eq!(names(&files), expected);
    let literal = files.iter().find(|file| file["name"] == "中文\\name.txt").unwrap();
    assert_eq!(literal["relative"], json!(["src", "中文\\name.txt"]));
    assert_eq!(literal["matches"][0]["range"], json!({"start": 7, "end": 13}));
    assert_eq!(files.iter().find(|file| file["name"] == "dirty.txt").unwrap()["buffer"], true);
    assert_eq!(progress["skipped_binary"], 1);
    assert_eq!(progress["skipped_large"], 1);

    let (files, _) = run(&mut session, root, &dir, options("needle", &[], json!({"include": "src/*.txt", "include_ignored": true})));
    assert_eq!(names(&files), ["private.txt", "中文\\name.txt"].map(String::from).into());
}

#[test]
fn match_limits_zero_width_matches_and_handle_scope() {
    let dir = workspace("search-limits");
    file(&dir, "many.txt", "needle\n".repeat(10010).as_bytes());
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let (files, progress) = run(&mut session, root, &dir, options("needle", &[], json!({})));
    assert_eq!(files.iter().map(|file| file["matches"].as_array().unwrap().len()).sum::<usize>(), 10000);
    assert_eq!(progress["limit_hit"], true);

    file(&dir, "zero.txt", "中文\n".as_bytes());
    let (files, _) = run(&mut session, root, &dir, options("^|$", &[], json!({"regex": true, "include": "zero.txt"})));
    assert_eq!(files.iter().map(|file| file["matches"].as_array().unwrap().len()).sum::<usize>(), 3);

    let handle = session.ok("search", json!({"root": root, "path": dir, "options": options("needle", &[], json!({}))}))["search"].clone();
    let other = workspace("search-other");
    let second = mount(&mut session, &other);
    assert_eq!(session.kind("search_next", json!({"root": second, "search": handle, "limit": 1})), "permission_denied");
    session.ok("search_close", json!({"root": root, "search": handle}));
    assert_eq!(session.kind("search_next", json!({"root": root, "search": handle, "limit": 1})), "unavailable");
    assert_eq!(session.ok("search_close", json!({"root": root, "search": handle})), json!({}), "closing twice is harmless");
}

#[test]
fn a_search_outside_the_mounted_root_and_bad_page_sizes_are_refused() {
    let dir = workspace("search-scope");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    assert_eq!(session.kind("search", json!({"root": root, "path": "/etc", "options": {}})), "permission_denied");
    let handle = session.ok("search", json!({"root": root, "path": dir, "options": options("x", &[], json!({}))}))["search"].clone();
    assert_eq!(session.kind("search_next", json!({"root": root, "search": handle, "limit": 33})), "failed");
    assert_eq!(session.kind("search_next", json!({"root": root, "search": handle, "limit": 0})), "failed");
    session.ok("search_close", json!({"root": root, "search": handle}));
}

#[test]
fn the_cumulative_ignore_budget_is_visible() {
    let dir = workspace("search-ignore");
    file(&dir, ".gitignore", "ignored\n".repeat(8193).as_bytes());
    file(&dir, "visible.txt", b"needle");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let (files, progress) = run(&mut session, root, &dir, options("needle", &[], json!({})));
    assert!(files.is_empty());
    assert_eq!(progress["limit_hit"], true);
    assert!(progress["first_error"].as_str().unwrap().contains("cumulative"), "{progress}");
}

#[test]
fn a_bad_pattern_is_an_explicit_failure_not_an_empty_result() {
    let dir = workspace("search-bad");
    file(&dir, "a.txt", b"needle");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = session.ok("search", json!({"root": root, "path": dir, "options": options("(", &[], json!({"regex": true}))}))["search"].clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    let failure = loop {
        let reply = session.call("search_next", json!({"root": root, "search": handle, "limit": 1}));
        if reply.get("error").is_some() { break reply; }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(failure["error"]["kind"], "failed");
    session.ok("search_close", json!({"root": root, "search": handle}));
}

fn children(parent: u32) -> usize {
    std::fs::read_dir("/proc").unwrap().flatten().filter(|entry| entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("stat")).ok())
        .filter(|text| text.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1).map(|ppid| ppid == parent.to_string())).unwrap_or(false))
        .count()
}

fn descriptors(pid: u32) -> usize { std::fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count() }

#[test]
fn a_hundred_search_cancel_cycles_reap_children_and_descriptors() {
    let dir = workspace("search-cycles");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let pid = session.child.id();
    let baseline = descriptors(pid);
    for _ in 0..100 {
        let handle = session.ok("search", json!({"root": root, "path": dir, "options": options("needle", &[], json!({}))}))["search"].clone();
        session.ok("search_close", json!({"root": root, "search": handle}));
        let deadline = Instant::now() + Duration::from_secs(3);
        // No test-side wait: the agent itself must collect each child.
        while children(pid) > 0 {
            session.ok("ping", json!({}));
            assert!(Instant::now() < deadline, "a closed search left a child");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert_eq!(descriptors(pid), baseline);
}

#[test]
fn disconnecting_ends_every_search() {
    let dir = workspace("search-disconnect");
    for index in 0..200 { file(&dir, &format!("d{index}/f.txt"), b"needle"); }
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let pid = session.child.id();
    for _ in 0..3 { session.ok("search", json!({"root": root, "path": dir, "options": options("needle", &[], json!({}))})); }
    session.ok("search", json!({"root": root, "path": dir, "options": options("needle", &[], json!({}))}));
    assert_eq!(session.kind("search", json!({"root": root, "path": dir, "options": options("needle", &[], json!({}))})), "limit_exceeded");
    session.input.take();
    session.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(children(pid), 0);
}

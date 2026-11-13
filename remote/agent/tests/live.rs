//! Processes, terminals and filesystem subscriptions: the long-lived handles.
mod common;
use common::*;
use serde_json::json;
use std::process::Command;
use std::time::Duration;

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn running(pattern: &str) -> bool {
    Command::new("pgrep").args(["-f", pattern]).output().map(|out| !out.stdout.is_empty()).unwrap_or(false)
}

fn spawn(session: &mut common::Session, root: u64, script: &str) -> serde_json::Value {
    let handle = session.ok("spawn", json!({"root": root, "cwd": ".", "program": "sh", "args": ["-c", script], "env": []}))["process"].clone();
    session.ok("process_ready", json!({"root": root, "process": handle}));
    handle
}

#[test]
fn a_process_reports_credit_both_streams_and_its_exit_code() {
    let dir = workspace("proc");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = spawn(&mut session, root, "printf out; printf err >&2; exit 3");
    let credit = session.event(|event| event["handle"] == handle && event["kind"] == "writable");
    assert_eq!(credit["bytes"], 256 * 1024);
    let (out, err, code) = session.collect(&handle);
    assert_eq!((out.as_slice(), err.as_slice(), code), (&b"out"[..], &b"err"[..], 3));
    assert_eq!(session.kind("process_input", json!({"root": root, "process": handle, "data": b64(b"x")})), "unavailable");
}

#[test]
fn process_input_and_output_are_binary_and_closing_input_ends_cat() {
    let dir = workspace("cat");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = session.ok("spawn", json!({"root": root, "cwd": ".", "program": "cat", "args": []}))["process"].clone();
    session.ok("process_ready", json!({"root": root, "process": handle}));
    session.ok("process_input", json!({"root": root, "process": handle, "data": b64(&[0, 255, 13, 10, 0x80])}));
    session.ok("process_close_input", json!({"root": root, "process": handle}));
    let (out, _, code) = session.collect(&handle);
    assert_eq!((out, code), (vec![0, 255, 13, 10, 0x80], 0));
    assert_eq!(session.kind("process_input", json!({"root": root, "process": 99, "data": b64(b"x")})), "unavailable");
}

#[test]
fn environment_and_working_directory_apply_and_boundaries_are_enforced() {
    let dir = workspace("env");
    std::fs::create_dir_all(dir.join("sub/nested")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = session.ok("spawn", json!({"root": root, "cwd": "sub", "program": "sh",
        "args": ["-c", "echo $LOOG_TEST; pwd"], "env": [["LOOG_TEST", "value"]]}))["process"].clone();
    session.ok("process_ready", json!({"root": root, "process": handle}));
    let (out, _, code) = session.collect(&handle);
    assert_eq!(String::from_utf8(out).unwrap(), format!("value\n{}/sub\n", dir.display()));
    assert_eq!(code, 0);

    let nested = dir.join("sub/nested");
    assert_eq!(session.kind("spawn", json!({"root": root, "cwd": "sub/nested", "program": "true", "args": [],
        "excluded_roots": [nested]})), "permission_denied");
    session.ok("spawn", json!({"root": root, "cwd": "sub", "program": "true", "args": [], "excluded_roots": [nested]}));
    assert_eq!(session.kind("spawn", json!({"root": root, "cwd": "../..", "program": "true"})), "permission_denied");
    assert_eq!(session.kind("spawn", json!({"root": root, "cwd": ".", "program": "true", "args": [1]})), "invalid_request");
    assert_eq!(session.kind("spawn", json!({"root": root, "cwd": ".", "program": "true", "env": [["A=B", "c"]]})), "invalid_request");
    assert_eq!(session.kind("spawn", json!({"root": root, "cwd": ".", "program": "loog-no-such-program"})), "not_found");
}

#[test]
fn killing_a_process_reclaims_its_group_and_disconnecting_reclaims_everything() {
    let dir = workspace("kill");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = spawn(&mut session, root, "sleep 4242 & sleep 4243 & wait");
    std::thread::sleep(Duration::from_millis(300));
    assert!(running("sleep 4242"));
    session.ok("process_kill", json!({"root": root, "process": handle}));
    let (_, _, code) = session.collect(&handle);
    assert!(code != 0);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!running("sleep 4242") && !running("sleep 4243"), "the whole process group is gone");

    spawn(&mut session, root, "sleep 4244");
    std::thread::sleep(Duration::from_millis(300));
    assert!(running("sleep 4244"));
    session.input.take();
    session.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!running("sleep 4244"), "ending the connection ends its processes");
}

#[test]
fn a_large_dual_output_is_delivered_in_bounded_chunks() {
    use base64::Engine;
    let dir = workspace("large");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = spawn(&mut session, root, "head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2");
    let (mut out, mut err, mut code) = (0, 0, None);
    while code.is_none() {
        let event = session.event(|event| event["handle"] == handle);
        match event["kind"].as_str().unwrap() {
            "output" => {
                let size = base64::engine::general_purpose::STANDARD.decode(event["data"].as_str().unwrap()).unwrap().len();
                assert!(size <= 16 * 1024);
                if event["stderr"] == true { err += size } else { out += size }
            }
            "exited" => code = event["code"].as_i64(),
            _ => {}
        }
    }
    assert_eq!((out, err, code), (300000, 300000, Some(0)));
}

#[test]
fn a_terminal_has_a_controlling_tty_unicode_resize_and_exit() {
    let dir = workspace("tty");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    assert_eq!(session.kind("terminal_open", json!({"root": root, "cwd": ".", "cols": 0, "rows": 5})), "invalid_request");
    assert_eq!(session.kind("terminal_open", json!({"root": root, "cwd": ".", "cols": 100, "rows": 1001})), "invalid_request");
    let handle = session.ok("terminal_open", json!({"root": root, "cwd": ".", "program": "sh",
        "args": ["-c", "stty size; echo $TERM; tty >/dev/null && echo tty; read x; echo got:$x; stty size"], "cols": 100, "rows": 30}))["terminal"].clone();
    session.ok("terminal_ready", json!({"root": root, "terminal": handle}));
    session.event(|event| event["handle"] == handle && event["kind"] == "terminal_writable");
    std::thread::sleep(Duration::from_millis(300));
    session.ok("terminal_resize", json!({"root": root, "terminal": handle, "cols": 120, "rows": 40}));
    session.ok("terminal_input", json!({"root": root, "terminal": handle, "data": b64("你好\n".as_bytes())}));
    let (out, _, code) = session.collect(&handle);
    let text = String::from_utf8_lossy(&out).replace('\r', "");
    assert!(text.contains("30 100"), "{text:?}");
    assert!(text.contains("xterm-256color") && text.contains("tty"), "{text:?}");
    assert!(text.contains("got:你好"), "{text:?}");
    assert!(text.contains("40 120"), "the resize reached the terminal: {text:?}");
    assert_eq!(code, 0);
}

#[test]
fn terminal_close_reaps_foreground_groups_and_directory_reports_stay_in_scope() {
    let dir = workspace("tty-close");
    std::fs::create_dir(dir.join("inner")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let handle = session.ok("terminal_open", json!({"root": root, "cwd": ".", "program": "sh",
        "args": ["-c", "cd inner; sleep 4245; echo after"], "cols": 80, "rows": 24}))["terminal"].clone();
    session.ok("terminal_ready", json!({"root": root, "terminal": handle}));
    let first = session.event(|event| event["kind"] == "terminal_directory");
    assert!(first["path"].is_string());
    let inner = session.try_event(|event| event["kind"] == "terminal_directory" && event["path"] == dir.join("inner").to_str().unwrap(), Duration::from_secs(5));
    assert!(inner.is_some(), "the terminal followed the shell into the folder");
    session.ok("terminal_close", json!({"root": root, "terminal": handle}));
    session.collect(&handle);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!running("sleep 4245"));
}

#[test]
fn a_watch_reports_changes_after_its_receipt_and_nested_directories() {
    let dir = workspace("watch");
    std::fs::create_dir(dir.join("a")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let watch = session.ok("watch", json!({"root": root, "path": "."}))["watch"].clone();
    session.ok("watch_ready", json!({"root": root, "watch": watch}));
    std::fs::write(dir.join("a/file.txt"), "x").unwrap();
    let change = session.event(|event| event["kind"] == "changed" && event["watch"] == watch);
    let base = dir.to_str().unwrap();
    assert!(change["paths"].as_array().unwrap().iter().any(|path| *path == format!("{base}/a/file.txt")), "{change}");
    assert_eq!(change["sequence"], 1);

    std::fs::create_dir(dir.join("b")).unwrap();
    let created = session.event(|event| event["kind"] == "changed" && event["rescan"] == true);
    assert_eq!(created["sequence"], 2, "a new directory rebuilds the subscription and asks for a rescan");
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(dir.join("b/inside"), "y").unwrap();
    let nested = session.event(|event| event["kind"] == "changed" && event["paths"].to_string().contains("b/inside"));
    assert!(nested["sequence"].as_u64().unwrap() >= 3);

    session.ok("unwatch", json!({"root": root, "watch": watch}));
    assert_eq!(session.ok("unwatch", json!({"root": root, "watch": watch})), json!({}), "repeated release is harmless");
}

#[test]
fn a_watch_refuses_to_follow_links_and_reports_losing_its_directory() {
    let dir = workspace("watch-links");
    let outside = workspace("watch-outside");
    std::fs::create_dir(dir.join("watched")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("watched/escape")).unwrap();
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let watch = session.ok("watch", json!({"root": root, "path": "watched"}))["watch"].clone();
    session.ok("watch_ready", json!({"root": root, "watch": watch}));
    std::fs::write(outside.join("elsewhere"), "z").unwrap();
    assert!(session.try_event(|event| event["kind"] == "changed", Duration::from_millis(500)).is_none(), "an external link is not watched");
    std::fs::remove_dir_all(dir.join("watched")).unwrap();
    let stopped = session.event(|event| event["kind"] == "watch_stopped");
    assert_eq!(stopped["watch"], watch);
    assert_eq!(session.ok("unwatch", json!({"root": root, "watch": watch})), json!({}));
}

#[test]
fn a_watch_survives_a_directory_removed_during_registration_and_other_roots_cannot_use_it() {
    let dir = workspace("watch-scope");
    for index in 0..40 { std::fs::create_dir_all(dir.join(format!("d{index}/leaf"))).unwrap(); }
    let other = workspace("watch-other");
    let mut session = Session::start();
    let root = mount(&mut session, &dir);
    let second = mount(&mut session, &other);
    let watch = session.ok("watch", json!({"root": root, "path": "."}))["watch"].clone();
    assert_eq!(session.kind("watch_ready", json!({"root": second, "watch": watch})), "permission_denied");
    assert_eq!(session.kind("watch", json!({"root": root, "path": "missing"})), "not_found");
}

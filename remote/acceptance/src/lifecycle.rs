//! Opt-in repeated real component/SSH lifetimes with live owned resources.
use super::*;

fn connect(h: &mut Harness, call: u64) -> Duration {
    let began = Instant::now();
    h.tick(vec![host::Inbound::RunCommand(host::CommandContext {
        call,
        command: "loog.remote.ssh.connect".into(),
        snapshot: None,
        selections: vec![],
        selection_precondition: None,
    })]);
    let nodes = h.instance.take_ui_snapshot().unwrap();
    let button = nodes
        .iter()
        .find(|node| node.description.id == "connect")
        .expect("connection form")
        .node;
    h.tick(vec![host::Inbound::UiEvent(host::UiEvent {
        node: button,
        id: "ext.15:loog.remote.ssh.1.1.connect".into(),
        payload: host::UiEventPayload::Click,
    })]);
    h.until(|h| h.scope.is_some());
    h.tick(vec![]);
    began.elapsed()
}
fn disconnect(h: &mut Harness, call: u64) {
    h.tick(vec![host::Inbound::RunCommand(host::CommandContext {
        call,
        command: "loog.remote.ssh.disconnect".into(),
        snapshot: None,
        selections: vec![],
        selection_precondition: None,
    })]);
    h.until(|h| h.scope.is_none() && h.child.is_none());
    for _ in 0..4 {
        h.tick(vec![]);
        h.router.poll(&h.consumer, 1024);
    }
    assert_eq!(h.router.pending_count(), 0);
    assert_eq!(h.router.queued_bytes(), 0);
    assert!(h.router.ready_owners().is_empty());
    assert!(
        h.instance.poll_timers().1.is_none(),
        "heartbeat timer leaked"
    );
    h.notifications.clear();
    h.incoming = None;
    for reader in h.readers.drain(..) {
        reader.join().unwrap();
    }
}

pub(super) fn run(h: &mut Harness, cycles: usize, mut reload: impl FnMut() -> host::Instance) {
    assert!((1..=1000).contains(&cycles));
    println!("LIFECYCLE host_pid={} cycles={cycles}", std::process::id());
    let began = Instant::now();
    let folder_name = format!("lifecycle-{}", std::process::id());
    let mut directory = None;
    let mut previous_child = None::<u32>;
    let mut connect_ms = Vec::new();
    let mut stat_ms = Vec::new();
    let mut peak_agent_fds = 0;
    let mut peak_agent_rss_kib = 0;
    let mut loaded_agent_fds = 0;
    let mut loaded_agent_rss_kib = 0;
    let mut reload_ms = Vec::new();
    // One final connection verifies cleanup of the last measured lifetime.
    for iteration in 0..=cycles {
        let duration = connect(h, iteration as u64 * 2 + 1);
        if iteration < cycles {
            connect_ms.push(duration.as_millis());
        }
        let root = h.router.scope(h.scope.unwrap()).unwrap().1.clone();
        let api::Answer::Path { path } = h
            .request(api::Operation::ChildPath {
                parent: root.clone(),
                name: folder_name.clone(),
            })
            .unwrap()
        else {
            panic!("directory path")
        };
        if iteration == 0 {
            h.request(api::Operation::CreateDirectory { path: path.clone() })
                .unwrap();
            directory = Some(path.clone());
        }
        assert_eq!(directory.as_ref(), Some(&path));
        let api::Answer::Directory { entries, cursor } = h
            .request(api::Operation::List {
                path: path.clone(),
                cursor: None,
                limit: 16,
            })
            .unwrap()
        else {
            panic!("cleanup listing")
        };
        assert!(
            entries.is_empty() && cursor.is_none(),
            "staged save survived disconnect: {entries:?}"
        );
        let api::Answer::Started { handle: probe } = h.request(api::Operation::Spawn {
            program: "python3".into(), cwd: root.clone(), env: vec![], args: vec!["-c".into(),
                "import os,sys,json; p=os.getppid(); old=int(sys.argv[1]); assert not old or not os.path.exists('/proc/'+str(old)), 'child survived disconnect'; status=open('/proc/'+str(p)+'/status').read(); rss=next(int(x.split()[1]) for x in status.splitlines() if x.startswith('VmRSS:')); print(json.dumps({'fds':len(os.listdir('/proc/'+str(p)+'/fd')),'rss':rss}))".into(),
                previous_child.unwrap_or(0).to_string()],
        }).unwrap() else { panic!("probe") };
        let events = h.process_exit(probe);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, api::Notification::Exited { code: Some(0), .. }))
        );
        let output: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                api::Notification::Output {
                    bytes,
                    stderr: false,
                    ..
                } => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        let measured: serde_json::Value = serde_json::from_slice(&output).unwrap();
        peak_agent_fds = peak_agent_fds.max(measured["fds"].as_u64().unwrap());
        peak_agent_rss_kib = peak_agent_rss_kib.max(measured["rss"].as_u64().unwrap());
        if iteration == cycles {
            h.request(api::Operation::Remove {
                path,
                recursive: false,
            })
            .unwrap();
            disconnect(h, iteration as u64 * 2 + 2);
            break;
        }
        let tick = Instant::now();
        h.request(api::Operation::Stat { path: path.clone() })
            .unwrap();
        stat_ms.push(tick.elapsed().as_millis());
        h.request(api::Operation::Watch { path: path.clone() })
            .unwrap();
        let api::Answer::Path { path: file } = h
            .request(api::Operation::ChildPath {
                parent: path,
                name: "unfinished".into(),
            })
            .unwrap()
        else {
            panic!("file path")
        };
        let api::Answer::Started { handle: write } = h
            .request(api::Operation::BeginWrite {
                path: file,
                size: 64,
                expected_revision: None,
            })
            .unwrap()
        else {
            panic!("staged write")
        };
        h.request(api::Operation::AppendWrite {
            write,
            offset: 0,
            bytes: vec![1; 32],
        })
        .unwrap();
        let api::Answer::Started { handle: process } = h.request(api::Operation::Spawn {
            program: "python3".into(), cwd: root, env: vec![], args: vec!["-u".into(), "-c".into(),
                "import os,time,json; p=os.getppid(); status=open('/proc/'+str(p)+'/status').read(); rss=next(int(x.split()[1]) for x in status.splitlines() if x.startswith('VmRSS:')); print(json.dumps({'pid':os.getpid(),'fds':len(os.listdir('/proc/'+str(p)+'/fd')),'rss':rss}),flush=True); time.sleep(60)".into()],
        }).unwrap() else { panic!("child") };
        h.until(|h| {
            for event in h.router.poll(&h.consumer, 32) {
                if let api::Delivery::Notification { event, .. } = event { h.notifications.push(event); }
            }
            h.notifications.iter().any(|event| matches!(event, api::Notification::Output { handle, bytes, stderr: false } if *handle == process && bytes.contains(&b'\n')))
        });
        let output: Vec<_> = h
            .notifications
            .iter()
            .filter_map(|event| match event {
                api::Notification::Output {
                    handle,
                    bytes,
                    stderr: false,
                } if *handle == process => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        let measured: serde_json::Value = serde_json::from_slice(&output).unwrap();
        previous_child = Some(measured["pid"].as_u64().unwrap() as u32);
        loaded_agent_fds = loaded_agent_fds.max(measured["fds"].as_u64().unwrap());
        loaded_agent_rss_kib = loaded_agent_rss_kib.max(measured["rss"].as_u64().unwrap());
        disconnect(h, iteration as u64 * 2 + 2);
        h.router.retire(&h.owner);
        h.owner.generation += 1;
        let reloading = Instant::now();
        h.instance = reload();
        reload_ms.push(reloading.elapsed().as_micros());
        if (iteration + 1) % 10 == 0 {
            println!(
                "LIFECYCLE completed={} elapsed_s={:.1}",
                iteration + 1,
                began.elapsed().as_secs_f64()
            );
        }
    }
    connect_ms.sort_unstable();
    stat_ms.sort_unstable();
    reload_ms.sort_unstable();
    let p95 = |values: &[u128]| values[(values.len() * 95).div_ceil(100).saturating_sub(1)];
    println!(
        "PASS: {cycles} real WASM/SSH connect/disconnect/reload lifetimes plus cleanup verification; connection_p50_ms={} connection_p95_ms={} stat_p50_ms={} stat_p95_ms={} reload_p95_us={} agent_probe_peak_fds={peak_agent_fds} agent_probe_peak_rss_kib={peak_agent_rss_kib} agent_loaded_peak_fds={loaded_agent_fds} agent_loaded_peak_rss_kib={loaded_agent_rss_kib} elapsed_s={:.2}",
        connect_ms[cycles / 2],
        p95(&connect_ms),
        stat_ms[cycles / 2],
        p95(&stat_ms),
        p95(&reload_ms),
        began.elapsed().as_secs_f64()
    );
}

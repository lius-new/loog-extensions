use std::{env, fs, path::PathBuf};

fn main() {
    // One source builds each package; the package name says how its Agent channel starts.
    println!("cargo:rustc-check-cfg=cfg(transport, values(\"ssh\", \"wsl\", \"docker\"))");
    let package = env::var("CARGO_PKG_NAME").expect("package name");
    let transport = package.strip_prefix("loog-remote-").expect("loog-remote-<transport> package name");
    assert!(["ssh", "wsl", "docker"].contains(&transport), "unknown transport {transport}");
    println!("cargo:rustc-cfg=transport=\"{transport}\"");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut agents = String::from("pub static AGENTS: &[Agent] = &[\n");
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        let bundle = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../../remote/agent/target/bundle")
            .canonicalize().expect("build the bundled agents with remote/agent/build.mjs first");
        println!("cargo:rerun-if-changed={}", bundle.join("agents.txt").display());
        let metadata = fs::read_to_string(bundle.join("agents.txt")).expect("build the bundled agents with remote/agent/build.mjs first");
        for line in metadata.lines() {
            let mut parts = line.split(' ');
            let (machine, size, hash) = (parts.next().expect("machine"), parts.next().expect("size"), parts.next().expect("digest"));
            let size: usize = size.parse().expect("agent size");
            assert!(["x86_64", "aarch64"].contains(&machine));
            assert!(size > 0 && size <= 32 * 1024 * 1024 && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
            println!("cargo:rerun-if-changed={}", bundle.join(machine).display());
            // The agent is stored deflated and inflated piece by piece as it is sent: kept whole it
            // would sit in the component's memory (and its compiled image) for as long as the
            // package is running, though only one is ever used and only while connecting.
            let raw = fs::read(bundle.join(machine)).expect("read the bundled agent");
            assert_eq!(raw.len(), size, "agents.txt names the size of the {machine} agent");
            let packed = miniz_oxide::deflate::compress_to_vec(&raw, 10);
            assert!(miniz_oxide::inflate::decompress_to_vec(&packed).is_ok_and(|back| back == raw),
                "the {machine} agent must survive compression");
            let stored = out.join(format!("agent-{machine}.deflate"));
            fs::write(&stored, &packed).expect("write the compressed agent");
            agents.push_str(&format!("    Agent {{ machine: {machine:?}, sha256: {hash:?}, size: {size}, bytes: include_bytes!({:?}) }},\n",
                stored.to_str().expect("UTF-8 output path")));
        }
    }
    agents.push_str("];\n");
    fs::write(out.join("agents.rs"), agents).unwrap();
    println!("cargo:rerun-if-env-changed=LOOG_SOURCE");
    let root = PathBuf::from(env::var_os("LOOG_SOURCE").expect("set LOOG_SOURCE to the matching checkout"));
    let wit = root.join("wit/loog-extension");
    println!("cargo:rerun-if-changed={}", wit.display());
    fs::write(out.join("bindings.rs"),
        format!("wit_bindgen::generate!({{path: {:?}, world: \"extension-workspace\", generate_all}});",
            wit.to_str().expect("UTF-8 SDK path"))).unwrap();
}

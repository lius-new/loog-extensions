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
            agents.push_str(&format!("    Agent {{ machine: {machine:?}, sha256: {hash:?}, bytes: include_bytes!({:?}) }},\n",
                bundle.join(machine).to_str().expect("UTF-8 bundle path")));
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

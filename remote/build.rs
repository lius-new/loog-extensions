use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        let bundle = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../../remote/search/target");
        for name in ["bundle.txt", "bundle.gz"] {
            println!("cargo:rerun-if-changed={}", bundle.join(name).display());
        }
        let metadata = fs::read_to_string(bundle.join("bundle.txt"))
            .expect("build the bundled search worker with remote/search/build.mjs first");
        let mut parts = metadata.lines();
        let size: usize = parts.next().expect("worker size").parse().expect("worker size");
        let hash = parts.next().expect("worker digest");
        assert!(size > 0 && size <= 32 * 1024 * 1024 && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        println!("cargo:rustc-env=LOOG_SEARCH_SIZE={size}");
        println!("cargo:rustc-env=LOOG_SEARCH_SHA256={hash}");
    }
    println!("cargo:rerun-if-env-changed=LOOG_SOURCE");
    let root = PathBuf::from(env::var_os("LOOG_SOURCE").expect("set LOOG_SOURCE to the matching checkout"));
    let wit = root.join("wit/loog-extension");
    println!("cargo:rerun-if-changed={}", wit.display());
    fs::write(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs"),
        format!("wit_bindgen::generate!({{path: {:?}, world: \"extension-workspace\", generate_all}});",
            wit.to_str().expect("UTF-8 SDK path"))).unwrap();
}

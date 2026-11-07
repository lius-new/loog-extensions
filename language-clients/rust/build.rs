use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=LOOG_SOURCE");
    let source = PathBuf::from(
        env::var_os("LOOG_SOURCE").expect("set LOOG_SOURCE to the matching Loog checkout"),
    );
    let wit = source.join("wit/loog-extension");
    println!("cargo:rerun-if-changed={}", wit.display());
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest = crate_dir.join("../manifest.json");
    let configuration = crate_dir.join("../../../language-clients/rust/clients.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", configuration.display());
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest).expect("read plugin manifest"))
            .expect("parse plugin manifest");
    let configs: serde_json::Value =
        serde_json::from_slice(&fs::read(configuration).expect("read client configuration"))
            .expect("parse client configuration");
    let name = package["name"].as_str().expect("plugin name");
    let id = package["id"].as_str().expect("plugin id");
    let config = configs
        .get(id)
        .expect("client must have an explicit configuration");
    let list = |key: &str| {
        config[key]
            .as_array()
            .expect("string array")
            .iter()
            .map(|v| format!("{:?}", v.as_str().expect("configuration string")))
            .collect::<Vec<_>>()
            .join(",")
    };
    let generated = format!(
        "wit_bindgen::generate!({{path: {:?}, world: \"extension-ui\", generate_all}});\npub const NAME: &str = {:?};\npub const ID: &str = {:?};\npub(crate) const CONFIG: config::Configuration = config::Configuration {{languages: &[{}], program: {:?}, args: &[{}], markers: &[{}], outermost: {}, file_fallback: {}}};\n",
        wit.to_str().expect("UTF-8 WIT path"),
        name,
        id,
        list("languages"),
        config["program"].as_str().expect("program"),
        list("args"),
        list("markers"),
        config["outermost"].as_bool().expect("outermost"),
        config["fileFallback"].as_bool().expect("fileFallback"),
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs"),
        generated,
    )
    .unwrap();
}

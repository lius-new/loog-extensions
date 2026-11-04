use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=LOOM_SOURCE");
    let source = PathBuf::from(env::var_os("LOOM_SOURCE").expect("set LOOM_SOURCE"));
    let wit = source.join("wit/loom-extension");
    println!("cargo:rerun-if-changed={}", wit.display());
    let generated = format!(
        "wit_bindgen::generate!({{path: {:?}, world: \"extension-ui\", generate_all}});\npub const NAME: &str = \"Fixture\";\npub const ID: &str = \"example.language-fixture\";\npub(crate) const CONFIG: config::Configuration = config::Configuration {{languages: &[\"rust\"], program: \"${{extension}}/fake_lsp\", args: &[], markers: &[], outermost: false, file_fallback: false}};\n",
        wit.to_str().expect("UTF-8 WIT path"),
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs"),
        generated,
    )
    .unwrap();
}

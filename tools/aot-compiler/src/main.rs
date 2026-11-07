use std::{
    hash::{Hash, Hasher},
    path::Path,
};
fn sha(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 4 {
        return Err("expected WASM TARGET CODE_OUTPUT METADATA_OUTPUT".into());
    }
    // Versioned policy shared with loog-extension-host::aot::portable_engine.
    // Explicit targets use baseline CPU features, independent of the build worker.
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    config.target(&args[1])?;
    let engine = wasmtime::Engine::new(&config)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut hash);
    let wasm = std::fs::read(&args[0])?;
    let code = engine.precompile_component(&wasm)?;
    std::fs::create_dir_all(
        Path::new(&args[2])
            .parent()
            .ok_or("output requires a parent")?,
    )?;
    std::fs::write(&args[2], &code)?;
    let metadata = serde_json::json!({"profile":"loog-wasmtime-50.0.1-portable-v1","target":args[1],"compatibility":format!("{:016x}",hash.finish()),"wasm_sha256":sha(&wasm),"code_sha256":sha(&code)});
    std::fs::write(&args[3], serde_json::to_vec_pretty(&metadata)?)?;
    println!("{}", metadata);
    Ok(())
}

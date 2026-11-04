//! Encode the component metadata emitted by wit-bindgen; no WASI adapter needed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected core-module.wasm and output.wasm".into());
    }
    let module = std::fs::read(&args[0])?;
    let component = wit_component::ComponentEncoder::default()
        .module(&module)?
        .validate(true)
        .encode()?;
    std::fs::write(&args[1], component)?;
    Ok(())
}

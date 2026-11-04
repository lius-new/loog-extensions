# gopls Language Server

Extension package: 0.2.4. Upstream program: 0.24.0. Requires Loom 0.1.10 or newer.

The private Rust WebAssembly component owns the Go language client, protocol,
document synchronization and server lifecycle. It uses Loom's general
process/provider/UI interfaces; server execution follows project trust.
Its passive status block appears before the cursor-position component.

The manifest's 21 provider declarations are retained, including completion and
resolve, diagnostics, navigation, symbols, formatting, rename, hierarchies,
semantic tokens, actions and lenses. The server's negotiated capabilities
determine which are available. Configuration and initialization settings accept
JSON text. Initialization changes restart sessions; configuration changes notify
running servers. The performance setting records bounded timing summaries.

Build requirements: Rust 1.98.0 with the wasm32-unknown-unknown target, Go and Node.
Set LOOM_SOURCE to a matching Loom checkout, run npm ci, then:

```sh
node build.mjs [platform…]
LOOM_SOURCE=/path/to/loom cargo test --manifest-path rust/Cargo.toml --locked --lib
```

The Rust manifest and locked dependencies are under rust/. The private shared
source and builder are under ../../language-clients/rust/. The Rust component
encoder produces extension.wasm independently for each plugin.
No JavaScript engine or TypeScript SDK is included in the component. Unsigned development builds still use Wasmtime's local compilation/cache path.

PROVENANCE.md records server sources; licenses/ includes the Rust dependency
licenses. See rust/BENCHMARKS.md for the TS/Rust comparison and reproduction.

Official publication adds authenticated, platform-specific AOT compiled with
Wasmtime 50.0.1. An AOT-capable Loom host loads matching signed code without local
compilation. Older hosts and incompatible/missing AOT use the retained WASM.
The regular build archive is unsigned input to the trusted publication stage.

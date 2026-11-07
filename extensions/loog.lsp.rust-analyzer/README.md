# rust-analyzer Language Server

The rust-analyzer language server.

Extension package: 0.2.4. Upstream program: 2026-10-19.

## Serves

- **rust-analyzer**: `rust`

License: MIT OR Apache-2.0. Platform: every supported platform.

The extension's Rust WASM owns the language client, protocol, document
synchronization and server lifecycle. Loog supplies general process, document,
provider and UI APIs. The language client stays inside this plugin; Loog supplies the general WIT contract.
Opening a Rust document activates the extension; execution uses existing project
trust. The extension uses Loog 0.1.10's general UI tree API to place its colored
status block before the cursor-position component.

Client sources are in the repository's private `language-clients/rust/`;
this package owns its Rust manifest and lock file under `rust/`. Every package bundles its own independent
client instance. Set `LOOG_SOURCE` to the matching Loog checkout, run `npm ci`,
then `node build.mjs [platform…]`. The component has a 16 MiB build limit.
The declared `configuration` and `initialization` settings accept JSON text.

`PROVENANCE.md` records where the files came from and `licenses/` carries their licenses.

Client builds require Rust 1.98.0 and the `wasm32-unknown-unknown` target.
The installed component contains no JavaScript engine. Without compatible signed AOT, Wasmtime compiles the component on first load
and caches native code locally. The external rust-analyzer
server is unchanged.

Official publication adds authenticated, platform-specific AOT compiled with
Wasmtime 50.0.1. An AOT-capable Loog host loads matching signed code without local
compilation. Older hosts and incompatible/missing AOT use the retained WASM.
The regular build archive is unsigned input to the trusted publication stage.

# TypeScript Language Server

The TypeScript Language Server language server.

Extension package: 0.2.4. Upstream program: 6.0.2 (TypeScript 5.9.3).

## Serves

- **TypeScript Language Server**: `javascript`, `typescript`

Requires: `loom.runtime.node@^0.1.0`.

License: Apache-2.0. Platform: every supported platform.

The extension's Rust WASM implements the JavaScript/TypeScript language
client, capability negotiation, document synchronization, requests and server
lifecycle. JavaScript and TypeScript have separate acceptance fixtures; JSX/TSX
use their corresponding server language IDs. The client is in this extension
repository under `language-clients/rust/`.

Execution follows project trust. The extension draws its status color block
before the cursor-position component through Loom 0.1.10's general UI tree API.
It uses the locked TypeScript dependency bundled beside
the server when the workspace has no compatible TypeScript installation.

Set `LOOM_SOURCE` to the matching Loom checkout, run `npm ci`, then
`node build.mjs`. Private `language-clients/rust/` sources are compiled into this
package's own WASM; no shared runtime client exists. The declared `configuration`
and `initialization` settings accept JSON text.

`PROVENANCE.md` records where the files came from and `licenses/` carries their licenses.

Client builds require Rust 1.98.0 and the `wasm32-unknown-unknown` target.
The installed component contains no JavaScript engine. Without compatible signed AOT, Wasmtime compiles the component on first load
and caches native code locally. The external TypeScript
language server still runs on the declared Node runtime.

Official publication adds authenticated, platform-specific AOT compiled with
Wasmtime 50.0.1. An AOT-capable Loom host loads matching signed code without local
compilation. Older hosts and incompatible/missing AOT use the retained WASM.
The regular build archive is unsigned input to the trusted publication stage.

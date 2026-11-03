# rust-analyzer Language Server

The rust-analyzer language server.

Extension package: 0.2.2. Upstream program: 2026-10-19.

## Serves

- **rust-analyzer**: `rust`

License: MIT OR Apache-2.0. Platform: every supported platform.

The extension's TypeScript WASM owns the language client, protocol, document
synchronization and server lifecycle. Loom supplies general process, document,
provider and UI APIs. The TypeScript SDK does not contain this language client.
Opening a Rust document activates the extension; execution uses existing project
trust. The extension uses Loom 0.1.10's general UI tree API to place its colored
status block before the cursor-position component.

Client sources are in `src/main.ts` and the repository's private
`language-clients/` build sources. Every package bundles its own independent
client instance. Set `LOOM_SOURCE` to the matching Loom checkout, run `npm ci`,
then `node build.mjs [platform…]`. The component has a 16 MiB build limit.
The declared `configuration` and `initialization` settings accept JSON text.

`PROVENANCE.md` records where the files came from and `licenses/` carries their licenses.

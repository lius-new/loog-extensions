# gopls Language Server

The gopls language server.

Extension package: 0.2.0. Upstream program: 0.24.0.

## Serves

- **gopls**: `go`

License: BSD-3-Clause. Platform: every supported platform.

The extension's TypeScript WASM owns its Go language client, document synchronization
and server lifecycle. It uses Loom's general process/provider/UI APIs and creates
its own status color block. Server execution follows project trust.

Set `LOOM_SOURCE` to the matching Loom checkout, run `npm ci`, then
`node build.mjs [platform…]` with Go installed. The private `language-clients/`
sources are bundled independently into this package, and are not part of the SDK.
The declared `configuration` and `initialization` settings accept JSON text.

`PROVENANCE.md` records where the files came from and `licenses/` carries their licenses.

# TypeScript Language Server

The TypeScript Language Server language server.

Extension package: 0.2.2. Upstream program: 6.0.2 (TypeScript 5.9.3).

## Serves

- **TypeScript Language Server**: `javascript`, `typescript`

Requires: `loom.runtime.node@^0.1.0`.

License: Apache-2.0. Platform: every supported platform.

The extension's TypeScript WASM implements the JavaScript/TypeScript language
client, capability negotiation, document synchronization, requests and server
lifecycle. JavaScript and TypeScript have separate acceptance fixtures; JSX/TSX
use their corresponding server language IDs. The client is in this extension
repository, not Loom's general TypeScript SDK.

Execution follows project trust. The extension draws its status color block
before the cursor-position component through Loom 0.1.10's general UI tree API.
It uses the locked TypeScript dependency bundled beside
the server when the workspace has no compatible TypeScript installation.

Set `LOOM_SOURCE` to the matching Loom checkout, run `npm ci`, then
`node build.mjs`. Private `language-clients/` sources are compiled into this
package's own WASM; no shared runtime client exists. The declared `configuration`
and `initialization` settings accept JSON text.

`PROVENANCE.md` records where the files came from and `licenses/` carries their licenses.

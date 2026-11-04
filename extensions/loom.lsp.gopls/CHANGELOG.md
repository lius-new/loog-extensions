# Changelog

## 0.2.4

- Add release-side authenticated AOT for supported platforms, eliminating local compilation on matching AOT-capable Loom hosts.
- Retain portable WASM for older hosts and incompatible runtime profiles.

## 0.2.3

- Replace the TypeScript/JavaScript-engine client with a Rust WebAssembly component.
- Preserve typed language providers, document synchronization, settings, file watches, server recovery and status UI.
- Reduce component size and runtime memory; keep the pinned gopls server unchanged.

## 0.2.2

- Show a passive, content-sized status block using the plugin name and ready, busy, stopped or error states.
- Place the server status before the cursor-position component using the general UI tree API.
- Upgrade: Use Loom 0.1.10 or newer for relative UI placement.

## 0.2.1

- Fix workspace loading, diagnostics, completion and hover for Windows extended-length and UNC paths.
- Keep server-reported workspace errors visible in the status bar after initialization and progress updates.

## 0.2.0

- Bundle an extension-owned language client with typed providers and status bar UI.
- Use general Loom process, document and workspace APIs instead of a host server declaration.

## 0.1.3

- Changed the extension category from Languages to Language Services.

## 0.1.2

- The manifest names what the extension is for: `contributes.language_service.servers` replaces `contributes.language_servers`. The server is unchanged; this needs a Loom that reads the new name.

## 0.1.1

- The extension builds itself; nothing in what it contains changed.

## 0.1.0

- Repackaged from the pinned upstream release listed in PROVENANCE.md.

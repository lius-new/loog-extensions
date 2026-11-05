# Changelog

## 0.1.1

- Report how the service is doing to Loom (`loom:service/status`) instead of drawing a status item; Loom shows it and lists the service in its language services panel.
- Name the service in `contributes.language_service.name`.

## 0.2.4

- Add release-side authenticated AOT for supported platforms, eliminating local compilation on matching AOT-capable Loom hosts.
- Retain portable WASM for older hosts and incompatible runtime profiles.

## 0.2.3

- Replace the TypeScript client and embedded JavaScript engine with a Rust WebAssembly component.
- Preserve language providers, settings, synchronization, file watches, multi-root sessions, recovery and passive status UI.
- Retain the pinned language server payload and runtime dependencies.

## 0.2.2

- Show a passive, content-sized status block using the plugin name and ready, busy, stopped or error states.
- Place the server status before the cursor-position component using the general UI tree API.
- Upgrade: Use Loom 0.1.10 or newer for relative UI placement.

## 0.2.1

- Fix JavaScript and TypeScript workspace loading, diagnostics, completion and hover for Windows extended-length and UNC paths.
- Keep server-reported workspace errors visible in the status bar after initialization and progress updates.

## 0.2.0

- Bundle the JavaScript/TypeScript language client with typed providers and status bar UI.
- Use general Loom APIs and the locked TypeScript dependency instead of a host server declaration.

## 0.1.3

- Changed the extension category from Languages to Language Services.

## 0.1.2

- The manifest names what the extension is for: `contributes.language_service.servers` replaces `contributes.language_servers`. The server is unchanged; this needs a Loom that reads the new name.

## 0.1.1

- The extension builds itself; nothing in what it contains changed.

## 0.1.0

- Repackaged from the pinned upstream release listed in PROVENANCE.md.

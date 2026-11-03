# Changelog

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

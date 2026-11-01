# The index

`index.json` lists every published extension and, for each version, the package per
platform. It is published as an asset of the `registry` release, next to
`index.json.sig`, and replaced on every publication.

```json
{
  "schema": 1,
  "generated": "2026-11-01T00:00:00.000Z",
  "expires": "2026-12-01T00:00:00.000Z",
  "sequence": 3,
  "keyId": "index-2026-1",
  "extensions": [
    {
      "id": "loom.lang.rust",
      "name": "Rust Language",
      "publisher": "Loom",
      "description": "Syntax highlighting for Rust.",
      "categories": ["Languages"],
      "keywords": ["rust", "syntax", "tree-sitter"],
      "license": "MIT",
      "languages": [{ "name": "rust", "fileTypes": ["rs"] }],
      "versions": [
        {
          "version": "1.0.0",
          "engines": { "loom": ">=0.1.8 <0.3" },
          "capabilities": [],
          "requires": [],
          "recommends": ["loom.lsp.rust-analyzer"],
          "artifacts": [
            { "platform": "any", "url": "https://github.com/…/loom.lang.rust-1.0.0-any.loomext", "sha256": "…", "size": 123456 }
          ]
        }
      ]
    }
  ]
}
```

- `languages[].fileTypes` (and `serves` for servers) let Loom decide which extension a
  file needs without downloading anything.
- Versions are newest first. A published version never changes; to fix one, release
  a new version. `index` refuses to rebuild a version with other contents.
- A version is for `any` platform or for named ones. Loom installs the package for its
  own platform, else `any`.
- `engines.loom` is a version range; Loom picks the newest version that is compatible.
- `sequence` only grows. Loom ignores an index whose sequence is lower than one it has
  seen, so an old index cannot be put back. After `expires` Loom treats the index as
  stale and offers nothing new, while already installed extensions keep working.

## The signature

`index.json.sig` names the key and carries an Ed25519 signature of the exact bytes of
`index.json`:

```json
{ "keyId": "index-2026-1", "algorithm": "ed25519", "signature": "<base64>" }
```

Loom verifies it before parsing the index, with the public key it carries for that
`keyId`. Each package's integrity is the SHA-256 in the signed index.

## What Loom keeps

Loom stores the extensions Loom bundles in `extensions.lock` (written by `lock`) with
a snapshot of the signed index it was locked against. The snapshot is the index used
when the live one cannot be fetched.

## Publication provenance

Automatically published version entries include an optional `source` object with
`revision` (the complete extension-repository commit SHA) and `fingerprint` (SHA-256
of the extension's non-Markdown input file names and content hashes). This lets the
publisher reject changed packaging inputs under an existing version. Consumers
that do not use this publication receipt may ignore it. Historical versions need
not have it; PR/main Git comparisons protect their subsequent changes.

## Interrupted publication

The automatic publisher first uploads and verifies immutable `index-<sequence>.json`
and `index-<sequence>.json.sig` assets on the registry release, then replaces the
current `index.json` and `index.json.sig` aliases. Loom continues to read the current
aliases. If replacing them is interrupted, the publisher can verify the immutable
generation and repair the pair without rebuilding packages or reusing a sequence.
An incomplete generation reserves its sequence; the next new generation advances
beyond it. These extra assets are publication recovery data, not a new client API.

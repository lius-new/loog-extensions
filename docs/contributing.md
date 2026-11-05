# Contributing

## Application and plugin boundary

Plugins own their UI, shared keys and business/state rules. Loom supplies generic
capabilities and MUST NOT gain an adapter or named container for each plugin.
Using existing capabilities must not require changing or releasing Loom. Follow
[the shared UI contract](shared-ui.md) for contributions and lifecycle rules.

Only official extensions live here; there is no third-party submission.

## Adding or changing an extension

An extension is a project in `extensions/<id>/`. It owns its internal layout and
build tools: `build.mjs` can use Node, Go, Rust or shell commands. CI installs the
project with `npm ci` and invokes `node build.mjs`. The result, for every supported
platform, must be a package named
`<id>-<version>-<platform>.loomext` that Loom can import ([Packages](package-format.md)).
Normally an extension depends only on its own folder or pinned external packages.
The official Rust language clients intentionally compile the private common
source under `language-clients/rust` independently into each component. Their
individual Cargo manifests and lock files remain authoritative. This shared
source owns their common UI policy; it must not be moved into Loom.

1. Change the project. Pin what it downloads by hash, and commit the lock files.
2. Bump the version in the extension's manifest. A published version never changes: if
   the files in its folder changed (Markdown aside) and the version did not, publishing
   refuses it.
3. Build the packages as the project does, and try them:
   `node scripts/loom-ext.mjs check <packages>` and `dev <packages> --to <dir>`.
4. Add a line to the extension's `CHANGELOG.md`.
5. The build matrix discovers new extension folders automatically. Each project must
   support `npm ci` followed by `node build.mjs`, producing packages in its own
   `dist/` folder. Add any extra toolchain setup to the matrix in
   `.github/workflows/extensions.yml`. Build jobs have read-only repository access
   and never receive the signing key.

## Naming

`loom.lang.<name>`, `loom.lsp.<name>`, `loom.runtime.<name>`, `loom.theme.<name>`,
`loom.queries.<name>`. Lowercase letters, digits and hyphens.

## Releasing

Merge the versioned changes to `main`; the build matrix builds the affected projects
and the shared publisher delivers their completed packages in one batch. See
[Automatic releases](releases.md) for immutable versions, validation and interrupted-run
recovery.

The publisher produces the bundle lock and index snapshot as artifacts. Copy them
into Loom's repository before its next application release.

# Contributing

Only official extensions live here; there is no third-party submission.

## Adding or changing an extension

An extension is a project in `extensions/<id>/`. Nothing about its layout or tools is
prescribed: it can build with Node, Go, Rust, a shell script or by hand. What it must
do is produce, for every platform it supports, a package named
`<id>-<version>-<platform>.loomext` that Loom can import ([Packages](package-format.md)).
It depends on nothing else in this repository; code shared between extensions is not
allowed, only what is published elsewhere (npm, PyPI, crates.io, …), pinned and locked
in the extension's own folder.

1. Change the project. Pin what it downloads by hash, and commit the lock files.
2. Bump the version in the extension's manifest. A published version never changes: if
   the files in its folder changed (Markdown aside) and the version did not, publishing
   refuses it.
3. Build the packages as the project does, and try them:
   `node scripts/loom-ext.mjs check <packages>` and `dev <packages> --to <dir>`.
4. Add a line to the extension's `CHANGELOG.md`.
5. A new extension also needs its workflow, `.github/workflows/ext-<id>.yml`: copy
   another one and change what it builds with. It must build the packages, upload them
   as an artifact, and call `publish.yml` on `main`; it must not see the signing key.

## Naming

`loom.lang.<name>`, `loom.lsp.<name>`, `loom.runtime.<name>`, `loom.theme.<name>`,
`loom.queries.<name>`. Lowercase letters, digits and hyphens.

## Releasing

Merge the versioned changes to `main`; the extension's workflow builds it and the
shared publisher delivers it. See [Automatic releases](releases.md) for immutable
versions, validation and interrupted-run recovery.

The publisher produces the bundle lock and index snapshot as artifacts. Copy them
into Loom's repository before its next application release.

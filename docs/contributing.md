# Contributing

Only official extensions live here; there is no third-party submission.

## Adding or changing an extension

1. A hand-written extension (theme, snippets, queries): edit its folder. A built one
   (language, server, runtime): edit its `recipe.toml`.
2. For a server or runtime, run `node scripts/loom-ext.mjs pin <id>` after changing a
   version, and commit `pins.toml` and `npm/`.
3. Bump `version` in the manifest or recipe. A published version never changes; the
   index refuses to rebuild one with other contents.
4. Run `npm test` and `node scripts/loom-ext.mjs check`.
5. `node scripts/loom-ext.mjs build <id>` and `validate <id>` if you have the
   toolchain and the validator.
6. Add a line to the extension's `CHANGELOG.md` (generated ones do it from the recipe).

## Naming

`loom.lang.<name>`, `loom.lsp.<name>`, `loom.runtime.<name>`, `loom.theme.<name>`,
`loom.queries.<name>`. Lowercase letters, digits and hyphens.

## Releasing

The Release workflow builds the named extensions, publishes them as GitHub releases
and replaces the signed index. It needs the `LOOM_INDEX_KEY` secret (make one with
`loom-ext keygen`, and give Loom the printed public key). After a release, run
`loom-ext lock` and copy `extensions.lock` and the index snapshot into Loom's
repository so the next Loom build bundles and falls back to the same versions.

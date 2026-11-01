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

Merge the explicitly versioned changes to `main`; the Release workflow automatically
delivers pending versions after its checks pass. See [Automatic releases](releases.md)
for immutable versions, production validation and interrupted-run recovery.

The workflow produces the bundle lock and index snapshot as artifacts. Copy them
into Loom's repository before its next application release.

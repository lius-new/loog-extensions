# Automatic releases

Extension versions are written explicitly in `recipe.toml` (`extension.version`)
for built extensions, or `manifest.json` (`version`) for handwritten ones. Merge
changes to `main`; the Release workflow tests the tools, checks every definition,
compares the checkout to the signed registry, and delivers pending versions.
There is no release form to fill in for normal publication.

## Version policy

- A new id or a version newer than all its published versions is pending.
- Published versions are immutable and are not rebuilt. Changes to an extension's
  packaging inputs require a new extension version. PR and main checks compare
  Git changes; new index entries also record the input fingerprint and commit.
- Markdown documentation alone does not request a new package. Shared builder or
  toolchain changes apply to future versions and do not republish existing ones.
- Server/runtime `pack.version` pins the upstream program. `extension.version`
  names the extension package and may advance independently for a packaging fix;
  changing it alone does not change the downloaded program or its pins.
- Removed definitions do not remove previously published versions from the index.

Each run scans all current pending versions, including an interrupted earlier
batch. Multiple queued pushes may coalesce; the current version of each extension
in the checkout is the delivery target. Do not merge several unpublished versions
of the same extension if every intermediate version needs to be released.

## Delivery

1. Fetch and verify the existing registry. Network errors, missing signatures and
   unknown keys fail the run; none is treated as an empty registry.
2. Check the signing key and prepare a production validator. There is no
   structure-only fallback in automatic publication.
3. Build each pending extension for all platforms its recipe declares, validate
   every staged package and its language samples, and pack it. Failures are
   recorded independently so other extensions can proceed. Staging and expanded
   upstream archives are removed after each extension to bound runner disk use.
4. Publish complete extension releases in dependency order. Draft assets can be
   repaired; public assets are never overwritten. Every asset's size and GitHub
   SHA-256 are checked before and after publication. Tags point to the tested
   checkout commit.
5. Fetch the latest verified registry again, preserve its existing versions and
   add successful versions whose dependencies are available. Upload a signed,
   immutable `index-<sequence>.json` generation first and verify it; then replace
   the current index/signature pair and verify those published bytes too.
6. Produce the bundle lock/snapshot and a delivery report. A batch with failed or
   deferred extensions finishes as failed even when independent ones succeeded.

Publication is serialized with `cancel-in-progress: false`. The workflow checks
and builds the same source SHA, and signing secrets are only exposed to the
preflight and publication steps. Cached downloads and compiled grammar modules
remain hash-verified by their existing builders.

## Recovery and preview

Rerun the failed workflow to finish delivery. If an extension release became public
before its index update finished, the original packages are downloaded, verified
and validated; they are not recompiled or repacked. If the index already contains
the verified version, it is not rewritten and its sequence is not incremented.
If an index/signature upload was interrupted, the newest verified immutable
generation restores the current pair. An incomplete generation is not overwritten;
its sequence is reserved and the next new index uses a higher sequence.

**Actions → Release → Run workflow** remains available for maintenance. Its default
`preview=true` only lists pending versions. Turning preview off performs the same
strict delivery as a main push. Publication only runs from `main`.

The run summary and `extension-delivery-<run-id>` artifact contain the plan,
prepared results, final report, packages, signed index and bundle lock. A green
preview means the plan is valid; it is not proof that packages were published.

The commands used by CI can also be run locally:

```sh
node scripts/loom-ext.mjs release plan
node scripts/loom-ext.mjs release preflight   # requires LOOM_INDEX_KEY
LOOM_EXT_VALIDATE=/absolute/path/loom-ext-validate node scripts/loom-ext.mjs release prepare
node scripts/loom-ext.mjs release publish     # requires gh authentication and LOOM_INDEX_KEY
```

`release plan` reads the registry and writes `.work/release/plan.json` without
publishing. `release prepare` builds and validates without publishing. These files
are tied to the checkout SHA and the extension input fingerprints.

## Enabling the workflow

The repository must have its existing `LOOM_INDEX_KEY` Actions secret; its public
key must match `loom-ext.toml`'s `index.keys`. The key stays separate from Loom's
application signing key.

Prefer a published Linux x64 validator pinned by URL and SHA-256 in
`tools/validator.lock`. Until one exists, CI builds the exact production source
commit in `tools/validator-source.toml` with its locked dependencies. **That commit
must be available in the Loom repository before this workflow is merged to main.**
The currently pinned commit is the locally tested `61088d608b3fd3e7c77fc0f861d660cdeafce251`,
available on Loom's `codex/extension-validator-source` branch. The workflow uses
the immutable commit SHA. Do not replace it with an arbitrary moving branch or
silently bypass validation. Once validator
binaries are published, update `tools/validator.lock`; the binary takes precedence.

The first automatic run picks up all 46 currently pending definitions. This is a
preview count, not a claim that all recipes have already built successfully.
The workflow emits the lock as an artifact; refreshing the copy in Loom's repository
for an application release remains a separate task.

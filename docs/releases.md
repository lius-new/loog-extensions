# Automatic releases

An extension is released when its own workflow, `.github/workflows/ext-<id>.yml`,
runs on `main`: the workflow builds the extension's packages however the extension
builds them, uploads them as an artifact, and calls the shared `publish.yml`. There is
no release form to fill in for normal publication.

## Version policy

All current extension packages start at **0.1.0**. Package versions are independent
of both Loom and the bundled upstream program. Use patch releases for fixes and
minor releases for additions or incompatible pre-1.0 changes. Each extension moves
to 1.0.0 when its behavior and integration are stable; there is no shared release
number. Runtime requirements refer to the extension package, for example
`loom.runtime.node@^0.1.0`, not Node's upstream major version.

- A version newer than all of an extension's published versions is new.
- Published versions are immutable. When the extension's files changed since a version
  was published (Markdown documentation excepted) and its version did not, publishing
  refuses it: bump the version. What counts as the extension's files is what git
  tracks in its folder, so ignored folders such as `node_modules` or build output
  never do. Index entries record that fingerprint and the commit.
- The version in the manifest names the extension package and advances independently of
  the upstream program it repackages.
- Removed extensions do not remove previously published versions from the index.

## Delivery

1. The extension's workflow builds the packages and uploads them. It never sees the
   signing key and cannot publish anything itself.
2. `publish.yml` checks that the signing key is the one Loom trusts, prepares the
   production validator (there is no structure-only fallback), and runs
   `release intake`.
3. Intake unpacks every package and checks it: manifest, files, name, one version for
   all platforms, a version newer than the published ones, the extension unchanged
   since a published version, and Loom's own loaders importing it. One extension's
   failure does not stop others.
4. `release publish` creates each extension's release as a draft, uploads the
   packages, checks every asset's size and GitHub SHA-256, publishes it, then checks
   again. Creation returns the stable release ID; later steps use it instead of
   rediscovering the draft through tags or lists. Tags point to the tested commit.
5. It then fetches the latest verified registry again, keeps its existing versions and
   adds the successful ones whose dependencies are available. A signed, immutable
   `index-<sequence>.json` generation is uploaded and verified first; then the current
   index/signature pair is replaced and verified.
6. The bundle lock and a delivery report are produced. A batch with failed or deferred
   extensions finishes as failed even when independent ones succeeded.

Publication is serialized (`concurrency: extension-publication`), so there is one
writer of the index at a time whichever extensions are releasing. GitHub keeps only the
newest waiting run of such a group and cancels the older waiting ones, even without
`cancel-in-progress`; many extensions releasing together therefore show some publish
jobs as canceled. Nothing is lost: each publish run first collects the packages of
every extension build of the same commit that has finished, and publishes all of them
(extensions already published are skipped). The last run waiting always sees every
build that finished before it started, and a build that finishes later asks for
another run.

## Recovery

Rerun the failed workflow. If a release became public before the index update
finished, intake takes the packages the release was published with, not the ones this
run built, provided the extension's files are those of the released commit; nothing is
rebuilt or re-uploaded. If the index already lists the version it is left alone and its
sequence does not grow. If an index/signature upload was interrupted, the newest
verified generation restores the current pair, and an incomplete generation is never
overwritten.

The run's artifact `delivery-<artifact>-<run-id>` contains the plan, results, final
report, packages, signed index and bundle lock.

The commands can also be run locally:

```sh
node scripts/loom-ext.mjs release intake path/to/packages --dry-run   # checks only
node scripts/loom-ext.mjs release preflight    # requires LOOM_INDEX_KEY
node scripts/loom-ext.mjs release intake path/to/packages   # records what to publish
node scripts/loom-ext.mjs release publish      # requires gh authentication and LOOM_INDEX_KEY
```

## Enabling the workflows

The repository must have its `LOOM_INDEX_KEY` Actions secret; its public key must match
`loom-ext.toml`'s `index.keys`. The key stays separate from Loom's application signing
key. Extension workflows pass it to `publish.yml` with `secrets: inherit` and never
mention it themselves.

Prefer a published Linux x64 validator pinned by URL and SHA-256 in
`tools/validator.lock`. Until one exists, CI builds the exact production source commit
in `tools/validator-source.toml` with its locked dependencies. The commit must stay
reachable in the Loom repository (it is on `main`); the workflow uses the immutable
commit SHA. Do not replace it with an arbitrary moving branch or silently bypass
validation. Once validator binaries are published, update `tools/validator.lock`; the
binary takes precedence.

The workflow emits the lock as an artifact; refreshing the copy in Loom's repository
for an application release remains a separate task.

## Early-stage baseline

All packages started over at 0.1.0 (index sequence 8 is the first generation of the
new baseline). That one-time reset is finished and its tooling removed: published
releases and their tags are immutable from here on, and no workflow input deletes them.

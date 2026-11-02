# Automatic releases

`.github/workflows/extensions.yml` handles each push to `main` as one build batch:
it selects the affected extensions, builds their packages in parallel, then calls
the shared `publish.yml` once after every selected build has finished. Pull requests
build the same way but never publish. Each extension still owns its build script
and dependencies; there is no release form for normal publication.

Selection uses the complete git diff: pushes compare the previous tip to the current
commit, and pull requests compare their merge base to the tested commit. Changes
under `extensions/<id>/` select that extension. Changes to workflows, scripts,
validator tools, root package files or `loom-ext.toml` select all extensions. Other
repository documentation does not start extension builds. An initial push or a
force push whose previous tip is unavailable conservatively builds everything.

"All" means all active recipes. IDs in `loom-ext.toml`'s `deferred` list are
excluded from automatic and manual builds. Their historical sources and existing
published assets remain intact until component migration. Rust, Go and JS/TS are
the first component clients; other pure server declarations cannot run in the new
Loom host. New language-service index versions retain their component ABI.

For manual recovery, dispatch **Extensions** on `main`. Its optional `extensions`
input accepts comma-separated IDs; leave it empty to build all extensions. Dispatches
on other branches only build. Rerunning failed jobs reuses the successful build
artifacts from the same run and retries the failed builds and publisher.

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

1. Each selected extension builds its packages in an independent matrix job and
   uploads a `packages-<id>` artifact. Builds never receive the signing key. A failed
   build does not cancel the others; after the matrix finishes, the publisher can
   deliver successful builds while the overall workflow remains failed.
   The publisher downloads packages from this workflow run only, so commits and PR
   artifacts cannot be mixed into another batch. If no build produced packages,
   publication fails before any release or index is written.
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
writer of the index at a time across all commits. `cancel-in-progress: false` keeps
the active publisher running, and `queue: max` keeps up to 100 batches waiting
instead of replacing the older waiting batch. The limit now counts batches, not
extensions. If the queue fills, GitHub cancels additional batches; rerun those
workflows when capacity is available. Each batch downloads and validates its packages
once, including compatible runtimes and their dependents together. Already published
versions are skipped; failed or deferred deliveries remain visible in the report.

## Recovery

Rerun the failed workflow. If a release became public before the index update
finished, intake takes the packages the release was published with, not the ones this
run built, provided the extension's files are those of the released commit; nothing is
rebuilt or re-uploaded. If the index already lists the version it is left alone and its
sequence does not grow. If an index/signature upload was interrupted, the newest
verified generation restores the current pair, and an incomplete generation is never
overwritten.

The run's artifact `delivery-<run-id>-<attempt>` contains the plan, results, final
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
key. Only the batch's publish job passes `LOOM_INDEX_KEY` explicitly to `publish.yml`;
the selection and build jobs have read-only repository access and no signing secret.

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

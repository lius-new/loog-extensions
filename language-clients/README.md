# Official language clients

These private build sources are compiled into each of the Rust, Go and
JavaScript/TypeScript extension components. Each installed extension owns its
RPC connection, documents, capability registrations, processes and status block.
The general Loom SDK contains none of this language-specific behavior.

Set `LOOM_SOURCE` to the matching Loom source checkout. Builders use its WIT,
SDK and locked component toolchain. CI obtains those from `lius-new/loom` at
`20722d73bc3f068ca60f4a2f608b9542d54c261f`; the production validator uses the same
source revision. The host contract is committed and pushed before these clients.
No SDK is
downloaded or installed into the user's editor at runtime.

Each extension also contributes metadata-only language identities and file
associations. Syntax extensions remain independent and retain their own parser
bindings. Language metadata follows Loom's existing restart requirement when
installed or changed.

`configuration` and `initialization` settings accept JSON text. Initialization
changes restart sessions; configuration changes are delivered to live sessions.
The optional boolean `performance` setting logs bounded 100-sample RPC, JSON decode,
result-conversion and SDK submission summaries, containing no document text. It takes effect
on the next activation.

Local checks:

```sh
node --experimental-transform-types --test language-clients/client.test.ts language-clients/protocol.test.ts
node --experimental-transform-types --test --test-concurrency=1 language-clients/acceptance.test.ts
node language-clients/stage.mjs
```

`stage.mjs` prepares local acceptance artifacts under `.work/packages`. It uses
the built pinned gopls (falling back to the available local binary), pinned
downloaded rust-analyzer and locked TypeScript
payload. Release packaging uses each extension's `build.mjs` and pinned server
toolchain; the local staging command is not a release or an installation.

The ignored native Loom acceptance uses these packages through the real WASM
executor and native provider consumers. `LOOM_MIGRATION_BYTES=1048576` or
`8388608` selects exact Unicode document sizes. The optional
`LOOM_MIGRATION_LANGUAGE` filter and `LOOM_MIGRATION_TRACE` are diagnostic aids;
formal comparison runs all languages with tracing disabled. Failed requests must
not be omitted from performance results.

Completion transport conservatively checks the native ASCII smart-case
subsequence pattern before converting candidates. Unicode candidates and patterns
remain with the native matcher. Filtered lists are incomplete and keep original
resolve indices; broadening the pattern requests the full applicable list again.

Windows verification of this source: 29 client unit tests, four real-server
functional/two-root cases, and all twelve native WASM performance cohorts passed.
Loom's post-removal suite passed 1700 tests with 28 ignores. Its retained
`docs/architecture/language-migration-evidence` has paired old-host/component
reports: 100 samples per operation, three document sizes and all four languages;
all 36 p95 comparisons passed the unchanged budget. Components are below 16 MiB.
The three final package builds also passed the updated production importer.
Manual desktop QA and Linux/macOS execution are not claimed by these Windows runs.

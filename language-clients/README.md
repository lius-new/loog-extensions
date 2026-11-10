# Official language clients

The official clients compile private Rust sources from `rust/` into
independent WASM components. Each extension has its own Cargo manifest and lock
file; `rust/clients.json` retains its language IDs, program, arguments and root
selection policy. JavaScript/TypeScript includes JSX/TSX. Each installed extension owns its
RPC connection, documents, capability registrations, processes and private UI contributions.

The Python (Pyright), YAML and Bash clients run their pinned Node server through
`loog.runtime.node` and require Loog 0.2.0 or newer. Pyright uses the nearest
Python project marker. YAML and Bash use the open workspace folder. The Bash
client sends the LSP language ID `shellscript` for Loog's `bash` documents.

clangd serves C, C++ and Objective-C, Lua Language Server serves Lua, and Taplo
serves TOML. Marksman serves Markdown, Ruff serves Python, ZLS serves Zig, and
Biome serves JavaScript, TypeScript, JSON and CSS.

The CSS and SCSS, HTML, JSON and JSON with Comments, and ESLint clients run
separate servers from the locked `vscode-langservers-extracted` package through
`loog.runtime.node`. The JSON client retains the original bundle ID. ESLint
loads the workspace’s ESLint installation and project configuration.

PHPantom and Neocmakelsp bundle native servers for PHP and CMake. The Svelte
and Dockerfile clients run their pinned servers through `loog.runtime.node`.

Roslyn serves C# projects with an installed .NET SDK. The Postgres Language
Server handles PostgreSQL SQL. The GraphQL client uses GraphQL Config and runs
its pinned server through `loog.runtime.node`.

Eclipse JDT LS 1.63.0 serves Java with an installed JDK 25 or newer. Buf provides
Protobuf support through its pinned native CLI.
The 0.2.4 clients require Loog 0.1.10 and its independent `loog:ui/tree@0.1.0`
interface. Each client chooses `statusbar.cursor-position` as a logical target
and inserts its status before that component; there is no language-specific
host layout branch. Missing targets keep the indicator hidden until it returns.
The workflow's pinned Loog SDK source has been updated
to a committed revision containing that interface; builders reject older SDKs.
The general Loog SDK contains none of this language-specific behavior.

The development clients contribute one shared status entry, with generic
`Language Services` text and no independent colored background or server name.
They provide the shared key, document contexts, priorities, tree descriptions
and state-driven motion; Loog only supplies general UI capabilities. States from
other documents/projects cannot override the active document. Busy icons rotate
continuously; ready, error and stopped icons stay static. Progress does not
restart motion, republish membership or display changing percentages.
The entry is passive, with no new click, hover or details UI. Detailed failures
remain in plugin logs. See [plugin-owned shared UI](../docs/shared-ui.md) for the
required application/plugin boundary and lifecycle contract.

Set `LOOG_SOURCE` to the matching Loog source checkout. Builders use its WIT
and locked Rust component toolchain. CI obtains those from the single Loog commit
pinned in `tools/loog-source.toml`, which the AOT verification and the validator
use as well; move it forward when Loog changes its extension interface. Release
builds intentionally
reject older SDKs rather than silently producing per-server status blocks.
No versions are bumped by local staging. No SDK is downloaded or installed into
the user's editor at runtime.

Each extension also contributes metadata-only language identities and file
associations. Syntax extensions remain independent and retain their own parser
bindings. Language metadata follows Loog's existing restart requirement when
installed or changed.

`configuration` and `initialization` settings accept JSON text. Initialization
changes restart sessions; configuration changes are delivered to live sessions.
The optional boolean `performance` setting logs bounded 100-sample RPC, JSON decode,
result-conversion and SDK submission summaries, containing no document text. It takes effect
on the next activation.

Rust checks (run for each extension ID):

```sh
LOOG_SOURCE=/path/to/loog cargo test --manifest-path extensions/loog.lsp.rust-analyzer/rust/Cargo.toml --locked --lib
```

The previous TypeScript client sources and SDK have been removed. Release
builders, local staging and the native UI test fixture all use the Rust client.
The shared componentizer also uses Rust and requires no JavaScript engine SDK.

Build the native UI fixture and prepare local staging:

```sh
node language-clients/fixtures/native-ui/build.mjs
node language-clients/stage.mjs
```

`stage.mjs` prepares local acceptance artifacts under `.work/packages`. It uses
versioned `<id>/<version>` folders and updates `extensions.json`, preserving
other installed packages and disabled choices while pinning the staged versions.
Loog loads the versions named in that index, not flat folders next to it.
Set `LOOG_EXTENSIONS_DIR` to the absolute `.work/packages` path and launch Loog
from the same terminal so the test process inherits the override. The script uses
the built pinned gopls (falling back to the available local binary), pinned
downloaded rust-analyzer and locked TypeScript
payload. Release packaging uses each extension's `build.mjs` and pinned server
toolchain; the local staging command is not a release or an installation.

The ignored native Loog acceptance uses these packages through the real WASM
executor and native provider consumers. `LOOG_MIGRATION_BYTES=1048576` or
`8388608` selects exact Unicode document sizes. The optional
`LOOG_MIGRATION_LANGUAGE` filter and `LOOG_MIGRATION_TRACE` are diagnostic aids;
formal comparison runs all languages with tracing disabled. Failed requests must
not be omitted from performance results.

Completion transport conservatively checks the native ASCII smart-case
subsequence pattern before converting candidates. Unicode candidates and patterns
remain with the native matcher. Filtered lists are incomplete and keep original
resolve indices; broadening the pattern requests the full applicable list again.

Historical Windows verification of the 0.2.2 TypeScript implementation: 29 client unit tests, four real-server
functional/two-root cases, and all twelve native WASM performance cohorts passed.
Loog's post-removal suite passed 1700 tests with 28 ignores. Its retained
`docs/architecture/language-migration-evidence` has paired old-host/component
reports: 100 samples per operation, three document sizes and all four languages;
all 36 p95 comparisons passed the unchanged budget. Components are below 16 MiB.
The three final package builds also passed the updated production importer.
Manual desktop QA and Linux/macOS execution are not claimed by these Windows runs.

The Rust migration comparison is documented in `rust/BENCHMARKS.md`. It uses
identical server payloads, the native Loog WASM executor, isolated cold caches,
and three repeats of 100 requests per operation for small, 1 MiB and 8 MiB
Unicode documents. Host memory excludes separate language-server processes.
Migrating the TypeScript client does not remove its Node server dependency.

## Authenticated release AOT

Version 0.2.4 keeps these Rust WASM clients and adds authenticated native AOT in
the trusted publication stage. A matching AOT-capable Loog host skips local
compilation. Old hosts and incompatible profiles retain the portable WASM path.
See `rust/AOT.md` for the authority boundary, native CI gate and measured costs.


### Local status-motion preview

The unreleased clients use `loog:ui/motion@0.1.0`. CI's WIT snapshot includes
that interface; publication compiler and validator pins remain independent.
A future motion-enabled release must align those publication tools with the
motion-capable host. Current local staging keeps package versions unchanged and
does not publish or replace registry artifacts.

`node language-clients/stage.mjs` prepares the three updated WASM clients and
their existing servers under `language-clients/.work/packages`. Launch the
motion-capable Loog with `LOOG_EXTENSIONS_DIR` pointing there, and open Go, Rust
and TypeScript files in a trusted workspace. Startup/indexing rotates the icon;
ready, error and stopped states stay static. The four plugin-owned Material
Design Icons use the matching host's generic filled-path icon capability.

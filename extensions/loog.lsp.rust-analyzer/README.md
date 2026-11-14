# rust-analyzer Language Server

The rust-analyzer language server.

Extension package: 0.2.4. Upstream program: 2026-10-19.

## Serves

- **rust-analyzer**: `rust`

License: MIT OR Apache-2.0. Platform: every supported platform.

The extension's Rust WASM owns the language client, protocol, document
synchronization and server lifecycle. Loog supplies general process, document,
provider and UI APIs. The language client stays inside this plugin; Loog supplies the general WIT contract.
Opening a Rust document activates the extension; execution uses existing project
trust. The extension uses Loog 0.1.10's general UI tree API to place its colored
status block before the cursor-position component.

Client sources are in the repository's private `language-clients/rust/`;
this package owns its Rust manifest and lock file under `rust/`. Every package bundles its own independent
client instance. Set `LOOG_SOURCE` to the matching Loog checkout, run `npm ci`,
then `node build.mjs [platform…]`. The component has a 16 MiB build limit.
The declared `configuration` and `initialization` settings accept JSON text.

The development build can synchronize provided Rust buffers through the new
workspace interface. `environmentProgram` defaults to `@bundled`: the plugin
prepares its pinned Linux server in a private target cache, checks its digest,
then starts it. Set a program name or path **on the target environment** to use
your own installation. `environmentArguments` is a JSON array encoded as a
setting string (default `[]`). The local bundled executable is never used as a
remote path. The provider root must be granted execution trust. This work requires
the matching development Loog host and is not a published compatibility promise.

Managed preparation needs target Python 3, HTTPS access to the pinned upstream
release, and at least 576 MiB free staging space. User-only `environmentPython`
defaults to `python3`; `environmentCache` defaults to the target user's cache
directory. A private lock serializes installations. Completed tools are verified
before reuse; damaged caches fail visibly instead of being overwritten. At most
four versions or interrupted installs and 1 GiB are admitted per plugin cache.
Normal cancellation cleans its own stage; a hard kill may leave a bounded stage.
Choose a fresh cache directory if a damaged or full cache prevents startup.
Linux x86_64 has passed real SSH acceptance; Linux aarch64 pins are available but
not yet end-to-end verified. Project toolchains such as Cargo/rustc/rustfmt remain
the project's responsibility; they are not installed by this preparation step.

The resource adapter discovers project markers through scoped provider operations
and uses one session per scope/project, up to four processes. Parent traversal is
limited to 64 directories and 32 configured marker names. Buffer edits reuse the
project selection; reopening a file or reloading settings discovers it again.
It asynchronously reads `.loog/extension-settings.json` from that selected
project. Trusted project values override user values for project-scoped keys;
user-only keys keep their user values. No setting is inherited from a native
project. Program selection, initialization, configuration notifications and
server configuration requests share the resolved session values. User-setting
and resource-root changes restart discovery and the affected resource adapter;
project-file edits currently require closing/reopening the project documents or
reloading the language service. Reads are limited to 256 KiB and caches to 16
scope/project pairs and 1 MiB total.
It reads at most 4 MiB per document and retains at most 16 MiB of text.
Servers advertising diagnostic pulls use a bounded request tied to the document
lifecycle and revision. Other servers must push the exact server document version;
stale or unversioned findings are discarded. Native hover,
completion, completion resolve and insertion have passed real SSH acceptance.
Completion results are capped at 256 items, 32 KiB per source item and a
conservative 512 KiB response budget; opaque resolve data is bound to its process
and complete document revision. Server commands are not executed by completion.
Automatic subproject selection and reopening have passed real SSH acceptance
with a nested rust-project.json project. A cold session must finish document
synchronization before interactive requests can reliably meet their deadlines.
Unopened definition preview, navigation, references and history have passed
combined native language/file/terminal acceptance over real SSH. URI resolution
can try other authorized readable roots in the same environment, bounded to 128
roots and 512 attempts. That fallback has state-machine tests; cross-root and
standard-library navigation still need end-to-end coverage.
Formatting uses path-free snapshot edits (1024 edits and 512 KiB replacement
text). Real rustfmt formatting, undo, format-before-save and service restart
have passed the combined SSH test. The save test uses ten seconds of scheduling
slack, while production retains its one-second step deadline. Resource-specific
on-type capability configuration and native on-type formatting/undo have passed
the isolated protocol fixture test. Resource service reports carry
complete identities and are coalesced until acknowledgement. Readiness, status
isolation, service restart, startup failure and recovery after correcting the
program setting have passed combined real SSH acceptance without opening a window.
Resource renaming has passed combined real SSH acceptance, including preparation,
an unopened second file, unchanged disk contents and grouped undo/redo. Proposals
are text-only, at most 16 files/1024 edits/512 KiB of replacement text. Duplicate
resources, stale versions, cross-environment changes, file operations and annotated
edits requiring confirmation are rejected as a whole.

The development adapter also handles server `workspace/applyEdit`. Up to four
requests can await URI mapping or host completion. It responds `applied: true`
only on the host's actual buffer-commit receipt, never on submission alone. Server
replacement cancels outstanding operations; accepted mutations are never replayed.
The receipt chain passed combined SSH acceptance using an isolated protocol fixture
which checks two-file success, invalid-range failure, unchanged disk and grouped
undo. This is distinct from the real rust-analyzer rename test.

The adapter now publishes negotiated provider options for each process's exact
resource contexts. Mode filters, completion triggers/resolve and rename preparation
no longer share native server configuration. Resource on-type trigger publication
has passed end-to-end verification with the protocol fixture.
Existing local language features keep their established interface.

The resource adapter watches project markers and project extension settings
through scoped provider paths. A change rechecks the selected project and restarts
services in that scope. Other environments retain their sessions. Monitoring is
bounded to sixteen roots and 4,096 dependency paths/1 MiB per component; sequence
gaps require a rescan, and stopped subscriptions report an error until a root reload.
The shared adapter's live configuration path has passed real JSON/Node acceptance.

Resource code actions use bounded lazy offers (64 items, 24 KiB per original
item, 512 KiB estimated total). The selected action resolves its URIs through
authorized resource scopes; unsupported file operations or annotations reject
the entire proposal. Commands require an advertised name and a matching process,
scope and document lifecycle, and wait for outstanding host edit receipts.
Native action resolution, invalid-proposal rollback, command-failure reporting,
command edit receipts, and save actions involving unopened resources have passed
combined SSH acceptance with the protocol fixture. Saving persists the requested
document while other edited buffers remain unsaved;
completion-triggered server commands remain disabled in this adapter.

`PROVENANCE.md` records where the files came from and `licenses/` carries their licenses.

Client builds require Rust 1.98.0 and the `wasm32-unknown-unknown` target.
The installed component contains no JavaScript engine. Without compatible signed AOT, Wasmtime compiles the component on first load
and caches native code locally. The external rust-analyzer
server is unchanged.

Official publication adds authenticated, platform-specific AOT compiled with
Wasmtime 50.0.1. An AOT-capable Loog host loads matching signed code without local
compilation. Older hosts and incompatible/missing AOT use the retained WASM.
The regular build archive is unsigned input to the trusted publication stage.

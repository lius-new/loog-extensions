# Private workspace transport for language clients

This crate is plugin-owned and contains no host, SSH or platform implementation.
`Processes` submits generic workspace commands and consumes replies on later
dispatches. Its `Process` identities are separate from local host processes and
provider handles. Notifications are matched by scope and provider handle.

Input is bounded to 256 KiB per process and sent in 16 KiB chunks under provider
credits. Keep unaccepted bytes in the language client's bounded outbox and resume
on `Writable`. `close_input` follows acknowledged input; `kill` also handles a
spawn which succeeds after cancellation. Call `drive` once per guest dispatch
to retry control submissions rejected before acceptance. Failed or unknown input
operations are never replayed. Up to four processes can be retained, including
ones waiting for cleanup after failure.

`Frames` decodes Content-Length messages from raw output. Message budgets are
explicit (at most 32 MiB), header size is 8 KiB, an input chunk is at most 64 KiB,
and at most 256 complete messages are returned per chunk. Malformed framing
fails closed. UTF-8 is checked only after a complete message is available.

Run `cargo test --offline --locked --manifest-path language-clients/rust/workspace/Cargo.toml`
from the plugin repository. The twenty-seven tests cover flow control, cancellation,
scope identity, unknown outcomes, teardown, framing boundaries and version-fixed
document discovery. `Documents` retains metadata for up to 128 matching buffers,
reads one buffer at a time in 64 KiB chunks (at most 4 MiB per buffer), and drops
the entire partial read on revision changes. URI conversion is requested from
the matching environment's provider. Revoked execution grants close affected
language documents while preserving ordinary editor access. Optional project
discovery walks at most 64 provider-defined parents, probes at most 32 marker
names, and selects nearest/outermost markers or a configured fallback. No native
path parsing is used. Discovered projects are reused across buffer edits.

`Documents::with_settings` reads the selected project's
`.loog/extension-settings.json` through provider-defined child paths and 64 KiB
chunks, at most 256 KiB with a consistent revision. Missing files mean no project
overrides; denied, malformed or mixed-revision reads fail that load. The host's
`ResolveSettings` validates declarations and applies defaults/user/project values
for the exact scope, without native-project settings. Cache keys include scope
and opaque project path; shared immutable values are bounded to 16 projects and
1 MiB overall. Edits reuse them, closing the last project document releases them,
and root/user-setting changes or explicit reload discard them and in-flight reads.
Project-file edits while a document remains open do not yet trigger a reload;
close/reopen, reload the language service or change its user settings to re-read.

`LocationUris` resolves server URIs through authorized read scopes, preserving the
environment in every result. It retains at most 32 jobs and 256 URIs per job;
pending URI text is capped at 128 KiB per job and 512 KiB overall, and expanded
resource results at 192 KiB per job. Duplicate URIs share a lookup. Cancellation
also handles a late submission receipt, and never publishes a partial result.
The originating scope is preferred; up to 128 readable roots in the same
environment can be tried, with at most 512 total attempts per job. Only denied,
absent or unavailable results advance to another root; malformed URI errors do
not. A dependency root does not need code execution permission. Multi-root
behavior has state-machine coverage; the SSH acceptance currently uses one root.

`remote/acceptance` exercises this crate with the real SSH provider component.
With `LOOG_REMOTE_LANGUAGE_PROGRAM` pointing to the test environment's verified
rust-analyzer, it additionally checks a ProjectJson Rust project: initialization,
hover, definition, completion, versioned syntax diagnostics and shutdown.
The consumer library is compiled into that test harness. The official Rust
language component also opts into this crate using its `environment` feature;
its adapter is `../src/provided.rs`. Native diagnostics, hover and completion acceptance uses
`LOOG_REMOTE_LANGUAGE_PACKAGE` and `LOOG_REMOTE_LANGUAGE_PROGRAM` with Loog's
ignored `native_resource_editor_real_provider` test. Subproject discovery and
reopening, unopened definition preview, navigation, references and history also
pass that real SSH test. With `LOOG_REMOTE_FORMATTER` naming an installed target
rustfmt, the combined test covers native formatting, undo and format-before-save,
alongside all file/terminal checks. Service restart restores fresh diagnostics.
The save acceptance allows ten seconds of scheduling slack; production retains
its one-second step deadline. Resource service status isolation, startup failure
and recovery also pass combined SSH acceptance. The JSON client also uses this adapter.

Resource rename now also passes the combined real SSH test: prepare/rename,
unopened-target reads, unchanged disk contents and group undo/redo. The plugin's
`provided_rename.rs` validates text-only server proposals before URI mapping;
`provided_edits.rs` bounds server applyEdit operations and waits for host receipts.
`LOOG_REMOTE_EDIT_SERVER` may name the isolated executable
`remote/fixtures/edit-server.py` on the target for the optional native receipt
test. That fixture requests two buffer edits and then an invalid range, and
publishes a marker only after receiving both outcomes. It never modifies project
files itself and is distinct from the real rust-analyzer acceptance.

The fixture also covers resource on-type formatting, lazy code-action resolution,
two-file edits, a following command whose reply precedes its edit receipt,
invalid-proposal rollback without command execution, explicit command failures,
and save actions reading an unopened resource. These passed the combined native
SSH test in 202.74 seconds. Saving writes only the requested document; other
affected buffers keep their unsaved changes. No foreground window is involved.

Project settings subsequently passed the complete SSH test in 215.34 seconds:
target initialization overrides user settings and native-project values remain
isolated; the protocol fixture verifies initialization, configuration notification
and configuration request values. `provided_program.rs` now reads literal target
program/argument settings, refusing native placeholders (64 arguments/32 KiB).
The JSON component opts into the same adapter. `provided_diagnostics.rs` adds
one pending pull per process, a coalescing 128-document queue, revision checks and
a 30-second deadline. Node/JSON/JSONC acceptance passed in 118.88 seconds using
explicit target paths. Plugin-owned automatic preparation of pinned Linux
rust-analyzer and Node/npm resources subsequently passed the combined language,
file and terminal test in 278.69 seconds. Preparation runs isolated Python in the
target process, without consuming language stdin or producing protocol stdout;
its private implementation is `../environment_tools.py`. Eight offline installer
tests cover hashing, concurrent installation, damaged caches, traversal, expansion,
cache budgets and failure cleanup. Managed startup allows 300 seconds (the target
installer has a 270-second deadline); existing explicit-program startup stays at
30 seconds. No package manager scripts or target project toolchain are installed.

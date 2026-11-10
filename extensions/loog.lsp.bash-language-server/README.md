# Bash Language Server

Bash language services for shell scripts and startup files.

The extension runs its own Loog WASM client and the pinned server (5.8.1)
through `loog.runtime.node`. Open a Bash file in a trusted workspace to
start the service. The client uses the workspace folder as its project root.

The `configuration` and `initialization` extension settings accept JSON text.
Loog enables the features the server advertises for the open file.
Linting uses ShellCheck when it is available on `PATH`.

`PROVENANCE.md` records the locked npm dependencies and `licenses/` contains
the Rust client and dependency notices.

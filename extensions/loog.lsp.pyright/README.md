# Pyright

Python language services powered by Pyright.

The extension runs its own Loog WASM client and the pinned server (1.1.414)
through `loog.runtime.node`. Open a Python file in a trusted workspace to
start the service. The client looks for the nearest Python project configuration, then uses the workspace folder.

The `configuration` and `initialization` extension settings accept JSON text.
Loog enables the features the server advertises for the open file.

`PROVENANCE.md` records the locked npm dependencies and `licenses/` contains
the Rust client and dependency notices.

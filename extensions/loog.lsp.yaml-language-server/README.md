# YAML Language Server

YAML validation, completion, navigation and formatting.

The extension runs its own Loog WASM client and the pinned server (2.0.0)
through `loog.runtime.node`. Open a YAML file in a trusted workspace to
start the service. The client uses the workspace folder as its project root.

The `configuration` and `initialization` extension settings accept JSON text.
Loog enables the features the server advertises for the open file.

`PROVENANCE.md` records the locked npm dependencies and `licenses/` contains
the Rust client and dependency notices.

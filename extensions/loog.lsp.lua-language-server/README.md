# Lua Language Server

Lua completion, diagnostics and navigation.

Open a supported file in a trusted workspace to start the Loog WASM client.
The client finds the nearest project marker (`.luarc.json`, `.luarc.jsonc`) and falls back to the workspace folder.

The package contains a pinned native server for its platform.

The `configuration` and `initialization` extension settings accept JSON text.

See the [upstream server documentation](https://luals.github.io/).

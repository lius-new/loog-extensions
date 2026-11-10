# HTML Language Server

HTML completion, document symbols and formatting.

Open a supported file in a trusted workspace to start the Loog WASM client.
The client finds the nearest project marker (`package.json`) and falls back to the workspace folder.

The server runs through the `loog.runtime.node` extension.

The `configuration` and `initialization` extension settings accept JSON text.

See the [upstream server documentation](https://github.com/hrsh7th/vscode-langservers-extracted).

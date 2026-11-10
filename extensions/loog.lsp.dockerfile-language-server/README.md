# Dockerfile Language Server

Dockerfile and Containerfile completion, diagnostics and formatting.

Open a supported file in a trusted workspace to start the Loog WASM client.
The server runs through `loog.runtime.node`.

Open a `Dockerfile` or `Containerfile` in the workspace. Docker Engine is
optional for these editing features.

The `configuration` and `initialization` extension settings accept JSON text.

See the [upstream server documentation](https://github.com/rcjsuen/dockerfile-language-server).

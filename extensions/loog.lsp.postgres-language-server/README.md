# Postgres Language Server

PostgreSQL SQL syntax diagnostics, migration linting and formatting.

Open a supported file in a trusted workspace to start the Loog WASM client.
The package includes the pinned language server for its platform.

This server parses PostgreSQL SQL. Use a `postgres-language-server.jsonc`
file to configure linting or database-backed features. Enable the upstream
preview formatter in that file with `{"format":{"enabled":true}}`.

See the [upstream server documentation](https://pg-language-server.com/0.28.2/).

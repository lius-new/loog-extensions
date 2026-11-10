# ESLint Language Server

ESLint diagnostics and quick fixes for JavaScript and TypeScript.

Open a supported file in a trusted workspace to start the Loog WASM client.
The server runs through `loog.runtime.node`.

Install ESLint in the workspace and add the project’s ESLint configuration.
For TypeScript, configure the project’s TypeScript parser and rules in ESLint.

The `configuration` and `initialization` extension settings accept JSON text. When editing
`configuration`, keep the existing keys and change their values as needed.

See the [upstream server documentation](https://github.com/microsoft/vscode-eslint).

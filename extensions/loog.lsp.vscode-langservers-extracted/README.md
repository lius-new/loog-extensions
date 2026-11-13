# JSON Language Server

JSON and JSON with Comments completion, validation and formatting.

Open a supported file in a trusted workspace to start the Loog WASM client.
The server runs through `loog.runtime.node`.

This extension keeps the previous bundle ID for updates. CSS and SCSS, HTML,
and ESLint are available as separate extensions.

The `configuration` and `initialization` extension settings accept JSON text. Keep
`json.validate.enable` set to `true` when adding schema settings.

The development component also supports provided JSON documents through the
matching development Loog workspace interface. The provider must grant execution
for the project. `environmentProgram` defaults to `@bundled`: the plugin prepares
the pinned Node runtime and locked server packages for the target platform,
checks SHA-256/SHA-512 digests and starts the server. It never runs npm lifecycle
scripts. `environmentArguments` is a JSON array encoded as a setting string
(default `["--stdio"]`). To select your own target Node runtime,
set the program to that executable and the arguments to
`["/target/server/bin/vscode-json-language-server", "--stdio"]`. Native package
and runtime placeholders are refused in these settings. The host does not select
or install Node or the server.

Preparation requires Linux, Python 3 and HTTPS access to nodejs.org and the locked
npm package URLs. User-only `environmentPython` defaults to `python3` and
`environmentCache` defaults to the target user's cache directory. Installation
is serialized, completed cache files are verified before reuse, and damaged
caches are not overwritten. Limits are 128 MiB downloads, 512 MiB extracted
files, four cache entries and 1 GiB per plugin cache; staging needs 576 MiB free.
Choose a fresh cache directory if an interrupted or damaged cache blocks startup.
Linux x86_64 automatic preparation has passed real SSH acceptance; Linux aarch64
pins are available but have not yet passed end-to-end acceptance.

The plugin discovers `package.json` using provider paths, reads the selected
project's `.loog/extension-settings.json`, and resolves settings for that exact
scope. Native-project settings are not inherited. A session retains at most one
diagnostic pull and 128 queued documents, coalescing edits in queue order. The
plugin watches target project markers and configuration through provider paths.
Changes restart only services in the affected scope; deleting a project setting
restores its user/default value. Creating and deleting configuration passed real
SSH acceptance with native diagnostics and completion while preserving unsaved
text and undo. Monitoring has a sixteen-root and 4,096-path/1 MiB budget; sequence
gaps trigger a rescan. A stopped subscription reports an error and a root reload
retries it.
Replies
must still match the requested document lifecycle and revision; unversioned push
diagnostics are not relabeled as current. A diagnostic request times out after
30 seconds and ends that server session. Resource rename and code actions share
the generic resource edit adapter. Real WASM/SSH acceptance covers diagnostics,
schema completion, formatting/undo/save and JSONC comments with trailing-comma
warnings. The combined native/Node/file/terminal managed-tool test passed in
278.69 seconds, without opening or controlling a desktop window.

See the [upstream server documentation](https://github.com/hrsh7th/vscode-langservers-extracted).

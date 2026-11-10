# clangd

C and C++ language services powered by clangd.

Open a supported file in a trusted workspace to start the Loog WASM client.
The client finds the nearest project marker (`compile_commands.json`, `compile_flags.txt`, `.clangd`) and falls back to the workspace folder.

The package contains a pinned native server for its platform.

Give clangd a compilation database (`compile_commands.json`) or `compile_flags.txt`
so it can resolve project includes and compiler options.

The `configuration` and `initialization` extension settings accept JSON text.

See the [upstream server documentation](https://clangd.llvm.org/).

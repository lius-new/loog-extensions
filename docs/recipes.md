# Recipes

An extension that is not just files has a `recipe.toml` in its folder. The kind says
what is built; the manifest, README, changelog and icon are generated from the recipe.

## Language

```toml
schema = 1
kind = "language"

[extension]
name = "Rust"
version = "1.0.0"
description = "Syntax highlighting for Rust."
# recommends = ["loom.lsp.rust-analyzer"]

[sources.rust]                    # every source is pinned by commit and archive hash
repository = "https://github.com/tree-sitter/tree-sitter-rust"
revision = "77a3747266f4d621d0757825e6b11edcbf991ca5"
sha256 = "dee82ddfd01bfc3a8ed201cc03b56448107d3217a4b3a7a8fc7fa6bc32b2405b"
licenses = [ "LICENSE" ]

[grammars.rust]
source = "rust"
path = "."                        # the grammar inside the source
file-types = [ "rs" ]
sample = "samples/rust.txt"       # must be colored: loom-ext validate

[[grammars.rust.highlights]]
path = "queries/highlights.scm"
[[grammars.rust.injections]]
path = "queries/injections.scm"
```

Other options: `generate = true` (run `tree-sitter generate` first), `prebuilt = "x.wasm"`
(use a module shipped in the source), `optimize = false` with `support` C files (the WASI
driver, without Binaryen), `capture-map` (rename captures the application knows
differently), `patches` (query patches, relative to the extension folder), and
`directory` sources for files kept in the extension folder (under `local/`).

Grammars are compiled with the toolchain pinned in `tools/toolchain.toml`: the
Tree-sitter CLI, the WASI SDK and Binaryen, downloaded once and checked against their
hashes. The toolchain is fetched only when a grammar actually has to be compiled; a
module that was built before is reused when none of its inputs changed.

The extension's license is read from the license files of the sources that went in and
written as an SPDX expression; `NOASSERTION` when one is not recognised.

## Language server and runtime

```toml
schema = 1
kind = "server"                   # or "runtime"

[extension]
name = "rust-analyzer"
version = "2026.10.19"
description = "The rust-analyzer language server."
license = "MIT OR Apache-2.0"
# requires = ["loom.runtime.node@^22"]

[pack]
kind = "archive"                  # archive | npm | go
version = "2026-10-19"            # the extension version, with dates written 2026.10.19
upstream = { source = "github", repository = "rust-lang/rust-analyzer", tag = "2026-10-19" }

[pack.platforms.linux-x86_64]
asset = "rust-analyzer-x86_64-unknown-linux-gnu.gz"
format = "gz"                     # zip | tar.gz | tar.xz | gz | raw
executables = ["rust-analyzer"]

[servers.rust-analyzer]
name = "rust-analyzer"
command = "${extension}/rust-analyzer"
root = { markers = ["Cargo.toml"], strategy = "outermost", fallback = "file_directory" }
languages = [{ language = "rust", language_id = "rust" }]
```

- `archive`: an upstream release asset per platform, repackaged unchanged (`include`
  picks files). `pins.toml` holds each archive's URL, SHA-256 and size.
- `npm`: exact package versions in `pack.packages` and the lockfile in `npm/`; installed
  with `npm ci --ignore-scripts`, once, for `any` platform.
- `go`: a module at a fixed version with a fixed Go toolchain, compiled for each target.

`loom-ext pin <id>` writes `pins.toml` (and the npm lockfile) after a version changes:
from the publisher's own checksum when it gives one, otherwise by hashing the download.
Nothing is downloaded at build time that is not pinned there.

# Packages

A package is a `.loomext` file: a gzip-compressed tar archive with one root folder
named after the extension id, holding `manifest.json` and the extension's files.
File names are `<id>-<version>-<platform>.loomext`.

- Only files and folders; no links, devices or duplicate paths; at most 16,384 entries.
- At most 256 MiB compressed and 512 MiB of expanded file data. These limits
  match the current Loom importer; large servers keep their required type stubs.
- Entries are sorted and carry fixed times, so packing the same files gives the same
  bytes and the same SHA-256.
- Grammars are WebAssembly only. A native library is refused in language, theme and
  queries extensions; Loom would run it without a sandbox.

## The manifest

`manifest.json` follows the rules Loom applies when it loads an extension, plus what a
published one must have (`loom-ext check` reports each problem):

| Field | Notes |
| --- | --- |
| `schema` | `1` |
| `id` | `vendor.name…`, lowercase; the folder is named after it |
| `version` | `major.minor.patch` |
| `name`, `publisher`, `description`, `license` | One line each (60, 60, 200, 100 characters); `license` is an SPDX expression |
| `categories` | At most three of Themes, Languages, Snippets, Linters, Formatters, Editing, Source control, Productivity, Other |
| `repository` | An `https://` address |
| `icon`, `readme`, `changelog` | A PNG or SVG (up to 256 KiB), and two Markdown files (up to 512 KiB) |
| `engines.loom` | A version range, such as `>=0.1.8 <0.3` |
| `requires` | `["loom.runtime.node@^22"]`: extensions that must be installed with this one |
| `recommends` | Extension ids to suggest; a language recommends only language servers |
| `capabilities` | A language server declares `process.execute-local` |
| `executables` | Paths the application marks executable after unpacking |
| `contributes` | `themes`, `languages`, `snippets`, `queries`, `language_servers` |

Names must follow the scheme: `loom.lang.*` contributes a language, `loom.lsp.*`
declares a language server, `loom.theme.*` a theme, `loom.queries.*` queries,
`loom.runtime.*` a runtime.

### Languages

`contributes.languages` points at a `tree-sitter.json`: the standard Tree-sitter
description, with each grammar's WebAssembly module (`tree-sitter-<name>.wasm`) and its
`highlights` and `injections` queries inside the package.

### Language servers

```json
{
  "id": "loom.lsp.rust-analyzer.rust-analyzer",
  "name": "rust-analyzer",
  "languages": ["rust"],
  "command": "${extension}/rust-analyzer",
  "args": [],
  "root_markers": ["Cargo.toml", "rust-project.json"],
  "root_strategy": "outermost",
  "root_fallback": "file_directory",
  "priority": 10,
  "bindings": [{ "language": "rust", "language_id": "rust" }],
  "initialization_options": {}
}
```

A command starts with `${extension}/` (a file in this package) or `${runtime.<name>}/`
(a file in the `loom.runtime.<name>` extension this one `requires`). It never names a
program found on `PATH`. On Windows `.exe` is appended to a command without an
extension. `bindings` map Loom's languages (and variants) to the language ids the
server expects. `root_strategy`, `root_fallback`, `priority` and `bindings` go beyond
what Loom reads today; they are part of the contract Loom is being extended to.

## Platforms

A package for a server or runtime holds that platform's program. Languages, themes and
queries are the same everywhere and are built once, as `any`. A version is either `any`
or a set of named platforms, never both.

# Packages

A package is a `.loogext` file: a gzip-compressed tar archive with one root folder
named after the extension id, holding `manifest.json` and the extension's files.
File names are `<id>-<version>-<platform>.loogext`.

- Only files and folders; no links, devices or duplicate paths; at most 16,384 entries.
- At most 256 MiB compressed and 512 MiB of expanded file data. These limits
  match the current Loog importer; large servers keep their required type stubs.
- Entries are sorted and carry fixed times, so packing the same files gives the same
  bytes and the same SHA-256.
- Grammars are WebAssembly only. A native library is refused in language, theme and
  queries extensions; Loog would run it without a sandbox.

## The manifest

`manifest.json` follows the rules Loog applies when it loads an extension, plus what a
published one must have (`loog-ext check` reports each problem):

| Field | Notes |
| --- | --- |
| `schema` | `1` |
| `id` | `vendor.name…`, lowercase; the folder is named after it |
| `version` | `major.minor.patch` |
| `name`, `publisher`, `description`, `license` | One line each (60, 60, 200, 100 characters); `license` is an SPDX expression |
| `categories` | At most three of Themes, Syntax Highlighting, Language Services, Snippets, Linters, Formatters, Editing, Source control, Productivity, Other |
| `repository` | An `https://` address |
| `icon`, `readme`, `changelog` | A PNG or SVG (up to 256 KiB), and two Markdown files (up to 512 KiB) |
| `engines.loog` | A version range, such as `>=0.1.8 <0.3` |
| `requires` | `["loog.runtime.node@^0.1.0"]`: extensions that must be installed with this one |
| `recommends` | Extension ids to suggest; a highlight extension recommends only language service extensions |
| `capabilities` | A language service server declares `process.execute-local` |
| `executables` | Paths the application marks executable after unpacking |
| `contributes` | `highlight`, `highlight_queries`, `language_service`, `themes`, `snippets`, `commands`, `settings` |

Categories describe a specific function. Use `Syntax Highlighting` for grammar
packs and highlight queries, and `Language Services` for completion, hover,
navigation and language server integrations. Use `Linters`, `Formatters` and
`Snippets` for those functions. An extension can list multiple categories when
it provides multiple functions. `Languages` is no longer accepted.

Each contribution point is named by what it is for, and Loog refuses a manifest with
any other name (`languages` and `language_servers` are not accepted):

| Point | What it is for |
| --- | --- |
| `highlight` | Syntax colouring: grammar packs |
| `highlight_queries` | Extra colouring rules for a grammar |
| `language_service` | Typed `providers` implemented by the extension component |
| `themes` | Colour themes |

Names must follow the scheme: `loog.lang.*` contributes highlight, `loog.lsp.*`
declares language service providers, `loog.theme.*` a theme, `loog.queries.*` highlight
queries, `loog.runtime.*` a runtime. The index says what an extension is for by the
field it has (`highlight`, `language_service`), never by its id.

### Highlight

`contributes.highlight` points at a `tree-sitter.json`: the standard Tree-sitter
description, with each grammar's WebAssembly module (`tree-sitter-<name>.wasm`) and its
`highlights` and `injections` queries inside the package.

### Language service components

`contributes.language_service.name` is what the service is called wherever Loog lists
it, such as `Rust Analyzer`: one line of 1 to 40 characters. The package `name` says what is
installed; this says what answers for a language. The index carries it as
`language_service.name`.

`contributes.language_service.providers` lists typed providers:

```json
{
  "id": "loog.lsp.rust-analyzer.completion",
  "kind": "completion",
  "selector": { "languages": ["rust"] }
}
```

The manifest also declares `api: "loog:extension@0.1.0"`, its `component`,
activation events and capabilities. Private client code owns process launch,
root selection, protocol language IDs and initialization. Programs are package
executables or come from declared runtime dependencies; launch still requires
workspace trust. General SDK APIs wrap Loog services and contain no language client.

All current language service recipes use typed providers. The retired `servers`
declaration is rejected by the host. Each manifest declares its minimum Loog
version.

## Platforms

A package for a server or runtime holds that platform's program. Highlight, themes and
highlight queries are the same everywhere and are built once, as `any`. A version is either `any`
or a set of named platforms, never both.

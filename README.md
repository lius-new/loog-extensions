# loog-extensions

The official extensions for [Loog](https://github.com/lius-new/loog): each one is its
own project, and the tools here take in the packages they build, check that Loog can
import them, sign and publish them. Loog itself ships almost none of this; it fetches
a signed index from here and installs what a person asks for, and what a file needs.

| Kind | Id | What it is |
| --- | --- | --- |
| Highlight | `loog.lang.<name>` | Syntax highlighting: a Tree-sitter grammar compiled to WebAssembly, its queries and file types |
| Language service | `loog.lsp.<name>` | An extension component with typed language providers, its own private client and the server payload |
| Runtime | `loog.runtime.<name>` | An interpreter other extensions run on, such as Node.js |
| Theme | `loog.theme.<name>` | Color themes |
| Queries | `loog.queries.<name>` | Extra highlight rules for a grammar that another extension owns |

Highlighting and the language service are separate extensions, so each can be chosen,
updated and disabled on its own. A highlight extension names the language service
extensions it works well with in `recommends`.

Every language service recipe under `extensions/loog.lsp.*` uses a Rust WASM
client. See [language-clients](language-clients/README.md) for the shared source
and staging checks. Each manifest declares its minimum Loog version.

## What an extension is here

A folder under `extensions/` that builds a package. How is up to the extension: its
own language, tools and dependencies, nothing shared with the others. What counts is
the result, a `.loogext` package that Loog can import. See
[Packages](docs/package-format.md) and [Contributing](docs/contributing.md).

## Using the tools

```sh
npm ci --ignore-scripts          # a few small dependencies for the tools
node scripts/loog-ext.mjs help   # the commands
node scripts/loog-ext.mjs check path/to/packages   # would publishing accept them?
```

Try an extension in a local Loog without publishing anything:

```sh
npm run try -- loog.theme.aura                   # starts the installed `loog`
npm run try -- loog.lsp.gopls --source <loog>    # builds against, and `cargo run`s, that checkout
```

Set `LOOG_SOURCE` to your Loog checkout once and `--source` is no longer needed.
`try` builds for this machine, unpacks into `.work/dev-extensions` and starts Loog
with `LOOG_EXTENSIONS_DIR` set: `--run` or `LOOG_EXT_RUN` if given, else `cargo run` in
the checkout, else `loog`. The same by hand:

```sh
cd extensions/loog.theme.aura && npm ci && node build.mjs
node scripts/loog-ext.mjs dev extensions/loog.theme.aura/dist --to ~/loog-extensions-dev
LOOG_EXTENSIONS_DIR=~/loog-extensions-dev loog
```

## Automatic releases

`.github/workflows/extensions.yml` selects the affected extensions and builds each
project in parallel. Once the whole batch has finished on `main`, it calls
`publish.yml` once to check the completed packages, publish the releases and update
the signed index. See [Automatic releases](docs/releases.md).

## Layout

```text
extensions/<id>/        one project per extension; builds its packages however it likes
.github/workflows/      batch extension builds, the shared publisher, and tool tests
tools/validator.lock    where to download loog-ext-validate
loog-ext.toml           the repository, the extensions Loog bundles, and index settings
scripts/                the loog-ext command line (Node, no build step) and its tests
docs/                   package format, index format, the tools, releases, contributing
```

## More

- [The tools](docs/tools.md)
- [Packages](docs/package-format.md) and [the index](docs/index-format.md)
- [Automatic releases](docs/releases.md)
- [Contributing](docs/contributing.md)

Licensed under MIT or Apache-2.0, at your option. The grammars, queries and programs
inside a built package keep their own licenses; each package carries them.

# loom-extensions

The official extensions for [Loom](https://github.com/lius-new/loom): each one is its
own project, and the tools here take in the packages they build, check that Loom can
import them, sign and publish them. Loom itself ships almost none of this; it fetches
a signed index from here and installs what a person asks for, and what a file needs.

| Kind | Id | What it is |
| --- | --- | --- |
| Highlight | `loom.lang.<name>` | Syntax highlighting: a Tree-sitter grammar compiled to WebAssembly, its queries and file types |
| Language service | `loom.lsp.<name>` | What answers hints and diagnostics: a language server program, per platform, with the languages it serves |
| Runtime | `loom.runtime.<name>` | An interpreter other extensions run on, such as Node.js |
| Theme | `loom.theme.<name>` | Color themes |
| Queries | `loom.queries.<name>` | Extra highlight rules for a grammar that another extension owns |

Highlighting and the language service are separate extensions, so each can be chosen,
updated and disabled on its own. A highlight extension names the language service
extensions it works well with in `recommends`.

## What an extension is here

A folder under `extensions/` that builds a package. How is up to the extension: its
own language, tools and dependencies, nothing shared with the others. What counts is
the result, a `.loomext` package that Loom can import. See
[Packages](docs/package-format.md) and [Contributing](docs/contributing.md).

## Using the tools

```sh
npm ci --ignore-scripts          # a few small dependencies for the tools
node scripts/loom-ext.mjs help   # the commands
node scripts/loom-ext.mjs check path/to/packages   # would publishing accept them?
```

Try an extension in a local Loom without publishing anything:

```sh
cd extensions/loom.theme.aura && npm ci && node build.mjs
node scripts/loom-ext.mjs dev extensions/loom.theme.aura/dist --to ~/loom-extensions-dev
LOOM_EXTENSIONS_DIR=~/loom-extensions-dev loom
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
tools/validator.lock    where to download loom-ext-validate
loom-ext.toml           the repository, the extensions Loom bundles, and index settings
scripts/                the loom-ext command line (Node, no build step) and its tests
docs/                   package format, index format, the tools, releases, contributing
```

## More

- [The tools](docs/tools.md)
- [Packages](docs/package-format.md) and [the index](docs/index-format.md)
- [Automatic releases](docs/releases.md)
- [Contributing](docs/contributing.md)

Licensed under MIT or Apache-2.0, at your option. The grammars, queries and programs
inside a built package keep their own licenses; each package carries them.

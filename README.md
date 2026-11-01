# loom-extensions

The official extensions for [Loom](https://github.com/lius-new/loom): their sources,
the recipes that build them, and the tools that check, package, sign and publish
them. Loom itself ships almost none of this; it fetches a signed index from here and
installs what a person asks for, and what a file needs.

| Kind | Id | What it is |
| --- | --- | --- |
| Language | `loom.lang.<name>` | A Tree-sitter grammar compiled to WebAssembly, its queries and file types |
| Language server | `loom.lsp.<name>` | A language server program, per platform, with the languages it serves |
| Runtime | `loom.runtime.<name>` | An interpreter other extensions run on, such as Node.js |
| Theme | `loom.theme.<name>` | Color themes |
| Queries | `loom.queries.<name>` | Extra highlight rules for a grammar that another extension owns |

Languages and language servers are separate extensions, so each can be chosen,
updated and disabled on its own. A language names the servers it works well with in
`recommends`.

## Using it

```sh
npm ci --ignore-scripts          # two small dependencies: smol-toml and tar
node scripts/loom-ext.mjs check  # everything that can be verified offline
node scripts/loom-ext.mjs help   # the commands
```

Try an extension in a local Loom without publishing anything:

```sh
node scripts/loom-ext.mjs dev loom.theme.aura --to ~/loom-extensions-dev
LOOM_EXTENSIONS_DIR=~/loom-extensions-dev loom
```

## Layout

```text
extensions/<id>/        one folder per extension: a manifest, or a recipe.toml for those that are built
tools/toolchain.toml    the pinned toolchain that compiles grammars
tools/validator.lock    where to download loom-ext-validate
loom-ext.toml           the repository, the extensions Loom bundles, and index settings
scripts/                the loom-ext command line (Node, no build step) and its tests
docs/                   package format, index format, recipes, the tools, contributing
dist/                   everything the tools produce (not committed)
```

## More

- [The tools](docs/tools.md)
- [Packages](docs/package-format.md) and [the index](docs/index-format.md)
- [Recipes](docs/recipes.md)
- [Contributing](docs/contributing.md)

Licensed under MIT or Apache-2.0, at your option. The grammars, queries and programs
inside a built package keep their own licenses; each package carries them in
`licenses/`.

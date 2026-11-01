# The tools

`node scripts/loom-ext.mjs <command>`. Every command that takes `[id…]` works on all
extensions when none is named. Nothing in `dist/` or `.work/` is committed.

| Command | What it does |
| --- | --- |
| `check [id…]` | Manifests follow Loom's rules; recipes are well formed and pinned; file types are not claimed twice; the bundled list names real extensions. No network. |
| `build [id…] [--offline] [--work dir] [--platforms a,b]` | Stages each extension into `dist/stage/<platform>/<id>/`. A folder with a manifest is copied; one with a `recipe.toml` is built. |
| `validate [id…] [--require-validator] [--offline]` | Checks what `build` staged and, with `loom-ext-validate`, loads each language pack as Loom does and colors its sample. |
| `pack [id…]` | Packs the stage into `dist/packages/<id>-<version>-<platform>.loomext` and records them in `packages.json`. Packing is reproducible. |
| `index [--previous file] [--key file \| --unsigned] [--now iso]` | Builds `dist/index/index.json` from the packages and signs it. |
| `lock [--out dir]` | Writes `extensions.lock` and an index snapshot for Loom's repository. Needs a signed index. |
| `publish [id…] [--yes] [--no-index]` | Uploads each package as a GitHub release, checks the assets, publishes it, then replaces the index. A dry run without `--yes`. |
| `dev [id…] [--to dir] [--offline]` | Builds and copies extensions into a folder Loom reads (`LOOM_EXTENSIONS_DIR`). |
| `pin [id…] [--refresh]` | Resolves the hashes and lockfiles servers and runtimes need. Needs the network. |
| `keygen --out file` | Makes the index signing key; prints the public key for Loom. |

## Stages

```text
build     extensions/<id>  ->  dist/stage/<platform>/<id>/
validate  dist/stage       ->  checked
pack      dist/stage       ->  dist/packages/*.loomext, packages.json
index     packages.json    ->  dist/index/index.json (+ .sig)
publish   dist/packages, dist/index  ->  GitHub releases
lock      dist/index       ->  dist/lock/extensions.lock, index.snapshot.json
```

Platforms are written `<os>-<arch>` (`windows-x86_64`, `linux-aarch64`,
`macos-aarch64`, …); a package with no program in it is `any`.

## Configuration

`loom-ext.toml`:

- `repository`: where releases are published and downloaded from.
- `bundled`: the extensions Loom ships in its installer. `lock` pins exactly these.
- `index.key-id`: names the public key Loom uses to check the signature.
- `index.validity-days`: how long a signed index stays valid.

Environment: `LOOM_INDEX_KEY` (the private key, instead of `--key`),
`LOOM_EXTENSIONS_DIR` (where `dev` copies to), `LOOM_EXT_VALIDATE` (the validator
binary), `LOOM_EXT_ROOT` (run the tools against another checkout), `GITHUB_TOKEN`
(for `pin`).

## loom-ext-validate

Built in Loom's repository from the loader and query compilers the application uses,
and attached to each Loom release; this repository only runs it:

```text
loom-ext-validate <extension-folder> [--samples samples.json]
```

It prints one JSON object per grammar, `{"grammar": "rust", "colored": 42}`, and exits
with 0 when the folder loads and every sample in `samples.json`
(`[{"grammar": "rust", "sample": "<file>"}]`) is colored. List its builds, with their
SHA-256, in `tools/validator.lock`; until then `validate` checks structure only.

## Signing

The index is signed with Ed25519 over the exact bytes of `index.json`; the signature
file names the key (`keyId`). The private key lives in the `LOOM_INDEX_KEY` secret of
the release workflow and nowhere else, and is not the key that signs Loom itself. Loom
carries the public keys. To rotate, publish with a new key id once Loom knows both.

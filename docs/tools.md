# The tools

`node scripts/loom-ext.mjs <command>`. They work on finished packages: how an
extension builds them is its own project's business. Nothing in `dist/` or `.work/` is
committed.

| Command | What it does |
| --- | --- |
| `check <package or folder…>` | Takes the packages in as publishing would, without publishing: each is unpacked, its manifest and files are checked, and with `loom-ext-validate` Loom's own loaders import it. |
| `dev <package or folder…> [--to dir]` | Unpacks the package for this machine (or the one that runs anywhere) into a folder Loom reads (`LOOM_EXTENSIONS_DIR`). |
| `release preflight` | Checks that the signing key is the one Loom trusts. |
| `release intake <package or folder…> [--dry-run]` | Checks the packages against the signed index and the releases already published, and records what is to be published. `--dry-run` only checks. |
| `release publish` | Uploads the releases, then the signed index; see [Automatic releases](releases.md). |
| `lock [--out dir]` | Writes `extensions.lock` and an index snapshot for Loom's repository. Needs a signed index. |
| `keygen --out file` | Makes the index signing key; prints the public key for Loom. |

A folder given to a command stands for the `.loomext` files in it.

Platforms are written `<os>-<arch>` (`windows-x86_64`, `linux-aarch64`,
`macos-aarch64`, …); a package with no program in it is `any`.

## Configuration

`loom-ext.toml`:

- `repository`: where releases are published and downloaded from.
- `bundled`: the extensions Loom ships in its installer. `lock` pins exactly these.
- `index.key-id`: names the public key Loom uses to check the signature.
- `index.validity-days`: how long a signed index stays valid.

Environment: `LOOM_INDEX_KEY` (the private key), `LOOM_EXTENSIONS_DIR` (where `dev`
copies to), `LOOM_EXT_VALIDATE` (the validator binary), `LOOM_EXT_ROOT` (run the tools
against another checkout).

## loom-ext-validate

Built in Loom's repository from the loader and query compilers the application uses;
this repository only runs it:

```text
loom-ext-validate <extension-folder> [--samples samples.json]
```

It exits with 0 when Loom can import the folder. Samples are optional: with a
`samples.json` (`[{"grammar": "rust", "sample": "<file>"}]`) it also requires each
sample to be colored; publishing does not ask for them. List its builds, with their
SHA-256, in `tools/validator.lock`.

## Signing

The index is signed with Ed25519 over the exact bytes of `index.json`; the signature
file names the key (`keyId`). The private key lives in the `LOOM_INDEX_KEY` secret of
the publish workflow and nowhere else, and is not the key that signs Loom itself. Loom
carries the public keys. To rotate, publish with a new key id once Loom knows both.

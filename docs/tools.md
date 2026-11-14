# The tools

`node scripts/loog-ext.mjs <command>`. They work on finished packages: how an
extension builds them is its own project's business. Nothing in `dist/` or `.work/` is
committed.

| Command | What it does |
| --- | --- |
| `check <package or folder…>` | Takes the packages in as publishing would, without publishing: each is unpacked, its manifest and files are checked, and with `loog-ext-validate` Loog's own loaders import it. |
| `dev <package or folder…> [--to dir]` | Unpacks the package for this machine (or the one that runs anywhere) into a folder Loog reads (`LOOG_EXTENSIONS_DIR`). |
| `try <id…> [--source loog-dir] [--to dir] [--run command] [--no-run]` | Builds the named extensions for this machine (`node build.mjs <platform>` in each folder, `npm ci` first when `node_modules` is missing), unpacks them as `dev` does, then starts Loog on that folder. Extensions they `requires` are built too unless the folder already holds them. The folder is `--to`, `LOOG_EXTENSIONS_DIR` or `.work/dev-extensions`; The Loog checkout is `--source` or `LOOG_SOURCE`: builds compile against it, and with no `--run` or `LOOG_EXT_RUN` Loog starts as `cargo run` in it. Otherwise Loog is `--run`, `LOOG_EXT_RUN` or `loog`, run from the current directory. `--no-run` stops after unpacking. Before `cargo run` it deletes `target/debug/extensions` in the checkout: a debug build never refreshes those bundled copies, so one left by an older Loog would show an extension at a version that no longer exists. Loog is restarted on every run; nothing is reloaded in place. |
| `release preflight` | Checks that the signing key is the one Loog trusts. |
| `release intake <package or folder…> [--dry-run]` | Checks the packages against the signed index and the releases already published, and records what is to be published. `--dry-run` only checks. |
| `release publish` | Uploads the releases, then the signed index; see [Automatic releases](releases.md). |
| `lock [--out dir]` | Writes `extensions.lock` and an index snapshot for Loog's repository. Needs a signed index. |
| `keygen --out file` | Makes the index signing key; prints the public key for Loog. |

A folder given to a command stands for the `.loogext` files in it.

Platforms are written `<os>-<arch>` (`windows-x86_64`, `linux-aarch64`,
`macos-aarch64`, …); a package with no program in it is `any`.

## Configuration

`loog-ext.toml`:

- `repository`: where releases are published and downloaded from.
- `bundled`: the extensions Loog ships in its installer. `lock` pins exactly these.
- `index.key-id`: names the public key Loog uses to check the signature.
- `index.validity-days`: how long a signed index stays valid.

Environment: `LOOG_INDEX_KEY` (the private key), `LOOG_EXTENSIONS_DIR` (where `dev` and
`try` copy to), `LOOG_EXT_RUN` (the command `try` starts), `LOOG_EXT_VALIDATE` (the validator binary), `LOOG_EXT_ROOT` (run the tools
against another checkout).

## loog-ext-validate

Built in Loog's repository from the loader and query compilers the application uses;
this repository only runs it:

```text
loog-ext-validate <extension-folder> [--samples samples.json]
```

It exits with 0 when Loog can import the folder. Samples are optional: with a
`samples.json` (`[{"grammar": "rust", "sample": "<file>"}]`) it also requires each
sample to be colored; publishing does not ask for them. List its builds, with their
SHA-256, in `tools/validator.lock`.

The resource workspace ABI requires the matching SDK and validator. Until new
validator binaries are published, the lock deliberately has no platform entries;
CI builds the validator from `tools/loog-source.toml`. Local development can set
`LOOG_EXT_VALIDATE` to a validator built from that same checkout. Do not reuse an
older binary left in `tools/bin` for workspace components.

Paired development PRs pin an immutable Loog commit and run AOT instantiation on
all six native hosts, including the SSH and JSON components. These checks use
ephemeral test signatures. Publication additionally requires that exact SDK commit
to be reachable from Loog main. A squash merge requires repinning and validation.

## Signing

The index is signed with Ed25519 over the exact bytes of `index.json`; the signature
file names the key (`keyId`). The private key lives in the `LOOG_INDEX_KEY` secret of
the publish workflow and nowhere else, and is not the key that signs Loog itself. Loog
carries the public keys. To rotate, publish with a new key id once Loog knows both.

# Official Rust WASM client comparison

Rust-analyzer and TypeScript Language Server 0.2.3 compile the same private Rust source independently. Server payloads and the Node runtime match the preserved 0.2.2 TypeScript baselines byte for byte. These are Windows x64 observations on a Ryzen 5 5600GT, not general limits. The Go comparison in extensions/loog.lsp.gopls/rust/BENCHMARKS.md is historical; Go now compiles the shared source and passed the native feature regression again.

| Language | WASM MiB, TS / Rust | Cold first usable s, TS / Rust | Small host resident MiB, TS / Rust |
| --- | --- | --- | --- |
| rust | 13.05 / 1.66 | 16.80 / 4.60 | 58.74 / 29.61 |
| javascript | 13.05 / 1.66 | 14.88 / 2.44 | 60.75 / 42.68 |
| typescript | 13.05 / 1.66 | 15.21 / 2.61 | 60.28 / 43.27 |

| Language | Bytes | Operation | TS p50 / p95 ms | Rust p50 / p95 ms |
| --- | --- | --- | --- | --- |
| rust | 116 | Completion | 12.972 / 14.629 | 11.817 / 13.313 |
| rust | 116 | Hover | 1.106 / 1.353 | 0.360 / 0.420 |
| rust | 116 | Definition | 1.101 / 1.397 | 0.358 / 0.396 |
| rust | 1048576 | Completion | 13.836 / 18.688 | 11.315 / 12.982 |
| rust | 1048576 | Hover | 1.131 / 2.322 | 0.353 / 0.401 |
| rust | 1048576 | Definition | 1.142 / 2.537 | 0.347 / 0.408 |
| rust | 8388608 | Completion | 13.020 / 14.348 | 11.794 / 12.918 |
| rust | 8388608 | Hover | 1.125 / 1.533 | 0.370 / 0.424 |
| rust | 8388608 | Definition | 1.122 / 1.360 | 0.362 / 0.404 |
| javascript | 139 | Completion | 26.719 / 30.371 | 24.068 / 27.990 |
| javascript | 139 | Hover | 1.816 / 2.471 | 0.983 / 1.453 |
| javascript | 139 | Definition | 1.470 / 1.773 | 0.675 / 0.928 |
| javascript | 1048576 | Completion | 27.548 / 30.370 | 24.848 / 28.227 |
| javascript | 1048576 | Hover | 1.783 / 2.378 | 0.925 / 1.409 |
| javascript | 1048576 | Definition | 1.503 / 2.033 | 0.669 / 0.873 |
| javascript | 8388608 | Completion | 28.945 / 31.636 | 24.354 / 27.373 |
| javascript | 8388608 | Hover | 1.932 / 2.650 | 0.889 / 1.279 |
| javascript | 8388608 | Definition | 1.618 / 3.311 | 0.674 / 0.902 |
| typescript | 120 | Completion | 20.957 / 30.648 | 18.776 / 25.858 |
| typescript | 120 | Hover | 1.533 / 2.263 | 0.969 / 1.625 |
| typescript | 120 | Definition | 1.343 / 2.032 | 0.647 / 1.073 |
| typescript | 1048576 | Completion | 27.977 / 37.387 | 23.720 / 30.193 |
| typescript | 1048576 | Hover | 1.778 / 2.530 | 0.929 / 1.397 |
| typescript | 1048576 | Definition | 1.576 / 2.727 | 0.652 / 0.923 |
| typescript | 8388608 | Completion | 20.845 / 30.769 | 19.708 / 26.215 |
| typescript | 8388608 | Hover | 1.480 / 2.141 | 0.788 / 1.134 |
| typescript | 8388608 | Definition | 1.231 / 2.055 | 0.577 / 0.723 |

All 81 paired gates use Rust p95 <= TS p95 + max(10 ms, 20% of TS p95): 81 passed. Each warm table entry pools three cohorts of 100 requests after ten warm-ups. The 16200 warm and 1800 cold-cohort measured requests all succeeded. Run order alternates; builds do not run during measurements. There is no outlier removal. Small completion latency is not uniformly improved.

Readiness requires ten consecutive usable completion/hover/definition probes and at least two seconds; first usable time is separate. End-to-end cold startup is observed once per route, with an empty Wasmtime cache. Component-only compile/load, instantiate and dispatch use three isolated processes per mode. OS and server caches are retained. Large Unicode/CRLF documents are exactly 1 MiB and 8 MiB.

Memory is the windowless native Loog test-host process, including Wasmtime and the plugin, excluding the separate language server and its children. TypeScript still depends on Node and tsserver. This is not whole-GUI or total ecosystem memory. Local first-load Wasmtime compilation remains; release-side AOT distribution is separate.

Each of the three package configurations passes 19 Rust protocol/lifecycle tests and Clippy. The 87 publishing tool tests pass. Production import validation passes for all three Windows packages. Native server checks also pass for JSX/TSX and the refactored Go client. Dynamic registration, rollback, cancellation, backpressure, settings, multiple roots and operation receipts have synthetic protocol coverage. Linux/macOS execution and manual GUI QA were not performed. An initial candidate regressed JS completion latency; redundant JSON copying/serialization was removed and every final language cohort rerun. Initial candidate measurements are preserved separately under rejected-attempt-1.

## Reproduce

Preserve the TS baseline before staging the candidate. Both roots need valid extensions.json indexes and versioned package folders; TS/JS also need the same loog.runtime.node. Use matching Loog source with the benchmark-only tests/example patch. Build before measuring:

```sh
cargo test --release --bin Loog --no-run --offline
cargo build --release -p loog-extension-host --example component_cost --offline
# In the extensions repository, HOST is the release test executable printed above:
node language-clients/rust/benchmark.mjs --loog LOOG --host HOST --baseline TS_ROOT --candidate RUST_ROOT --out EMPTY_OUTPUT --language rust --runs 3
# Repeat with --language javascript and --language typescript, each with a fresh output directory.
```

The runner verifies identical server payload hashes (the entire node_modules tree for TS/JS), and Node runtime hashes. Metadata retains component and test-executable hashes, commit IDs, tool versions, all cohorts and cache states. Candidate sources are uncommitted local changes accompanying the evidence; the commit ID alone does not identify the migration. LOOG_MIGRATION_VARIANTS=1 enables JSX/TSX native fixtures; LOOG_MIGRATION_FULL_CHECKS=1 probes negotiated features.

The preserved TS/JS development fixtures on both measured routes also retain nine identical, unused npm .bin launcher scripts. Release builders have always used --no-bin-links and omit those shims; the client invokes Node and cli.mjs directly. Final artifact auditing verifies every other server package file byte for byte against the measured payload. The full development-tree hash and normalized release-server hash are both retained in payload_audit in the artifact manifest. This layout difference is not a server version change.

## Artifact identities

- rust TS WASM: `5ca9d9e2c1c3630ce62f9d3d131fb7703dde547e8af17badbd2cec0ccb2dd885`
- rust Rust WASM: `b0bb9c63e0dd8f99184ad311af0d832a32d08067a7d67a2373b2ccf943e1b336`
- rust server payload: `6f76533ca854bc2708bfa1caab68240ed5b89e530eafc1ac9865f5a569f5ec1d`

- javascript TS WASM: `68f7dd8dbcef39a5819af82c194ffff0420a70e1a0b91eb450ca0393a9ba2ecc`
- javascript Rust WASM: `43304325c5e1abc0244ca6e0cc327d7aa93957cbcc61a5061db6edb2cc51d08d`
- javascript server payload: `facd91275c6784dd1220f2c64362deb4d03773d60a451e5bcc350fda0680423e`

- typescript TS WASM: `68f7dd8dbcef39a5819af82c194ffff0420a70e1a0b91eb450ca0393a9ba2ecc`
- typescript Rust WASM: `43304325c5e1abc0244ca6e0cc327d7aa93957cbcc61a5061db6edb2cc51d08d`
- typescript server payload: `facd91275c6784dd1220f2c64362deb4d03773d60a451e5bcc350fda0680423e`

# gopls Rust WASM comparison

Measured on Windows x64 with a Ryzen 5 5600GT, Rust 1.98.0 and the same pinned gopls v0.24.0. These are observations from this machine, not general limits.

The component shrank from 13.05 to 1.65 MiB. With an empty Wasmtime cache, initialization was 11.63 / 1.52 s (TS / Rust), and first usable completion, hover and definition took 12.90 / 2.73 s. Each end-to-end cold route ran once. Separate component cold-compilation medians over three fresh processes were 10.78 / 1.39 s.

| Bytes | Operation | TS p50 / p95 ms | Rust p50 / p95 ms |
| --- | --- | --- | --- |
| 144 | Completion | 15.693 / 18.149 | 14.927 / 20.356 |
| 144 | Hover | 0.750 / 0.924 | 0.316 / 0.373 |
| 144 | Definition | 0.774 / 1.001 | 0.318 / 0.406 |
| 1048576 | Completion | 16.301 / 24.966 | 14.758 / 19.691 |
| 1048576 | Hover | 0.717 / 0.869 | 0.317 / 0.404 |
| 1048576 | Definition | 1.474 / 1.946 | 0.985 / 1.380 |
| 8388608 | Completion | 15.608 / 18.719 | 14.134 / 16.425 |
| 8388608 | Hover | 0.709 / 0.960 | 0.286 / 0.401 |
| 8388608 | Definition | 6.108 / 8.040 | 5.404 / 7.145 |

| Bytes | Host resident MiB, TS / Rust | Host private MiB, TS / Rust |
| --- | --- | --- |
| 144 | 54.93 / 22.45 | 28.00 / 7.73 |
| 1048576 | 67.65 / 35.45 | 103.53 / 22.75 |
| 8388608 | 165.79 / 126.18 | 187.63 / 126.98 |

The warm table pools three 100-request cohorts per operation after ten warm-ups each. TS/Rust run order alternates. Readiness requires ten consecutive usable probes and at least two seconds; first usable time is recorded separately. No build runs concurrently. Failed measured requests fail the run and are not discarded. All 27 paired p95 gates passed the unchanged reference + max(10 ms, 20%) budget.

Memory is the windowless Loom test-host process, including Wasmtime and the one plugin, excluding the separate gopls process. It is not whole-GUI memory. Cold means empty Wasmtime caches; OS and Go caches are retained. Small completion latency varies between runs and is not uniformly improved. Large inputs contain Chinese, emoji and CRLF.

The Rust client has 15 protocol/lifecycle tests. Real native/WASM checks cover the negotiated provider surface, including symbols, references, rename, formatting, signature help, call/type hierarchy, completion, hover, definition and restart. Synthetic contract tests additionally cover resolve, diagnostics, file watches, rollback, cancellation, backpressure, settings, multiple roots and native operation receipts. Linux/macOS and manual GUI QA were not run.

## Reproduction

Keep the original TS 0.2.2 package before staging Rust 0.2.3. Both extension roots need extensions.json and versioned folders. Use the same gopls binary in both. Build the release host test and component_cost example from the matching Loom checkout:

```sh
cargo test --release --bin Loom --no-run --offline
cargo build --release -p loom-extension-host --example component_cost --offline
# In the extensions repository; HOST is the release test executable printed above:
node extensions/loom.lsp.gopls/rust/benchmark.mjs --loom LOOM --host HOST --baseline TS_ROOT --candidate RUST_ROOT --out EMPTY_OUTPUT --runs 3
```

benchmark.mjs records the platform, tool versions, component/server/test-executable hashes, every cohort and cache state. The output directory must be empty. Wasmtime compilation is still local on the first uncached activation; release-side AOT distribution is a separate change.

## Artifact identities

- TS WASM: `fe01047001e4123f9b57c9b93bf2e1c45490c3fa48f2fb9670ea7789da6de170`
- Rust WASM: `a1621e107ce20ce9337b6d57d291b34bdd894b320355f3a93f0edafa92aad92d`
- gopls: `9613ba491798d2468f279b2a4d8eca88323e863a642998ac261c970904f6c399`
- Loom source: `40053d5134fa2dc9b0358c0b66adee7b64ab5df0` plus the benchmark-only patch.
- Extensions baseline: `9b3f8c8ce63a4cebb9a6f003a8d74a95b5e4e9d3`; candidate source is the accompanying local migration.

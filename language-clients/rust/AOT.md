# Release AOT verification

The three official clients remain Rust WASM components. The trusted publisher compiles them with locked Wasmtime 50.0.1 and baseline CPU targets, signs code provenance with the existing publication key under a separate AOT signature domain, then uses the existing immutable release and registry path. Plugin build jobs never see the key, and handed-over native AOT is discarded.

The production host pins compiler authorities and verifies plugin identity/version, profile, target, engine fingerprint and both WASM/native SHA-256 before deserializing owned bytes. It retains the WASM/cache fallback and logs why an AOT artifact was rejected. The ordinary CPU-tuned engine/cache is unchanged; the portable engine receives the same epoch ticker and store limits. Production does not read the temporary benchmark trust environment variable.

An updated host is required: installing AOT does not retrofit an older Loom binary. Engine/profile changes require newly compiled, signed artifacts and new plugin versions; existing published packages remain immutable. TypeScript still runs its external Node server.

## Release gate

The reusable publication workflow compiles an ephemeral signed corpus from the actual package WASM on Linux, then requires loading and ABI instantiation on six native runners. It downloads the immutable artifact ID from the trusted compiler job, verifies the expected native triple, requires `published-aot` and no local compilation-cache creation, instantiates the real component world and dispatches an empty batch. Production signing still recompiles WASM itself and never uses the test corpus as a release package. An explicit no-client batch may skip the matrix; failure blocks publication.

## Windows measurement

The accompanying evidence uses identical Rust WASM 0.2.3 / signed AOT 0.2.4 inputs, server files and Node runtime. Four languages (Go/Rust/JS/TS), three document sizes, three cold starts and three warm cohorts of 100 requests per operation are retained. Every AOT cohort asserts its actual code source. The local authority is temporary and is injected only into the test executable. There is no production trust override.

Cold first usable time and memory include native host activity but exclude separately running language servers from memory. First usable time includes server indexing. Cold clears Wasmtime caches, not OS/server caches. Existing cached mappings can be faster and smaller than authenticated AOT copies; the tradeoff is shown. GUI manual validation and foreign-OS performance are not claimed by these Windows numbers.

```sh
node language-clients/rust/aot-benchmark.mjs --loom LOOM --host RELEASE_TEST_HOST --baseline WASM_ROOT --candidate AOT_ROOT --out EMPTY_DIR --language go --runs 3
# Repeat for rust, javascript, typescript with separate empty output directories.
# Test-signed local packages additionally require --test-authority BENCHMARK_PUBLIC_KEY_FILE.
```

The host-side `crates/extension-host/examples/component_cost.rs` measures compile/load, instantiate, empty dispatch and process memory in fresh processes; `aot_verify.rs` implements the native release gate. Full request logs, all sorted samples, runtime/tool hashes and methodology are retained with the comparison.

## Recorded comparison

```text
三个语言插件：发布端 AOT 端到端对照
对照：Rust WASM 0.2.3（首次本地编译）→ 签名 AOT 0.2.4；两边 WASM、服务器和 Node 文件哈希一致。JS 与 TS 共用一个插件，分别验收。
每个数值为三轮中位数，热态操作 p50/p95 合并三轮各 100 个请求。

端到端冷启动与宿主内存（包含服务器启动/索引耗时，内存不含独立服务器）：
语言 | 初始化 ms，WASM → AOT | 首次可用 ms，WASM → AOT | 就绪工作集 MiB，WASM → AOT | 进程峰值工作集 MiB，WASM → AOT
go | 1462.66 → 75.49 | 2519.77 → 1202.71 | 33.43 → 26.24 | 68.85 → 50.18
rust | 1444.39 → 40.82 | 4416.67 → 2920.43 | 40.44 → 33.40 | 68.65 → 50.30
javascript | 1636.13 → 174.95 | 2296.32 → 853.97 | 53.49 → 46.35 | 68.91 → 50.29
typescript | 1680.04 → 164.79 | 2412.13 → 891.37 | 53.93 → 46.96 | 68.44 → 50.21

独立组件（不启动服务器）冷加载/编译与峰值内存：
go: 1434.31 → 7.79 ms；峰值工作集 56.89 → 21.39 MiB；已缓存加载 1.29 → 7.78 ms。
rust: 1508.67 → 8.10 ms；峰值工作集 57.12 → 21.40 MiB；已缓存加载 1.38 → 7.82 ms。
javascript: 1521.42 → 7.76 ms；峰值工作集 57.25 → 21.45 MiB；已缓存加载 1.29 → 7.90 ms。
typescript: 1430.04 → 7.95 ms；峰值工作集 56.53 → 21.46 MiB；已缓存加载 1.34 → 7.94 ms。

缓存热态内存/首次可用/恢复：
go 144 B: 工作集 22.54 → 26.23 MiB；私有字节 7.67 → 11.59 MiB；首次可用 1100.12 → 1180.55 ms；恢复 1857.59 → 1846.87 ms。
go 1048576 B: 工作集 35.79 → 39.66 MiB；私有字节 22.61 → 27.28 MiB；首次可用 1113.40 → 1138.29 ms；恢复 1871.56 → 1862.96 ms。
go 8388608 B: 工作集 126.33 → 129.93 MiB；私有字节 126.84 → 131.80 MiB；首次可用 1378.53 → 1348.13 ms；恢复 2048.36 → 2002.48 ms。
rust 116 B: 工作集 29.77 → 33.46 MiB；私有字节 13.93 → 18.61 MiB；首次可用 2863.34 → 2906.28 ms；恢复 1493.19 → 1582.95 ms。
rust 1048576 B: 工作集 38.29 → 41.86 MiB；私有字节 23.57 → 28.34 MiB；首次可用 2793.53 → 882.38 ms；恢复 1912.13 → 2255.66 ms。
rust 8388608 B: 工作集 127.50 → 130.64 MiB；私有字节 127.90 → 132.25 MiB；首次可用 2923.52 → 2880.92 ms；恢复 7529.54 → 2112.76 ms。
javascript 139 B: 工作集 42.83 → 46.51 MiB；私有字节 28.00 → 32.57 MiB；首次可用 884.73 → 925.55 ms；恢复 1809.10 → 1838.39 ms。
javascript 1048576 B: 工作集 50.98 → 55.12 MiB；私有字节 36.95 → 42.57 MiB；首次可用 885.38 → 902.82 ms；恢复 1878.42 → 1823.69 ms。
javascript 8388608 B: 工作集 125.46 → 129.36 MiB；私有字节 127.99 → 132.56 MiB；首次可用 1158.41 → 1227.65 ms；恢复 2080.81 → 2110.09 ms。
typescript 120 B: 工作集 43.36 → 46.99 MiB；私有字节 28.20 → 33.25 MiB；首次可用 939.98 → 930.34 ms；恢复 1846.05 → 1883.64 ms。
typescript 1048576 B: 工作集 51.46 → 55.17 MiB；私有字节 38.34 → 42.67 MiB；首次可用 1037.69 → 953.20 ms；恢复 1922.76 → 2003.12 ms。
typescript 8388608 B: 工作集 125.88 → 129.40 MiB；私有字节 127.82 → 132.38 MiB；首次可用 1136.66 → 1282.96 ms；恢复 2100.93 → 2095.17 ms。

请求延迟：语言 | 字节 | 操作 | WASM p50/p95 ms | AOT p50/p95 ms
go | 144 | Completion | 20.163/24.483 | 20.332/23.976
go | 144 | Hover | 0.392/0.478 | 0.385/0.450
go | 144 | Definition | 0.389/0.456 | 0.379/0.443
go | 1048576 | Completion | 20.401/24.767 | 18.455/22.874
go | 1048576 | Hover | 0.408/0.491 | 0.384/0.442
go | 1048576 | Definition | 1.455/1.829 | 1.316/1.674
go | 8388608 | Completion | 19.178/22.751 | 18.210/22.518
go | 8388608 | Hover | 0.393/0.451 | 0.404/0.479
go | 8388608 | Definition | 7.518/9.886 | 7.624/10.178
rust | 116 | Completion | 11.567/12.927 | 11.517/13.209
rust | 116 | Hover | 0.359/0.402 | 0.365/0.414
rust | 116 | Definition | 0.352/0.396 | 0.358/0.406
rust | 1048576 | Completion | 11.257/12.582 | 11.520/13.128
rust | 1048576 | Hover | 0.360/0.405 | 0.360/0.414
rust | 1048576 | Definition | 0.353/0.396 | 0.354/0.396
rust | 8388608 | Completion | 11.218/12.754 | 11.378/12.598
rust | 8388608 | Hover | 0.367/0.428 | 0.356/0.402
rust | 8388608 | Definition | 0.375/0.540 | 0.351/0.393
javascript | 139 | Completion | 23.613/27.180 | 24.068/27.966
javascript | 139 | Hover | 0.881/1.219 | 0.932/1.637
javascript | 139 | Definition | 0.649/0.809 | 0.665/1.081
javascript | 1048576 | Completion | 24.141/27.119 | 23.869/26.692
javascript | 1048576 | Hover | 0.889/1.220 | 0.881/1.199
javascript | 1048576 | Definition | 0.671/0.849 | 0.655/0.809
javascript | 8388608 | Completion | 23.590/26.919 | 24.177/27.099
javascript | 8388608 | Hover | 0.876/1.153 | 0.911/1.199
javascript | 8388608 | Definition | 0.666/0.957 | 0.661/0.825
typescript | 120 | Completion | 24.954/28.398 | 24.875/28.160
typescript | 120 | Hover | 0.960/1.409 | 0.903/1.158
typescript | 120 | Definition | 0.665/0.800 | 0.640/0.748
typescript | 1048576 | Completion | 25.238/29.793 | 25.027/28.820
typescript | 1048576 | Hover | 0.953/1.390 | 0.897/1.259
typescript | 1048576 | Definition | 0.747/1.411 | 0.700/0.916
typescript | 8388608 | Completion | 24.236/26.762 | 24.420/27.194
typescript | 8388608 | Hover | 0.875/1.237 | 0.823/1.022
typescript | 8388608 | Definition | 0.683/0.865 | 0.638/0.713

逐轮 p95 门槛：108/108 通过，AOT <= WASM + max(10ms, WASM 的 20%)；门槛未放宽。
go 两条路线的协商能力相同，额外真实功能检查 21 项：CodeAction, CodeActionResolve, CodeLens, DocumentHighlight, DocumentSymbol, FoldingRange, Formatting, Implementation, IncomingCalls, InlayHint, OutgoingCalls, PrepareCallHierarchy, PrepareRename, PrepareTypeHierarchy, References, SelectionRange, SignatureHelp, Subtypes, Supertypes, TypeDefinition, WorkspaceSymbol。
rust 两条路线的协商能力相同，额外真实功能检查 20 项：CodeAction, CodeActionResolve, CodeLens, CompletionResolve, DocumentHighlight, DocumentSymbol, FoldingRange, Formatting, Implementation, IncomingCalls, InlayHint, OnTypeFormatting, OutgoingCalls, PrepareCallHierarchy, PrepareRename, References, SelectionRange, SignatureHelp, TypeDefinition, WorkspaceSymbol。
javascript 两条路线的协商能力相同，额外真实功能检查 20 项：CodeAction, CodeActionResolve, CodeLens, CompletionResolve, DocumentHighlight, DocumentSymbol, FoldingRange, Formatting, Implementation, IncomingCalls, InlayHint, OutgoingCalls, PrepareCallHierarchy, PrepareRename, RangeFormatting, References, SelectionRange, SignatureHelp, TypeDefinition, WorkspaceSymbol。
typescript 两条路线的协商能力相同，额外真实功能检查 20 项：CodeAction, CodeActionResolve, CodeLens, CompletionResolve, DocumentHighlight, DocumentSymbol, FoldingRange, Formatting, Implementation, IncomingCalls, InlayHint, OutgoingCalls, PrepareCallHierarchy, PrepareRename, RangeFormatting, References, SelectionRange, SignatureHelp, TypeDefinition, WorkspaceSymbol。

方法与边界：
- 96 个端到端轮次、48 个独立组件轮次；28800 个正式请求全部保留。失败会中止轮次，未筛除慢样本；所有 AOT 轮次必须报告 published-aot，不能靠本地缓存或回退编译通过。
- 每轮使用全新宿主/服务器；冷启动各三次，独立空缓存；顺序交替。就绪需连续十组可用补全/悬停/定义且持续至少两秒，首次可用另记。热态每项另预热十次，再测一百次。测量期间没有构建。
- 大文件含中文、emoji 和 CRLF，严格为 1 MiB 和 8 MiB；小文件按语言保持相同内容。原开发 TS 夹具的九个未使用 npm .bin 脚本已移除，两条路线均采用正式包的服务器布局。
- 内存是无窗口 Loom 测试宿主的工作集/私有字节，排除独立语言服务器及其子进程；不是整个 GUI 的内存。进程峰值为 OS 累积峰值，包含就绪前分配。
- AOT 优化首次加载；已有本地编译缓存可能更快。AOT 每次仍做验签、SHA-256 和安全复制，热态请求也不保证逐轮更快。Rust 索引会波动，不把单轮索引差异解释为编译器收益。
- 当前本地对照使用临时测试签名，测试信任仅在 cfg(test) 宿主显式注入；生产固定官方发布公钥。正式包由 CI 已有 LOOM_INDEX_KEY 签发，需含 AOT 加载器的新宿主。旧宿主、签名失败或不兼容配置保留 WASM 回退。
- 生产使用原生编译代码需可信编译器签名，并绑定插件 ID/版本、WASM/代码 SHA-256、目标和引擎配置。加载已验证的内存字节，避免验证后再映射可变文件的竞态。
- 本机 Windows x64 实测；跨系统 ABI 验收在发布前由 CI 六种原生 runner 强制执行。GUI 人工验收未作为这些数字的依据。

复现：language-clients/rust/aot-benchmark.mjs --help。原始每轮 JSON、所有请求样本、二进制/组件/服务器哈希、测试日志和源补丁随证据包保存。
```

References: [Wasmtime component deserialization](https://docs.rs/wasmtime/50.0.1/wasmtime/component/struct.Component.html#method.deserialize), [explicit targets](https://docs.rs/wasmtime/50.0.1/wasmtime/struct.Config.html#method.target), [GitHub native runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

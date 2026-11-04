# rust-ohos-kit

**同一个问题，换一种语言问：把 HarmonyOS 应用的逻辑放进 Rust，要付多少代价。**

> 这个仓库是**实验组**。对照组是 [`../go-ohos-kit`](../go-ohos-kit)：同一个 ArkTS 外壳、同样的八块
> 面板，内核是 Go，代价是 4 个 runtime 补丁 + 11 MB 产物。两边的补丁逐条对照、桥读数、体积在
> [`CONTEXT.md`](CONTEXT.md)。

对照实验。`../go-ohos-kit` 已经用 Go 回答过一遍：Go 内核 + ArkTS 宿主，真机跑通，
代价是 **4 个 Go runtime 补丁**（改 GOROOT、用 `-overlay` 维护）、一份 11 MB 的 `.so`，
以及一套「每次升级 Go 都要重跑锚点检查」的负担。

这个仓库问的是：Rust 要不要还同样的债。

## 已经量出来的

| | Go（`../go-ohos-kit`） | Rust（这里） |
|---|---|---|
| 交叉编译前的准备 | 4 个 runtime 补丁 + overlay 工具 + 每版 Go 重验 | **无**（NDK 自带 cargo 要的 linker） |
| 产物大小 | 11 MB | **772 KB** |
| `TLS_TPREL64` 重定位 | 必须是 0（补丁就是为了它） | **本来就是 0** |
| 内核能否脱离设备测试 | 能（`go test ./core/`） | 能（`cargo test -p rustkit-core`） |
| 数据库 | `modernc.org/sqlite`（纯 Go） | `rusqlite` + bundled（**编 C**，交叉编译 20 s） |
| SQLite 20k 行 seed | 60.2 ms | **12.3 ms** |
| 桥的往返（空调用，5 次） | 0.3 µs（0.3–0.3） | **0.1 µs**（0.1–0.2） |
| JSON 回串（5 次） | 3.1 µs | **1.3 µs** |
| 像素数学（5 次） | 475 µs | **362 µs** |

## 为什么它更省事

Go 是带着一整套操作系统进场的：调度器、TLS 约定、`_rt0` 启动流程、auxv 探测。`dlopen`
进来的 `.so` 没有这些，所以 Go 必须假装自己是那个进程，于是有了那四个补丁。

Rust 没有运行时可言：一个 ELF 里的代码，libc 由宿主进程准备好，线程局部变量走 `__thread`。
那四笔债一笔都不用还 —— 逐条对照见 [`CONTEXT.md`](CONTEXT.md) 第二节。

## 三层

```text
ArkTS / ArkUI       界面、权限、Kit、surface
    │  import 'librustcore.so'
NAPI 胶水           rust/bridge/csrc/napi_ohos.c —— 基本是 Go 版的原文，只换了前缀
    │  extern "C"
Rust 内核           rust/core —— 纯 Rust，Mac 上 cargo test
```

那层 C 胶水几乎是 Go 版的原文，这一点是刻意的：**如果"边界"这个说法成立，胶水就不该
分辨得出底下是什么语言。** 实际差别只有一个 `rustkit_set_emit()` 注册函数，因为 Rust
不引用 C 符号（只有 C 引用 Rust）。

## 已经搬过来的

`rust/core` 四个模块，测试用例是从 `go/core/*_test.go` 一条条搬的 —— 同一张照片、同一段
行走、同一个加速度，两边必须给出同一个数字：

| 模块 | 内容 |
|---|---|
| `todos` | 列表、id 规则（含"重启跳号"那个 off-by-one 的回归测试） |
| `image` | BT.601 luma、8 分桶直方图（余数给最高柱）、8×8 光照网格、4096 色桶不排序取前三 |
| `level` | 1/5 平滑、`atan2` 出 pitch/roll、每秒一次采样率 |
| `geo` | haversine、初始方位角、按精度过滤抖动、速度平滑 |
| `file` | magic bytes 认格式 + 全文件 SHA-256 |

线格式（`rows` + `encode_rows`）也在里面：同一页 2000 行，JSON 与紧凑格式两条路。

ArkTS 外壳已经搬过来（`app/`，改名 `librustcore` / `RustCore.ts` / 包名 `cn.isfk.rustkit`），
`hvigorw assembleHap` 通过。

**真机跑通了**：`cn.isfk.rustkit` 在设备上装着，八个面板都是 Rust 在供数据。
空调用 **0.3 µs → 0.1 µs**、JSON 回串 **3.1 → 1.3 µs**（两边各 5 次，5/5 一致）。原因不是
"Rust 更快"，是 **cgo 每次调用要付运行时切换，`extern "C"` 只是一次跳转**。
更细的读数（含哪些数字噪声大、不该引用）在 [`CONTEXT.md`](CONTEXT.md) 第 3 节末尾。

数据库也做了，而且结论和预想相反：`rusqlite` 的 bundled C 交叉编译到
`aarch64-unknown-linux-ohos` 只要 20 秒，跑起来 20k 行 seed **12.3 ms**（Go 的纯 Go 驱动 60.2 ms）。
代价不是"做不到"，是**纯不了**：`rust/core` 之外多出 `rust/store` 一个 crate，C 只从那里进来。
`.so` 也因此从 788 KB 长到 2.7 MB —— 仍比 Go 版的 11 MB 小 4 倍。

**八块面板、桥读数（各 5 次）、SQLite、相机预览、暗色主题都验过了。**

剩下的不是"还没做"，是"有意不做"：除了 SQLite 那一条，其余全部与 Go 版同构 ——
`scripts/check.sh` 现在还会断言两份 `color.json` 的 token 名集合一致（缺一个在暗色下会解析成
空，只在设备上现形）。

## 命令

```sh
scripts/check.sh              # fmt + clippy + test + 交叉编译 + 两条断言
scripts/check.sh --no-build   # 没有 SDK 的机器
platform/ohos/build.sh        # 只编 librustcore.so
platform/ohos/run.sh          # 编 + 装 + 跑（要设备；需要先搬 app/）
```

一次自检的样子：

```text
== cargo fmt ==          ok
== cargo clippy ==       ok
== cargo test ==         24 passed (core), 2 passed (bridge)
== cross build ==        librustcore.so — 772K, 15 exports, 0 TPRELs
```

## 目录

```text
rust/core/            纯 Rust：cargo test 在这里过，不认识手机
rust/bridge/          cdylib：src/lib.rs 是桥，csrc/napi_ohos.c 是 NAPI
platform/ohos/        build.sh（交叉编译）、run.sh（编 + 装 + 跑）
scripts/check.sh      fmt、clippy、test、交叉编译，以及两条硬断言
CONTEXT.md            这个仓库从哪来、Go 那四笔债、还没做什么
AGENTS.md             动手前要读的规则和坑
```

## 来源

界面与设计决策来自 [`../go-ohos-kit`](../go-ohos-kit)：那套 ArkTS UI 已经做过一次完整返工并在
真机验过（面板无边框、单一 ink 中性阶、Go 青只给"活着"的东西、分割线内缩且末行不画）。
搬 `app/` 时原样保留，别重新设计 —— 那是这个对照实验的**控制变量**。

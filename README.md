# rust-ohos-kit

**同一个问题，换一种语言问：把 HarmonyOS 应用的逻辑放进 Rust，要付多少代价。**

> **实验组** · 对照组：[isfk/go-ohos-kit](https://github.com/isfk/go-ohos-kit)
> 同一个 ArkTS 外壳、同样的八块面板，内核是 Go：**4 个 runtime 补丁**，产物 11 MB（这里 2.7 MB）。
> 那边用纯 Go 的 SQLite、内核零 C 依赖，代价是同样的 20k 行 seed 59.3 ms（这里 12.3 ms）——
> 它买到的是"不用编 C"这件事本身。补丁逐条对照、桥读数、体积表见[本仓库的 CONTEXT.md](CONTEXT.md)。
>
> 语言、生态、踩过的坑、什么时候选哪个：[**Go vs Rust，在 HarmonyOS 上**](docs/go-vs-rust.md)。

Go 那份的代价不止补丁：GOROOT 靠 `-overlay` 维护，每升级一次 Go 都要重跑锚点检查。
这个仓库把那层内核换成 Rust，问的是一个具体问题：**那些债还要不要还。**

## 已经量出来的

| | Go（对照组） | Rust（实验组） |
|---|---|---|
| 交叉编译前的准备 | 4 个 runtime 补丁 + overlay 工具 + 每版 Go 重验 | **无**（NDK 自带 cargo 要的 linker） |
| 产物大小 | 11 MB | **2.7 MB**（不含 SQLite 时 788 KB） |
| `TLS_TPREL64` 重定位 | 必须是 0（补丁就是为了它） | **本来就是 0** |
| 内核能否脱离设备测试 | 能（`go test ./core/`） | 能（`cargo test -p rustkit-core`） |
| 数据库 | `modernc.org/sqlite`（纯 Go，零 C） | `rusqlite` + bundled（**编 C**，交叉编译 20 s） |
| SQLite 20k 行 seed | 59.3 ms | **12.3 ms** |
| `remaining()` int 回传 | 0.3 µs | **0.1 µs** |
| `todos()` JSON 回串 | 3.1 µs | **1.3 µs** |
| 300 KB 像素数学 | 475 µs | **362 µs** |

**这些数字怎么读**（两边都适用，也是唯一诚实的读法）：

- 同一台设备、同一时段、两个 app 各跑 **5 次**取中位数。上面的 Go 侧读数是当场重测的，不是
  `../go-ohos-kit` README 里存的那组。
- 绝对读数**跨会话会漂**：同一个 `todos()` 调用，那份 README 记的是 1.5–1.7 µs（早先 4 次采样），
  今天同一份 build 读到 3.0–3.4 µs。int 和 300 KB 那两行两边吻合，JSON 那行差了 2 倍 ——
  所以**能引用的只有同一次会话里的比值**，别跨文档拼数字。
- 300 KB 那一行的「桥份额」是总时间减去核心自报的数学时间，两边噪声都在 ±50%
  （Go 5 次里从 71 跳到 205 µs），只能当量级看，`go` 那份 README 也是这么标注的。

## 为什么它更省事

Go 是带着一整套操作系统进场的：调度器、TLS 约定、`_rt0` 启动流程、auxv 探测。`dlopen`
进来的 `.so` 没有这些，所以 Go 必须假装自己是那个进程，于是有了那四个补丁。

Rust 没有运行时可言：一个 ELF 里的代码，libc 由宿主进程准备好，线程局部变量走 `__thread`。
那四笔债一笔都不用还 —— 逐条对照见 [`CONTEXT.md`](CONTEXT.md) 第二节。

代价落在别的地方，而且只有一处：**数据库要编 C**。`rusqlite` 的 bundled 把 SQLite 编进产物，
`.so` 因此从 788 KB 长到 2.7 MB（仍比 Go 版的 11 MB 小 4 倍）。这笔 C 被关在 `rust/store`
一个 crate 里，所以 `rust/core` 仍然无 FFI —— 和那边把 SQLite 关进 `go/store` 是同一个动作。

## 三层

```text
ArkTS / ArkUI       界面、权限、Kit、surface
    │  import 'librustcore.so'
NAPI 胶水           rust/bridge/csrc/napi_ohos.c —— 基本是 Go 版的原文，只换了前缀
    │  extern "C"
Rust 内核           rust/core（纯 Rust）+ rust/store（唯一编 C 的 crate）
```

那层 C 胶水几乎是 Go 版的原文，这一点是刻意的：**如果"边界"这个说法成立，胶水就不该
分辨得出底下是什么语言。** 实际差别只有一个 `rustkit_set_emit()` 注册函数，因为 Rust
不引用 C 符号（只有 C 引用 Rust）。

## 内核里有什么

五个模块，测试用例是从 `go/core/*_test.go` 一条条搬的 —— 同一张照片、同一段行走、同一个
加速度，两边必须给出同一个数字：

| 模块 | 内容 |
|---|---|
| `todos` | 列表、id 规则（含"重启跳号"那个 off-by-one 的回归测试） |
| `image` | BT.601 luma、8 分桶直方图（余数给最高柱）、8×8 光照网格、4096 色桶不排序取前三 |
| `level` | 1/5 平滑、`atan2` 出 pitch/roll、每秒一次采样率 |
| `geo` | haversine、初始方位角、按精度过滤抖动、速度平滑 |
| `file` | magic bytes 认格式 + 全文件 SHA-256 |

线格式（`rows` + `encode_rows`）也在里面：同一页 2000 行，JSON 与紧凑格式两条路。
`rust/store` 是第六个，也是唯一依赖 C 的那个：打开文件、seed、page、count、reset，4 个用例
（其中一个会关掉再开真实文件，确认行还在）。

## 真机上验过的

`cn.isfk.rustkit` 在设备上装着，八块面板全是 Rust 在供数据：

- 水准仪 50 samples/s，气泡随手机倾斜移动；定位读数随 fix 刷新
- 照片分析：8×8 光照网格 + 直方图 + 调色板，`2.2 ms in Rust`
- 线格式：JSON 173 KB / 2.9 ms 对比紧凑格式 120 KB / 2.9 ms，光过桥 0.2 ms
- SQLite：seed 20k 12.3 ms（store 12.2）· page 2k 5.5 ms（store 1.21），平台 `relationalStore` 对照 262.5 / 9.2 ms
- 相机预览出画面（`1280×720`）、暗色主题两边的观感一致

## 命令

```sh
scripts/check.sh              # fmt + clippy + 测试 + 交叉编译 + 五条断言
scripts/check.sh --no-build   # 没有 SDK 的机器
platform/ohos/build.sh        # 只编 librustcore.so，并放进 app/entry/libs
platform/ohos/run.sh          # 编 + 装 + 跑（要设备 + 一次 DevEco 签名）
```

`check.sh` 的五条断言盯的都是**只在设备上现形**的失败：产物无 `TLS_TPREL64`、
`napi_module_register` 被引用、NEEDED 里有 NAPI 与 hilog 两个系统库、两份 `color.json`
的 token 名集合一致。

一次自检的样子：

```text
== cargo fmt ==       ok
== cargo clippy ==    ok
== cargo test ==      28 core + 4 store + 2 bridge
== colour ramps ==    both ramps declare 15 tokens
== cross build ==     librustcore.so — 2.7M, 24 exports, 0 TPRELs
```

## 目录

```text
rust/core/            纯 Rust：cargo test 在这里过，不认识手机
rust/store/           唯一编 C 的 crate（rusqlite bundled）
rust/bridge/          cdylib：src/lib.rs 是桥，csrc/napi_ohos.c 是 NAPI 那一层
app/                  ArkTS 外壳，从 go-ohos-kit 搬来并改名（不是重新设计）
platform/ohos/        build.sh（交叉编译）、run.sh（编 + 装 + 跑）
scripts/check.sh      fmt、clippy、测试、交叉编译，以及那五条断言
CONTEXT.md            这个仓库从哪来、Go 那四笔债逐条对照、踩过的坑
AGENTS.md             动手前要读的规则和坑
```

## 来源

界面与设计决策来自[对照组](https://github.com/isfk/go-ohos-kit)：那套 ArkTS UI 已经做过一次完整返工并在
真机验过（面板无边框、单一 ink 中性阶、Go 青只给"活着"的东西、分割线内缩且末行不画）。
搬 `app/` 时原样保留，别重新设计 —— 那是这个对照实验的**控制变量**。

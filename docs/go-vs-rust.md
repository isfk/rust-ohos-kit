# Go vs Rust，在 HarmonyOS 上

同一套 ArkTS 外壳、同一个八面板演示、同一块测量表，内核分别用 Go 和 Rust 实现一遍。

- **对照组**：[go-ohos-kit](https://github.com/isfk/go-ohos-kit) — Go 内核
- **实验组**：[rust-ohos-kit](https://github.com/isfk/rust-ohos-kit) — Rust 内核
- 设备 HBN-AL00 / API 26；桥与数据库的读数取自 2026-11-04 同一时段，两个 app 各跑 5 次

> 这份文档在两个仓库里各放一份**完全相同的副本**（`docs/go-vs-rust.md`）。要改就两边一起改；
> `rust-ohos-kit/scripts/check.sh` 在两个仓库并排 checkout 时会比对两份是否一致。

## 一句话结论

**Go 在鸿蒙上要还的那四笔债，是"它自己是运行时"带来的，不是"接进鸿蒙"的代价。**
Rust 不欠那四笔债；它欠的是另一笔：**数据库要编 C**。
两者都能用 —— 取决于你要的是"零 C 依赖"，还是"最小产物 + 最低 FFI 地板"。

## 一、那四笔债是什么

Go 的 `.so` 被 `dlopen` 进来时，它必须假装自己是一个进程：runtime 要调度器、要 TLS 约定、
要读 argv/auxv 完成启动。于是有了改 GOROOT 的四个补丁：

| Go 的补丁 | 为什么需要 | Rust 需要吗 |
|---|---|---|
| `tls_arm64.s` | `g` 寄存器要塞进自己的 TLS | 不需要：没有 `g`，线程局部走 `__thread` |
| `rt0_linux_arm64.s` | 没有进程启动，argv/envp 是空的 | 不需要：没有 `_rt0` |
| `os_linux.go` | 没人给 auxv，page size 得自己用 `mincore` 探 | 不需要：`sysconf(_SC_PAGESIZE)` 宿主已备好 |
| `gokit_ohos.go` | 要宿主 save/load 那个寄存器 | 不需要 |

代价不只是补丁本身：`-overlay` 要自己维护，**每升级一次 Go 都要重跑锚点检查**。
一次性成本是小的，持续成本才是真的。

## 二、数字

标注了哪些可以引用 —— 这一栏比数字本身重要，理由见文末。

| | Go | Rust | 可引用性 |
|---|---|---|---|
| runtime 补丁 | 4 | **0** | ✅ |
| 产物 | 11 MB | **2.7 MB**（不含 SQLite 时 788 KB） | ✅ |
| 依赖的系统库 | libace_napi / libhilog / libc | 同左，逐个一致 | ✅ |
| `TLS_TPREL64` 重定位 | 必须为 0（补丁就为它） | **本来就是 0** | ✅ |
| 桥：空调用（`remaining()`） | 0.28–0.3 µs | **0.1 µs** | ✅ 两边各 5 次，5/5 一致 |
| 桥：JSON 回串（`todos()`） | 3.1 µs | **1.3 µs** | ⚠️ 只可同会话比；Go 侧历史记录是 1.5–1.7 |
| 像素数学（300 KB RGBA） | 475 µs | **362 µs** | ✅ 稳定，但与语言无关 |
| 桥：300 KB 拷贝 | 71–205 µs | 20–32 µs | ❌ 两边噪声 ±50%，只能看方向 |
| SQLite seed 20k | 59.3 ms | **12.3 ms** | ✅ 赢家是 C 的 SQLite，不是 Rust |
| 线格式 2000 行 | 173 KB / 2.9 ms | 120 KB / 2.9 ms | ✅ 两边一样 —— 换语言不改格式 |

**唯一"语言真的更快"的地方是桥的地板**：cgo 每次调用要付运行时状态切换，`extern "C"` 只是跳转，
差约 0.2 µs。其余差距都能归到算法写法，或者归到"C 实现 vs 纯 Go 实现"。

## 三、语言层面的真实差异

| | Go | Rust |
|---|---|---|
| FFI 地板 | cgo ≈ 0.28 µs/次 | ≈ 0.1 µs/次 |
| GC | 有（**手机上是否掉帧，没测**） | 无 |
| 并发手感 | 一个 goroutine 就够推事件 | 线程 + channel + `Arc<AtomicBool>`，约 40 行 |
| 边界安全 | `C.CString` + 手动 `free` | `CString::into_raw` + 显式 `rustkit_free`；panic 过 FFI 直接 abort，所以处处用 `Result` 返回错误 JSON 而不是 `unwrap` |
| 构建矩阵 | 4 补丁 + overlay 工具 + 锚点检查 | 一条 `cargo build --target aarch64-unknown-linux-ohos`（NDK 自带 cargo 要的 linker） |
| 生态 | `modernc.org/sqlite` 这类纯 Go 实现，让"零 C 依赖"成立 | 生态更广，但常用库（sqlite、crypto 的某些部分）都要编 C |
| 脱离设备测试 | `go test ./core/` | `cargo test -p rustkit-core`，同样不需要手机 |

## 四、这次踩的坑，按语言归类

**Go 侧（历史）**

- 四个 runtime 补丁，以及它们的**锚点会随 Go 升级移动** —— 编译不会报错，这是最贵的部分。
- `.zshrc` 里的 `CGO_ENABLED=0` 让 cgo 驱动变成一个 stub，第一次查询才报错。

**Rust 侧（这次现学的）**

- **静态库被整个丢掉**：C 胶水没有任何 Rust 代码引用，链接器一个成员都不拉，连
  `__attribute__((constructor))` 里的模块注册一起消失。`.so` 编得出来、导出都在，
  ArkTS 拿到的是一个空模块 —— **完全静默**。
- **`.so` 不链 NAPI 运行时**：`napi_module_register` 不在 app 进程里，链接器不吭声，
  装机后 `dlopen` 才报 `symbol not found`，进程一闪就退。

这两条现在都是 `check.sh` 里的断言。Go 的四个补丁是同一性质的教训：
**编译通过 ≠ 能跑。**

## 五、移植成本（诚实版）

- **机械的部分很小**：内核约 1500 行（含测试），五个模块加工线格式；ArkTS 外壳改名 14 个文件。
  真干下来是几小时。
- **贵的是验证**：大部分时间花在真机、日志、截图、以及改口上。
  也就是说 —— **移植不难，难的是知道它到底有没有跑起来。**
- 测试用例能一条条从 Go 搬过来（同一张照片、同一段行走必须给出同一个数字），
  这是这次能快速收敛的最大原因。

## 六、什么时候选哪个

| 你的约束 | 选 |
|---|---|
| 内核要零 C 依赖、工具链越简单越好、团队熟 Go | **Go**（接受 4 个补丁 + 升级维护） |
| 产物要小、不要 GC、FFI 调用频繁 | **Rust** |
| 需要一个成熟的纯 X 实现（如 SQLite） | **Go**（`modernc` 那一类） |
| 需要大量现成 crate、不介意编 C | **Rust** |
| 只想要"逻辑搬出 ArkTS、能在 Mac 上测" | 两个都行，看人 |

## 七、没测到的（空白和结论一样重要）

- **长时间运行**：内存、电池、热。全部是短会话。
- **GC 对 UI 帧的影响**：Go 有 GC、Rust 没有，这在手机上可能是真实差异，我们没测帧时间。
- **Rust 的 async 运行时**：用的是 `std::thread`，没上 tokio。
- **iOS / 桌面复用**：这条线完全没碰。

## 八、最值钱的一句

**语言不是变量，边界才是。**

桥 0.28 µs 还是 0.1 µs，对一次 16 ms 的帧都是噪声。真正决定这个架构成不成立的是
**边界划在哪里**：纯核心（`rust/core` / `go/core`）、编 C 的那一块单独关起来
（`rust/store` / `go/store`）、胶水不认识底下是什么语言。
换语言改不动这个结论，只会改变你付哪一笔账。

## 附：怎么复现

两个仓库里各自：

```sh
scripts/check.sh        # 测试 + 交叉编译 + 只在设备上现形的几条断言
platform/ohos/run.sh    # 编 + 装 + 跑（要设备；首次需要一次 DevEco 签名）
```

桥的读数在 app 里点 `Measure`（Bridge 卡片）。**每次点击内部已经是两万次调用的平均**，
所以缺的是运行间方差：点 5 次取中位数。同样的数字也会进日志
（`gokit-bench bridge:` / `rustkit-bench bridge:`），用 `hdc shell hilog -x | grep bench` 捞。

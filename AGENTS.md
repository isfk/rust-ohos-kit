# AGENTS.md

给在这个仓库里干活的 agent（也给我自己）。`README.md` 讲"是什么、为什么"，
`CONTEXT.md` 讲"从哪来、还差什么"，这里只讲**动手时要知道的事**。

## 心智模型

三层，边界别糊：

```text
ArkTS / ArkUI   ← 界面、权限、平台 Kit、生命周期、surface
    │  import 'librustcore.so'
NAPI 胶水       ← rust/bridge/csrc/napi_ohos.c：只搬字节，不含业务
    │  extern "C"
Rust 内核       ← rust/core：状态、算术、逻辑；线程随便开
```

## 五条铁律

1. **`rust/core` 必须保持纯 Rust**：不 FFI、不 import 任何 HarmonyOS 相关的 crate、不认识
   `filesDir`。它要能在 Mac 上 `cargo test` 跑通 —— 这是唯一不用设备就能验证的那部分。
   路径、句柄一律从 ArkTS 传进来（参数或 `&[u8]` / `fd`）。

   数据库放在 **`rust/store`**，不在 `rust/core`：`rusqlite` 的 `bundled` 会把 SQLite 的 C
   编进产物，那是"不纯"的唯一边界。关在一个独立 crate 里，core 就还能保持无 FFI。
   这和 Go 那边把 SQLite 关进 `go/store` 是同一个动作。`cargo test -p rustkit-store`
   仍然能在 Mac 上跑（bundled 对宿主平台一样能编）。

   **待办清单也住在这个文件里**（`filesDir/rustkit.db`，`todos` 表；`rows` 表仍是 20k
   行对比用）。清单的形状归 core（`rust/core/src/todos.rs`：id 规则、空标题不算一行、
   存失败要回滚），表的形状归 store，**两边在本文件里碰头** —— `to_store` / `from_store`
   就是全部，和 Go 那边 `todoPersister` 的位置一样。别让 `rust/store` 依赖 core：
   数据库能把业务的规则掰弯，那就不是"一个文件一个写者"，是一个文件一个老板。

2. **平台的东西留在平台侧**：解码（HEIF/JPEG 归系统解码器）、权限、surface、Kit 全在 ArkTS；
   内核只收已经规整的数据（RGBA 字节、fd、JSON）。**别在内核里塞 `image` crate 去解相机照片** ——
   相机默认存 HEIF。

3. **桥只搬字节**。`rust/bridge/src/lib.rs` 只做：取参数 → 调 core → 序列化 → 交指针。
   往那里塞业务，就是把可测的东西变不可测。`napi_ohos.c` 更狠：**它不该知道底下是什么语言**。
   这也是本仓库的论点 —— 那个文件几乎是 Go 版的原文，只换了前缀。

4. **ArkTS 只用状态管理 V2**：`@ComponentV2` + `@Local`/`@Param`/`@Event`/`@Monitor`；
   跨组件共享用 `@ObservedV2` + `@Trace` + `AppStorageV2`。
   V1 的 `@State`/`@Prop`/`@Link`/`@Watch`/`@StorageLink` **不能和 V2 混用**，一个也别加。

5. **`.cargo/config.toml` 里的 linker 别乱改**。NDK 自带
   `aarch64-unknown-linux-ohos-clang`，那个包装器已经把 `-target aarch64-linux-ohos
   --sysroot=<ndk>/native/sysroot` 写死了 —— 交叉编译能一句话过，靠的就是它。
   改它之前先看 `platform/ohos/build.sh` 有没有把 NDK 放上 PATH。

## 命令

| 想做什么 | 命令 | 要设备？ |
|---|---|---|
| 内核逻辑 | `cargo test -p rustkit-core` | 不要 |
| 数据库逻辑 | `cargo test -p rustkit-store` | 不要 |
| 全套自检 | `scripts/check.sh` | 不要 |
| 无 SDK 机器 | `scripts/check.sh --no-build` | 不要 |
| 只编 `.so` | `platform/ohos/build.sh` | 不要 |
| 编 + 装 + 跑 | `platform/ohos/run.sh` | **要** |
| 内核日志 | `hdc shell hilog -T rustkit` | 要 |

`hdc` 在 `$DEVECO_SDK_HOME/default/openharmony/toolchains/hdc`（默认
`~/command-line-tools/sdk`）。

## 改完怎么验：不许「应该没问题」

| 改了什么 | 拿什么结账 |
|---|---|
| `rust/core` | `cargo test -p rustkit-core` 输出（新逻辑必须有新用例） |
| 桥 / 新导出 | `scripts/check.sh`（含交叉编译与五条断言）+ 真机调用一次 |
| `napi_ohos.c` | `check.sh` 的 NEEDED 断言 + 真机启动一次（`hilog -x` 里不能有 `relocating`） |
| ArkTS / UI | `platform/ohos/run.sh` + 真机截图；`hilog` 佐证 |

- **截图是证据，但要看清**：`hdc shell snapshot_display -f /data/local/tmp/x.jpeg` +
  `hdc file recv`。别凭想象描述画面。
- **相机预览在 `snapshot_display` 里能抓到**（本机实测：截图里是真实画面）。所以"黑方块"
  就是真的黑（镜头对着暗处或挡住），不能当成截图限制来解释 —— 先看状态行（`1280×720`）和
  按钮是否变成 Stop，再看画面。
- **别用自动化工具替用户验手势**：`uitest uiInput drag/swipe` 对拖拽/捏合测不准。
- **判断错了要改口**：先复现、再归因。怀疑某个 bug 之后要用实验把两种可能分开，
  不能把"我看到的顺序"当因果。

## 陷阱清单

- **stdout/stderr 在鸿蒙里没有人接**（本仓库踩过）。`eprintln!` 和 panic 的消息走 fd 1/2，
  而 app 进程的这两个描述符给的是空气 —— 表现是"日志一片空白"，而不是报错。
  `rust/bridge/csrc/log_ohos.c` 用一个构造函数把 1/2 接进 pipe、再由一条 C 线程写 hilog：
  `hdc shell hilog -T rustkit` 就是它的出口。`check.sh` 断言产物里引用 `dup2`，
  因为这个对象被丢掉时同样是静默的（和上面那条一个失败类别）。
  写日志的方式就是 `eprintln!`，**不要在 Rust 里直接引 C 的 hilog 函数** —— 那会让
  "Rust 不引用任何 C 符号"这条前提不再成立（正是它让 `build.rs` 的写法有了依据）。
  **但这是一根进程级的重定向**：进程里任何 C/C++ 库写 fd 2 的东西都会挂上 `rustkit`
  这个 tag。实例：真机上分析一张 JPEG 时 hilog 出现 `rustkit: current ifd->count 13`
  —— 那是系统解码器的 EXIF 调试输出（`grep -rn ifd rust/` 是空的），不是核在说话。
  读日志时先想一想“这句像人写的吗”。
- **`stop()` 不写盘**。每次增/改/删都在同一次调用里提交（`List::change_durably` +
  `save_todos`），所以没有"退出时补一次保存"这条路径；加回来只会在最不适合的时刻
  多一条没人测过的分支。
- **借来的 fd 不要用 `Read` 读**。fd 的偏移量属于 open file description，不是属于某个
  句柄 —— ArkTS 拿到 fd 之后，`rustkit_inspect` 读一遍、紧接着 `image.createImageSource`
  再用同一个 fd。`File` 的 `Read` 会把它推到 EOF，解码器就什么也读不到。`rustkit_inspect`
  因此走 `pread`（`FileExt::read_at`，包成 `FromStart`），**偏移量一点不动**，也不需要
  `dup`。想看 `dup` 有用没用的实测：`dup` 得到的是**同一个** description，偏移量共享 ——
  Go 那边 `InspectFD` 里的 `syscall.Dup` 解决的是"有权关掉这个句柄"，不是偏移量。
  **非 seekable 的 fd（管道、套接字）会 `ESPIPE`，这里就让它失败**，不是省事而是语义：
  算整份 sha256 必须读完，读完流就没了 —— 拒绝比"hash 成功、解码器拿到空"诚实，而且
  一个字节都没消费。（"这个分支没法测" 是错的：`UnixStream::pair()` 造一对就能测。）
- **静态库会被整个丢掉**（本仓库踩过）。`cc` 编出来的 `.a`，如果 Rust 侧不引用其中任何符号，
  链接器一个成员都不拉 —— 连 `__attribute__((constructor))` 里的 `napi_module_register` 也消失。
  表现完全静默：`.so` 有了、导出都在、ArkTS 拿到一个空模块。
  所以 `build.rs` 用 `compile_intermediates()` 拿 `.o` 直接交给链接器，`check.sh` 里有断言钉住。
- **`.so` 必须链接 `libace_napi.z.so` 和 `libhilog_ndk.z.so`**（本仓库踩过，代价是一次装机）。
  `napi_module_register` 和 `OH_LOG_Print` **不是**宿主进程提供的：不链接它们，链接器照过，
  装机后 dlopen 才报 `Error relocating ...: napi_module_register: symbol not found`，
  app 一闪就退。参数在 `.cargo/config.toml` 的 `rustflags`，`scripts/check.sh` 里有 NEEDED 断言
  （Go 版在 `go/cgo_linux.go` 写的是同一件事：`#cgo LDFLAGS: -lace_napi.z -lhilog_ndk.z`）。
- **`Option<extern "C" fn(...)>` 和可空函数指针同 ABI**，所以 `rustkit_set_emit(NULL)` 合法。
  别改成 `fn()` + 一个布尔。
- **字符串的所有权只有两条路**：Rust 产出的 `char *` 必须由 `rustkit_free` 回收（Rust 分配，
  Rust 释放）；C 侧的 `toC` 用 `malloc`，由 C 自己 `free`。混着来会在某个设备上炸。
- **事件字符串只借用一次**：`emit()` 的指针在回调返回后立刻失效，C 侧必须先 `strdup` 再上
  threadsafe function。
- **panic 过 FFI 直接 abort**。桥里不要 `unwrap()` 上去；锁中毒用
  `with_kernel` 返回 `None`，让调用方回答一个错误 JSON，而不是带走进程。
- **`cc` 的环境变量名**：`CC_aarch64_unknown_linux_ohos`（下划线）和
  `CC_aarch64-unknown-linux-ohos`（连字符）两种拼法它都认，但**别只写一种**就以为配好了。
  用 `cargo build -v` 看实际命令行。
- **`target_env="ohos"`，不是 `target_os`**：`aarch64-unknown-linux-ohos` 的 `target_os` 是
  `linux`。要按平台区分，用 `CARGO_CFG_TARGET_ENV == "ohos"`。
- **主机侧 `cdylib` 能编也会链**：C 文件被 `__has_include` 挡掉，所以 Mac 上 `cargo build` 出一个
  没有 NAPI 的 `.so`/`.dylib`。别因此以为宿主构建就能当模块用。
- **`default-members` 只有 core**：`cargo build` / `cargo test` 不该把桥拖进来。桥永远带
  `--target aarch64-unknown-linux-ohos` 出现。
- **跨编译不需要 `rustup target add`** 之外的准备（本机已有该 target）。如果没有：
  `rustup target add aarch64-unknown-linux-ohos`。

## 已被证伪 / 不用再论证

- ~~「Rust 在 OHOS 上要打 runtime 补丁」~~：不需要。`cargo build --target
  aarch64-unknown-linux-ohos` 一次通过，产物 0 个 `TLS_TPREL64` 重定位。
  Go 版那四个补丁（`tls_arm64.s`、`rt0_linux_arm64.s`、`os_linux.go`、`gokit_ohos.go`）
  全是 Go runtime 自己的债，逐条对照见 `CONTEXT.md` 第二节。
- ~~「换个语言能变快」~~：Go 版已经测过，桥的往返 0.28 µs，换语言改不了这个数字。
  **这里该比的是体积、启动成本、维护面**，不是"Rust 比 Go 快"。
- ~~「Rust 上 SQLite 做不了/更贵」~~：**已证伪**。`rusqlite` + `bundled` 交叉编译到
  `aarch64-unknown-linux-ohos` 20 秒通过，20k 行 seed 12.3 ms（Go 的纯 Go 驱动 60.2 ms）。
  代价是纯度（C 进了产物、`.so` 从 788 KB 到 2.7 MB），不是能力。
- ~~「rusqlite 就是纯 Rust」~~：不是。`bundled` 会把 SQLite 的 C 编进 `.so`，所以它只能待在
  `rust/store`，不能进 `rust/core`。

## 别提交、别碰

- `app/build-profile.json5`：本机证书路径 + 密码占位，**不入库**（`.gitignore` 已挡）。
  入库的是 `build-profile.example.json5`。
- `target/`、`*.so`、`*.a`、`*.hap`、`.cache/`：都在 `.gitignore` 里。
- 删文件、改包名（`cn.isfk.rustkit`）、卸载设备上的 App：先问人。

## 约定

- **ArkTS 外壳两边一起改**。`app/entry/src/main/ets` 是这个对照实验的控制变量：能跑在
  Rust 内核上的东西，也必须在 Go 内核上跑。四个“装着行为”的文件（`RustCore.ts`、
  `events.ets`、`TodoComposer.ets`、`Index.ets`）在归一化掉 `rustcore`/`gocore` 和语言名之后
  必须逐字节相同 —— `scripts/check.sh` 会拿兄弟仓库对，漂了就失败。
  先在这里加修正、再回填是允许的（实际上真就是这么干的），但**不能只停在一半**。
- **语言**：代码注释和 commit message 用英文；README / AGENTS.md / CONTEXT.md 用中文。
- **注释写「为什么」**：桥接代码最容易变成「注释复述代码」。
- **`cargo fmt` 必须跑，`clippy -- -D warnings` 必须干净**（`scripts/check.sh` 会查）。

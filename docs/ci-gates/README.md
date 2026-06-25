# CI Gates 设计（长期目标）

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文只描述
> gate 设计；若与本地仓库迁移指令冲突，以迁移指令为准。

> 本分支 `codex/goal-ci-gate-design` 基线为 `origin/codex/core-product-integration`
> （commit `fb4c3a7`）。本文档只做 **gate 设计**，不直接改 GitHub Actions
> 或脚本。所有命令清单均以基线代码树为准核实，不臆造未来命令。

## 范围与硬约束

- **只允许新增/修改** `docs/ci-gates/**`。
- **禁止修改**：`.github/**`、`scripts/**`、`crates/**`、`bindings/**`、
  `protocol/**`、`Cargo.*`、`include/**`、`tools/**`、`native/**`。
- 本设计是对 *现有可运行命令* 的分层与调度建议；落地时由后续分支分别
  改 workflow / 脚本，每轮一个 commit。
- 现有 workflow `.github/workflows/core.yml` 与 `scripts/*.sh` **保持原样**，
  本文仅引用其行为，不重写。

## 设计原则

1. **失败闭环（fail-closed）**：任何依赖本机 SDK / target 的 gate，在环境缺失时
   必须以非零退出码失败，而不是静默跳过或误报通过。
2. **平台分层**：Core（host-agnostic）→ iOS → OHOS/Harmony → Android →
   集成 lane，下层依赖上层通过。
3. **PR 轻、nightly 重**：PR 只跑 host-agnostic + 可在 CI 镜像内安装的 iOS
   gate；OHOS/Harmony NAPI、Android JNI、HAP 真机归 nightly / 自托管。
4. **不越界声明**：Core-side smoke 不得冒充 App/真机 proof（参见
   `docs/ROLLING_INTEGRATION.md` 既有规则）。

---

## 1. 当前可运行命令清单

> 以下命令在基线 `fb4c3a7` 上逐一核实来源。`<mode>` 见 `tools/reader-cli/src/main.rs`。

### 1.1 Rust workspace（host-agnostic）

| 命令 | 来源 | 说明 |
| --- | --- | --- |
| `cargo fmt --check` | `scripts/check-local.sh:6` | 格式校验 |
| `cargo test --workspace` | `scripts/check-local.sh:7` | 全 workspace 测试 |
| `cargo build --workspace` | `scripts/build-local.sh:6` | 全 workspace 构建 |
| `cargo build -p reader-ffi --release` | `scripts/build-local.sh:7`、`scripts/ffi-smoke.sh:6` | FFI staticlib（release） |
| `cargo build -p reader-ffi` | `scripts/check-ios-swift-wrapper.sh:19` | FFI staticlib（debug，供 host swift smoke 链接） |
| `cargo run -p reader-cli` | `scripts/build-local.sh:8` | 默认 `--info` |
| `cargo run -p reader-cli -- --info` | `tools/reader-cli/src/main.rs:60` | `core.info` |
| `cargo run -p reader-cli -- --ping` | `tools/reader-cli/src/main.rs:65` | `runtime.ping` |
| `cargo run -p reader-cli -- --host-smoke` | `tools/reader-cli/src/main.rs:83` | `host.request` → `host.complete` 回路 |
| `cargo run -p reader-cli -- --conformance` | `.github/workflows/core.yml:30` | 协议一致性（`protocol/fixtures/conformance/**`） |
| `cargo run -p reader-cli -- --json '<cmd>'` | `tools/reader-cli/src/main.rs:70` | 单条 JSON 命令 |
| `cargo run -p reader-cli -- --stdin` | `tools/reader-cli/src/main.rs:74` | 从 stdin 读 JSON 命令 |
| `cargo run -p reader-cli -- --fixture-vertical <path>` | `tools/reader-cli/src/main.rs:96` | 远程阅读纵切（搜索→详情→目录→正文→进度） |
| `cargo run -p reader-cli -- --config-json '<cfg>' -- <mode>` | `tools/reader-cli/src/main.rs:131` | 自定义 runtime config |

纵切 fixture 样本：`tests/fixtures/remote_source/basic_source.json`。

### 1.2 单 crate 隔离测试（用于定位 / 并行 agent 解耦）

| 命令 | 备注 |
| --- | --- |
| `cargo test -p reader-rule` | 仅依赖 `reader-domain`，可独立构建 |
| `cargo test -p reader-js` | 依赖 `reader-contract`（协议层常被并发修改） |
| `cargo test -p reader-cli` | 含 `tests/fixture_vertical.rs` |

### 1.3 脚本封装（`scripts/*.sh`，保持原样）

| 脚本 | 等价展开 | 环境要求 |
| --- | --- | --- |
| `./scripts/check-local.sh` | `cargo fmt --check` + `cargo test --workspace` | Rust |
| `./scripts/build-local.sh` | `cargo build --workspace` + `cargo build -p reader-ffi --release` + `cargo run -p reader-cli` + `./scripts/ffi-smoke.sh` | Rust + cc/c++ |
| `./scripts/ffi-smoke.sh` | release staticlib + `cc`/`c++` 编译并运行 C 与 C++ smoke（对 `include/reader_core.h` + `target/release/libreader_core.a`） | Rust + cc + c++ |
| `./scripts/check-ios-swift-wrapper.sh` | `build-ios-xcframework.sh` + wrapper typecheck + host `reader-ffi` + 编译运行 Swift smoke（`core.info`/`runtime.ping`） | macOS + Xcode + iOS targets |
| `./scripts/build-ios-xcframework.sh` | 交叉编译 reader-ffi → `xcodebuild -create-xcframework` + Swift typecheck | macOS + Xcode + iOS targets |
| `./scripts/build-ohos.sh` | 交叉编译 `aarch64-unknown-linux-ohos` reader-ffi staticlib | OHOS target；`OHOS_SDK_HOME` 可选（提供 sysroot/llvm/libclang） |
| `./scripts/build-harmony-napi.sh` | `build-ohos.sh` + cmake/ninja 构建 `libreader_core_napi.so` | `OHOS_SDK_HOME` + OHOS native 工具链 |
| `./scripts/integration-queue.sh <branch> <base> <src>...` | worktree 合并编排 + 本地 gate（`RUN_OHOS=1`/`RUN_NAPI=1`/`PUSH=1`） | Rust；OHOS gate 需 SDK |

### 1.4 协议一致性 fixture（`--conformance` 驱动）

`protocol/fixtures/conformance/**`：`cancel/`、`host/`、`configs/`、`commands/`
各含 `valid-*` / `invalid-*` 用例。`--conformance` 退出码 2 表示有用例失败。

---

## 2. 按平台分层的 gate

下层 gate 默认依赖上层全部通过。每层标注 *本机 SDK 依赖* 与 *失败闭环点*。

### Layer 0 — Core（host-agnostic，任何 Rust + cc/c++ 环境）

| Gate | 命令 | 失败闭环 |
| --- | --- | --- |
| 格式 | `cargo fmt --check` | 非零退出 |
| 测试 | `cargo test --workspace` | 非零退出 |
| 构建 | `cargo build --workspace` | 非零退出 |
| FFI staticlib | `cargo build -p reader-ffi --release` | 非零退出 |
| 协议一致性 | `cargo run -p reader-cli -- --conformance` | 有失败用例 → 退出码 2 |
| 远程纵切 | `cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json` | 非零退出 |
| C/C++ ABI smoke | `./scripts/ffi-smoke.sh` | 缺 cc/c++ → 编译失败 |

> `./scripts/check-local.sh` + `./scripts/build-local.sh` 是 Layer 0 的现成伞 gate。

### Layer 1 — iOS（macOS + Xcode）

| Gate | 命令 | 本机 SDK 依赖 |
| --- | --- | --- |
| XCFramework 构建 | `./scripts/build-ios-xcframework.sh` | `xcodebuild`；rust targets `aarch64-apple-ios` + `aarch64-apple-ios-sim` |
| Swift wrapper smoke | `./scripts/check-ios-swift-wrapper.sh` | 同上 + `iphonesimulator` SDK |

**边界声明**：此层仅证明 Core-side wrapper compile/link/runtime。**不等于**
App/真机/URLSession/WebView 集成 proof（`bindings/ios/README.md`、
`docs/ROLLING_INTEGRATION.md` 均已声明）。

### Layer 2 — HarmonyOS / OHOS

| Gate | 命令 | 本机 SDK 依赖 |
| --- | --- | --- |
| OHOS 交叉 staticlib | `./scripts/build-ohos.sh` | rust target `aarch64-unknown-linux-ohos`；`OHOS_SDK_HOME`（sysroot + llvm）；`LIBCLANG_PATH`（bindgen） |
| Harmony NAPI `.so` | `./scripts/build-harmony-napi.sh` | `OHOS_SDK_HOME` + OHOS native cmake toolchain + 自带 cmake/ninja |

**边界声明**：基线 `bindings/harmony/` 只有 `native/CMakeLists.txt` +
`reader_napi.cpp`，**无** README/oh-package/sdk 测试/HAP。因此 NAPI gate
止步于 `.so` 产物；**不得**据此声明真机/ArkTS/HAP parity。

### Layer 3 — Android

基线 `bindings/android/` 仅有 `.gitkeep`：**无 JNI shim、无脚本、无 gate**。

| 状态 | 处理 |
| --- | --- |
| 当前 | 无可运行 Android gate |
| 失败闭环要求 | 任何 "Android gate" 在 shim/脚本落地前必须 **fail-closed**（跳过即失败，而非静默通过） |

### Layer 4 — 集成 lane（`scripts/integration-queue.sh`）

编排 worktree 合并 + Layer 0 gate；`RUN_OHOS=1`/`RUN_NAPI=1` 触发 Layer 2。
lane 划分见 `docs/ROLLING_INTEGRATION.md`（core-foundation / core-product /
android / ios / harmony）。

---

## 3. 必须失败闭环的环境缺失项

> 这些是 *设计要求*。基线脚本中已 fail-closed 的标注「已闭环」；未闭环的标注
> 「待落地（需改脚本，超出本分支范围）」。

| 项 | 所在 gate | 现状 |
| --- | --- | --- |
| 缺 `aarch64-apple-ios`/`aarch64-apple-ios-sim` rust target | iOS | **已闭环**：`build-ios-xcframework.sh:11-21` 检测并 `exit 1` |
| 缺 `xcodebuild` | iOS | **已闭环**：`build-ios-xcframework.sh:23-26` |
| 缺 `aarch64-unknown-linux-ohos` rust target | OHOS | **已闭环**：`build-ohos.sh:8-12` |
| `OHOS_SDK_HOME` 未设 | Harmony NAPI | **已闭环**：`build-harmony-napi.sh:6-10` |
| OHOS native 工具链/cmake/ninja 缺失 | Harmony NAPI | **已闭环**：`build-harmony-napi.sh:17-22` |
| OHOS 交叉缺 libclang（bindgen） | OHOS | **部分**：`build-ohos.sh:42` 设 `LIBCLANG_PATH` 默认值，但未在缺失时显式 `exit`；bindgen 自身会失败，属间接闭环 |
| Android JNI gate 不存在 | Android | **待落地**：需新增 shim + 脚本；落地前任何 Android gate 必须显式 fail-closed |
| Harmony HAP/真机 gate 不存在 | Harmony | **待落地**：`.so` 产物不得冒充真机 proof（规则层已约束，gate 层需显式拒绝） |
| iOS App/URLSession/WebView gate 不存在 | iOS | **待落地**：wrapper smoke 不得冒充 App proof |

**失败闭环原则**：环境缺失 → 非零退出码 + 明确 stderr 提示，**禁止**静默跳过
或降级为"通过"。

---

## 4. PR / Nightly 调度建议

### 4.1 适合 PR（快、host-agnostic 或 CI 镜像内可装）

| 命令 | 备注 |
| --- | --- |
| `cargo fmt --check` | Layer 0 |
| `cargo test --workspace` | Layer 0 |
| `cargo build --workspace` | Layer 0 |
| `cargo build -p reader-ffi --release` | Layer 0 |
| `cargo run -p reader-cli -- --conformance` | Layer 0 |
| `cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json` | Layer 0 |
| `./scripts/ffi-smoke.sh` | Layer 0，需 cc/c++（CI 镜像自带） |
| `./scripts/check-local.sh` | Layer 0 伞 |
| `./scripts/build-local.sh` | Layer 0 伞 |
| `./scripts/check-ios-swift-wrapper.sh` | Layer 1，macOS runner + job 内 `rustup target add`（现行 `core.yml` 已如此） |
| `./scripts/build-ios-xcframework.sh` | Layer 1，同上 |

> 现行 `.github/workflows/core.yml`（macos-15）已覆盖：`check-local.sh`、
> `build-local.sh`、`--conformance`、`check-ios-swift-wrapper.sh`。本设计不改动它。

### 4.2 适合 Nightly（慢、需本机 SDK / 自托管 / 交叉）

| 命令 | 备注 |
| --- | --- |
| `./scripts/build-ohos.sh` | Layer 2，需 OHOS target + SDK + libclang |
| `./scripts/build-harmony-napi.sh` | Layer 2，需 `OHOS_SDK_HOME` + native 工具链 |
| `./scripts/integration-queue.sh ...`（全 lane，`RUN_OHOS=1 RUN_NAPI=1`） | Layer 4，完整集成 |
| Android JNI gate（待落地） | Layer 3，需 Android NDK |
| Harmony HAP / 真机 gate（待落地） | Layer 2，需 DevEco/HAP 签名/真机 |
| iOS App/URLSession/WebView gate（待落地） | Layer 1，需 host 仓库协同 |

### 4.3 划分依据

- **PR**：反馈快、无外部 SDK、能在 GitHub-hosted macOS runner 内完成。
- **Nightly**：依赖本机 SDK / 自托管 runner / 真机，或耗时长的全 lane 集成。
- **禁止**：把 nightly-only gate 放进 PR 路径会导致 PR 因环境缺失而误红。

---

## 5. 依赖本机 SDK 的命令

| 命令 | 依赖的本机 SDK / 工具 | 安装/配置方式 |
| --- | --- | --- |
| `./scripts/build-ios-xcframework.sh` | Xcode（`xcodebuild`）、`iphonesimulator` SDK | macOS + Xcode |
| `./scripts/build-ios-xcframework.sh` | rust targets `aarch64-apple-ios`、`aarch64-apple-ios-sim` | `rustup target add ...`（可在 job 内装） |
| `./scripts/check-ios-swift-wrapper.sh` | 同上 + `swiftc` | 随 Xcode 提供 |
| `./scripts/build-ohos.sh` | rust target `aarch64-unknown-linux-ohos` | `rustup target add aarch64-unknown-linux-ohos` |
| `./scripts/build-ohos.sh` | OHOS SDK（sysroot + llvm/clang） | 设 `OHOS_SDK_HOME` |
| `./scripts/build-ohos.sh` | libclang（rquickjs-sys bindgen） | 设 `LIBCLANG_PATH` |
| `./scripts/build-harmony-napi.sh` | OHOS SDK native 工具链（`ohos.toolchain.cmake`、自带 `cmake`/`ninja`） | 设 `OHOS_SDK_HOME` |
| `./scripts/ffi-smoke.sh` | host C/C++ 编译器（`cc`/`c++`） | CI 镜像自带；非"SDK"但属本机工具依赖 |

> Android NDK / DevEco / HAP 签名工具在基线尚无对应脚本，列入「待落地」。

---

## 6. 落地路线（供后续分支，不在本分支执行）

1. **PR workflow 增强**（后续分支改 `.github/workflows/*`）：显式分层 job，
   Layer 0 必跑、Layer 1 在 macOS job 跑、Layer 2/3 用 `if: nightly` 守门。
2. **fail-closed 守门**（后续分支改 `scripts/*`）：为「待落地」项加显式
   `exit 1` + stderr 提示；Android/HAP gate 落地前以 fail-closed 占位。
3. **nightly workflow**（后续分支新增）：跑 OHOS/NAPI/全 lane 集成。
4. 每一步单独一个 commit，遵守本分支的硬约束边界。

---

*基线：`origin/codex/core-product-integration` @ `fb4c3a7`。本文档为设计文档，
不修改任何 workflow 或脚本。*

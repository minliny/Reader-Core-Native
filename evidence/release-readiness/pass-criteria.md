# 通过标准与 release gate 定义

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文为历史 gate
> 定义；当前发布标准必须补充三端共用同一 Rust Core 的可运行证据。

> 区分两类 gate：**Core-side smoke gate**（Core 产物+host smoke）与 **App/device gate**
> （真机/模拟器 App）。smoke 通过不得记为 App/device 通过。

## Core-side smoke release gate（V1 Core）

发布 Core-side smoke 版本需以下全部绿（macOS host 执行）：

1. `cargo fmt --check` —— 格式 gate。
2. `cargo test --workspace` —— 全 workspace 测试 gate（round-01：92 passed / 0 failed）。
3. `cargo run -p reader-cli -- --conformance` —— 协议 conformance gate（round-01：18/18）。
4. `cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json`
   —— remote.reading.v1 垂直 smoke gate（round-01：9 步全通，含 JS-unsupported 结构化错误）。
5. `cargo run -p reader-cli`（默认 `--info`）—— ABI smoke gate（abiVersion=1 + capabilities）。
6. `./scripts/ffi-smoke.sh` —— host FFI ABI smoke gate（C/C++ 链接 `libreader_core.a`，pong:true）。
7. `./scripts/check-ios-swift-wrapper.sh` —— Core-side iOS 产物 + Swift wrapper 编译/链接/host sim runtime smoke gate。
8. `./scripts/build-harmony-napi.sh` —— Core-side OHOS 静态库 + NAPI `.so` 构建 gate。

**round-01：1–8 全绿。** Core-side smoke release gate 满足。

## App/device release gate（未满足）

App/device 发布需以下各项，**当前均未验证**：

- iOS：iOS App 工程构建、模拟器/真机 App 启动、host adapter 接入、WebView 登录/Cookie、
  Keychain、文件沙箱、后台任务、签名分发。
- HarmonyOS：HAP 打包、DevEco/真机启动、`Index.ets` + SDK TS 侧接入、host adapter。
- Android：无 Core-side 产物构建路径，App/device gate 无法启动。

**结论：App/device release gate 未满足。** 任何“可发布 App”的声明需先补齐上述项。

## 判定规则

- 命令退出码 0 = 通过；非 0 = 失败，必须记录失败原因与退出码。
- 告警（如 `ld` macOS 版本告警）不视为失败，但需记录。
- smoke（host FFI / host sim wrapper / Core-side 交叉产物）只能判定“Core-side 产物可用”，
  不得判定“App/device 可用”。
- 每轮若任一 Core-side gate 红，则 Core-side smoke release gate 不满足，需记入 [blockers.md](blockers.md)。

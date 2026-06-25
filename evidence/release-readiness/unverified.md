# 未验证项

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文是历史未验证
> 清单；当前未验证项必须围绕旧 `Reader-Core` 到 Rust Core、再到三端 App 的迁移链路更新。

> 基线 `fb4c3a7`，round-01。列出“尚未跑/无法跑”的项，避免被误读为“已通过”。

## 平台产物 / App / device

- iOS App 工程构建、模拟器/真机 App 启动、host adapter 接入（WebView 登录/Cookie、
  Keychain、文件沙箱、后台任务、签名分发）——未验证。
- HarmonyOS HAP 打包、DevEco/真机启动、`bindings/harmony/Index.ets` + `bindings/harmony/sdk/*.ts`
  TS 侧——未验证（仅 Core-side `.so` 产物构建 smoke）。
- Android——无构建路径，全未验证。
- 真实网络/TLS/HTTP Transport（platform adapter）——Core-side smoke 不覆盖，未验证。

## 脚本端到端

- `./scripts/build-local.sh` 未作为整脚本端到端跑（分量已分别覆盖：build --workspace 由
  test 隐式覆盖、reader-ffi --release 由 ffi-smoke 覆盖、cli --info 与 ffi-smoke 已跑）。
- `./scripts/integration-queue.sh` 未跑（人工集成工具，非自动 gate）。
- `./scripts/build-ios-xcframework.sh` 未单独跑（由 check-ios-swift-wrapper.sh 内部调用覆盖）。
- `./scripts/build-ohos.sh` 未单独跑（由 build-harmony-napi.sh 内部调用覆盖）。

## 测试覆盖维度

- reader-rule / reader-js 在**本基线** `fb4c3a7` 上测试数较少（rule 15、js 16）。
  仓库另有 `codex/rule-engine-parity` 分支推进规则兼容性测试（51+51），但**不属本基线**，
  不计入本证据包的 gate 计数。
- doc-tests 全为 0（各 crate 无 doc-test）。
- 无性能/压力/长时运行测试。
- 无真实书源端到端测试（仅 fixture inline + host.complete 回路）。

## 环境

- CI runner（`macos-15`）与本地（macOS 26.5.1）版本不同；CI 上的实际结果未在本轮采集
  （CI 修改属禁止范围，仅对照本地）。
- Android Rust target 已安装（aarch64-linux-android、x86_64-linux-android）但无脚本使用，
  未验证 Android 交叉构建。

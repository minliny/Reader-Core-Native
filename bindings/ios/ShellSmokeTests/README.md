# iOS host adapter ShellSmokeTests

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文只记录 iOS host
> adapter 的 shell smoke 证据，不代表 iOS App/真机完成。

## 这是什么

`run.sh` 编译 Swift wrapper（`ReaderCoreClient.swift`）与本目录的
`host_adapter_smoke.swift`，链接 macOS host 静态库 `target/debug/libreader_core.a`，
运行二进制，把分区报告 tee 到 `report.txt`。

smoke 直接驱动**真实 Rust Core**（通过 C ABI + JSON protocol），不是 stub、不是纯
type-check。

## 运行

```bash
bash bindings/ios/ShellSmokeTests/run.sh
```

若 `target/debug/libreader_core.a` 缺失，runner 会先 `cargo build -p reader-ffi`。
host 静态库路径可用 `READER_CORE_HOST_LIB` / `CARGO_TARGET_DIR` 覆盖。

`check-ios-swift-wrapper.sh`（含 `--swift-only`）会作为 gate 调用本 runner。

## 分区规则（强制）

每条用例打印 `[core]` 或 `[app-side]` 标签，便于报告机器分区：

- `[core]` —— 通过 C ABI / JSON protocol 由 **Rust Core** 实际执行的能力
  （`core.info`、`runtime.ping`、Core 发出 `host.request`、`host.complete` 恢复、
  `CANCELLED`、结构化 `error` code）。
- `[app-side]` —— 由 **iOS Swift adapter** 执行的能力（`ReaderCoreClient` 生命周期、
  `ReaderCoreHostRequest` 字段映射、`URLSessionHostTransport` 成功/超时、
  transport failure → `host.error` → core error、`pollEvent` drain/consumed）。

## wrapper smoke ≠ 设备完成

本 smoke 在 macOS host 上链接并运行 Core 静态库，证明：

- adapter 能编译、链接、驱动 host Core build；
- host adapter 契约（`host.request` → transport → `host.complete`/`host.error`）闭环；
- Core 侧关键能力（ABI 版本、ping、cancel、structured error）在 host build 上可用。

**不**证明：

- iOS App 构建/启动；
- iOS 模拟器或真机运行；
- 真实网络/TLS 书源端到端；
- WebView 登录、App UI、后台生命周期。

App/真机验证必须另行声明，不得由本 smoke 推断。

## 与兄弟 gate 的关系

- `scripts/check-ios-swift-wrapper.sh` —— 完整 iOS lane gate（重建 xcframework +
  wrapper typecheck + 全量 inline wrapper smoke）。本 runner 是该 gate 的一个
  **分区证据子步骤**，不替代全量 wrapper smoke。
- `scripts/ffi-smoke.sh` —— C/C++ 直接驱动 ABI 的 smoke（[[c-abi-stable-boundary-goal]]
  lane）。本 smoke 走 Swift adapter，是 ABI 之上的 adapter 层证据。

## 证据文件

`report.txt` 由 runner 每次运行覆写，包含时间戳、host_lib 路径、编译/运行输出与分区
汇总。`report.txt` 是运行产物；如需归档，复制到 `evidence/release-readiness/rounds/`
并按该目录约定命名（不在本 lane 作用域内修改）。

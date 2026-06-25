# iOS host adapter 集成状态

> 当前最高优先级文档是 `docs/LOCAL_REPO_MIGRATION_DIRECTIVE.md`。本文只记录 iOS host
> adapter 状态，不代表 iOS App/真机迁移完成。

最后更新：2026-06-25

分支：`codex/full-branch-directory-consolidation`

Gate：`bash ./scripts/check-ios-swift-wrapper.sh`（完整）或 `--swift-only`（快路径）。

## 作用域

本 lane 只修改 `bindings/ios/**` 与 `scripts/check-ios-swift-wrapper.sh`。不修改
C ABI（`include/reader_core.h`、`crates/reader-ffi/`）、`protocol/**`、其他平台
bindings、`native/**`。ABI/protocol 不足时记入下文「ABI/protocol gap」，不改 ABI。

## evidence 纪律（强制）

- **wrapper smoke ≠ 设备完成。** Gate 通过只证明 Swift adapter 能编译、链接并在 macOS
  host build 上驱动 Rust Core，**不**证明 iOS App/模拟器/真机运行。
- 报告必须区分 **app-side 能力**（iOS Swift adapter 执行）与 **Core 能力**（Rust Core
  通过 ABI/protocol 执行）。
- 分区可运行证据见 [`ShellSmokeTests/`](ShellSmokeTests/README.md)：每条用例带
  `[core]` / `[app-side]` 标签，runner 输出 `ShellSmokeTests/report.txt`。

## Gate 状态

green。`--swift-only` 路径：wrapper typecheck + macOS host inline wrapper smoke +
`ShellSmokeTests/run.sh`（分区证据）。完整路径额外重建 `target/ios/ReaderCore.xcframework`
并对 simulator slice 做 wrapper typecheck。

最新一轮 ShellSmokeTests 结果（`ShellSmokeTests/report.txt`）：`[core] pass=11 fail=0`、
`[app-side] pass=14 fail=0`、`host adapter shell smoke passed`（共 25 用例）。

## 能力分区

### Core 能力（Rust Core 通过 ABI/protocol 执行，host build 上验证）

| 能力 | 实现 | ShellSmoke 覆盖 |
| --- | --- | --- |
| ABI 版本 | `rc_abi_version()` → 1 | `[core] abi version == 1` |
| `core.info` | 返回 abi/protocol version + capability 列表 | `[core] core.info returns abi+protocol version`、`advertises host bus capability` |
| `runtime.ping` | Core 返回 `pong=true` | `[core] runtime.ping pong=true` |
| `host.request` 发射 | `runtime.hostSmoke` 触发 `host.request` + operationId | `[core] Core emits host.request with operationId` |
| `host.complete` 恢复 | operationId 关联，恢复原 request | `[core] host.complete resumes original request` |
| cancel | pending host op cancel → `CANCELLED` | `[core] cancel surfaces CANCELLED` |
| 结构化 error | unknown method → `UNKNOWN_METHOD` | `[core] unknown method surfaces UNKNOWN_METHOD` |

### app-side 能力（iOS Swift adapter 执行，host build 上验证）

| 能力 | 实现 | ShellSmoke 覆盖 |
| --- | --- | --- |
| Client 生命周期 | `ReaderCoreClient` create/destroy | `[app-side] ReaderCoreClient create + destroy` |
| Host request 字段映射 | `ReaderCoreHostRequest` url/method/headers/body | `[app-side] ReaderCoreHostRequest maps ...` |
| `URLSessionHostTransport` 成功 | method/headers/status/body 映射 | `[app-side] URLSessionHostTransport maps ...` |
| `URLSessionHostTransport` 超时 | timeout → `hostTransportFailed` | `[app-side] ... timeout → hostTransportFailed` |
| transport failure → core error | 走 `host.error` → 结构化 core error | `[app-side] transport failure surfaces core error` |
| `pollEvent` drain/consumed | 非阻塞 drain + 已消费事件返回 nil | `[app-side] pollEvent drains ...`、`returns nil for consumed` |

### 既有 wrapper smoke 覆盖（全量 inline smoke，未分区）

`check-ios-swift-wrapper.sh` 内的 inline macOS Swift smoke 额外覆盖：internal command ID
分配（避免 requestId `1001` 碰撞）、`book.search` host HTTP loop 返回 books、malformed
JSON send failure。这些用例尚未迁移到分区 runner，作为全量 wrapper 回归保留。

> 注：round 2 已将上述三类用例迁入分区 runner（分别记为 `[app-side] internal command ID
> collision avoidance`、`[app-side] book.search host HTTP loop returns books`、
> `[core] malformed JSON send fails with non-zero status`），并新增
> `[app-side] manual host.error resumes original request as error`。inline smoke 仍保留
> 作为全量回归。

## 未验证（不得由 smoke 推断）

- iOS App 构建/启动、模拟器/真机运行。
- 真实网络/TLS 书源端到端。
- WebView 登录、App UI、后台生命周期与 runtime 销毁。

App/真机验证必须另行声明。

## ABI/protocol gap 说明

该 lane 的 ABI v1 header 不暴露 `rc_last_error` 或其他 structured synchronous FFI error
accessor。因此 Swift wrapper 对 `rc_runtime_create`、`rc_runtime_send`、
`rc_runtime_cancel` failure 只能暴露 coarse `Int32` status。异步 Core `error` event 的
结构化细节可通过 `ReaderCoreCoreError` 获取。

本 lane 不修改 C ABI。若未来 ABI 增加 structured synchronous error accessor（由
[[c-abi-stable-boundary-goal]] lane 处理），Swift wrapper 可以增强 `createFailed` /
`sendFailed` / `cancelFailed`，且不改变 host command/event flow。

当前无新增 ABI/protocol gap。

### 已记录 gap

- **`host.error` 的 `code` 必须是合法 `ErrorCode` 枚举变体（protocol 约束，非 ABI）。**
  `ReaderCoreClient.sendHostError(code:message:retryable:requestId:)` 接受任意 `String`
  作为 `code`，但 Rust Core 的 `HostErrorParams.error: CoreError` 通过 serde 反序列化
  `code: ErrorCode`（`SCREAMING_SNAKE_CASE` 枚举，变体见 `crates/reader-contract/src/error.rs`：
  `UNKNOWN_METHOD` / `INVALID_PARAMS` / `INVALID_PROTOCOL_VERSION` / `CANCELLED` /
  `INVALID_MESSAGE` / `INTERNAL`）。若 host 传入未知 code 字符串，`host.error` params
  解析失败，Core 把 error 发到 `host.error` 命令自身的 requestId（adapter 内部 auto-allocated），
  **原 pending 请求永不恢复** —— 表现为调用方超时。
  - ShellSmokeTests 用 `[app-side] manual host.error resumes original request as error`
    覆盖合法 code 路径；非法 code 路径未覆盖（会超时，不作为断言用例）。
  - 处理方向（已在 round 3 落地）：adapter 侧增加 `ReaderCoreHostErrorCode` 枚举（变体与
    Core `ErrorCode` 一一对应），`sendHostError` 在发送前校验 `code`，未知 code 立即抛
    `ReaderCoreClientError.invalidHostErrorCode`，避免 host 误传未知 code 导致原请求静默
    超时。ShellSmokeTests 用 `[app-side] sendHostError rejects unknown ErrorCode` 覆盖。
    此为 **app-side 增强**，未改 ABI/protocol；若要扩 `ErrorCode` 变体则属 protocol lane
    （[[c-abi-stable-boundary-goal]]），本 lane 不处理。

## 线程说明

- `rc_event_callback` 由 Core-owned background thread 触发。wrapper 会立即把 event bytes
  复制到 `Data`，并放入 thread-safe `NSCondition` buffer。
- `ReaderCoreClient.request` 和 `URLSessionHostTransport.perform` 会阻塞调用线程。应由
  host-owned task/thread 调用，不要从 Core callback thread 调用。
- `URLSessionHostTransport` 用可配置 timeout 将 URLSession async completion 桥接到同步
  host transport protocol。

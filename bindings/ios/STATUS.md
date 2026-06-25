# iOS Swift Wrapper 集成状态

最后更新：2026-06-24

分支：`codex/reader-core-runtime-protocol`

Gate：`bash ./scripts/check-ios-swift-wrapper.sh`

Gate 状态：green。该脚本已验证可构建 `target/ios/ReaderCore.xcframework`，对
simulator slice 做 Swift wrapper type-check，构建 macOS host `reader-ffi` static
library，链接 Swift smoke，并输出 `swift client smoke passed`。

## 已闭环能力

| 能力 | 实现 | Smoke 覆盖 |
| --- | --- | --- |
| Runtime create / cancel / destroy | `ReaderCoreRuntime` 封装 `rc_runtime_create` / `rc_runtime_cancel` / `rc_runtime_destroy`，`ReaderCoreClient.cancel` 转发 cancellation | `core.info` + `runtime.ping`；pending `runtime.hostSmoke` cancel 返回 `CANCELLED` |
| Send command | `ReaderCoreRuntime.send` / `ReaderCoreClient.send` / `request` | command round-trip 与 malformed JSON send failure |
| Poll / parse event | `ReaderCoreEventBuffer.poll` + `ReaderCoreEvent` parsing | `pollEvent` 非阻塞 drain `core.info` 和 host-request events |
| `http.execute` host.request | `ReaderCoreHostTransport` protocol；默认 `URLSessionHostTransport` 带 timeout | 本地 `URLProtocol` 校验 method/header/status/body 和 timeout；`book.search` host HTTP loop 返回 books |
| `host.complete` / `host.error` | `ReaderCoreClient.sendHostComplete` / `sendHostError`；`request` 内自动 completion | manual `runtime.hostSmoke` completion 恢复原 request；internal command IDs 避免 requestId `1001`；transport failure 走 `host.error` |
| Error exposure | Core `error` event 映射到 `ReaderCoreCoreError`；FFI failure 使用 coarse `Int32` status | `UNKNOWN_METHOD`、`CANCELLED`、transport failure、malformed send |

## ABI gap 说明

该 lane 的 ABI v1 header 不暴露 `rc_last_error` 或其他 structured synchronous FFI error
accessor。因此 Swift wrapper 对 `rc_runtime_create`、`rc_runtime_send`、
`rc_runtime_cancel` failure 只能暴露 coarse `Int32` status。异步 Core `error` event 的
结构化细节可通过 `ReaderCoreCoreError` 获取。

本 lane 不修改 C ABI。若未来 ABI 增加 structured synchronous error accessor，Swift
wrapper 可以增强 `createFailed` / `sendFailed` / `cancelFailed`，且不改变 host
command/event flow。

## 线程说明

- `rc_event_callback` 由 Core-owned background thread 触发。wrapper 会立即把 event bytes
  复制到 `Data`，并放入 thread-safe `NSCondition` buffer。
- `ReaderCoreClient.request` 和 `URLSessionHostTransport.perform` 会阻塞调用线程。应由
  host-owned task/thread 调用，不要从 Core callback thread 调用。
- `URLSessionHostTransport` 用可配置 timeout 将 URLSession async completion 桥接到同步
  host transport protocol。

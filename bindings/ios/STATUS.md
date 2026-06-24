# iOS Swift Wrapper — Integration Status

Last updated: 2026-06-24
Branch: `codex/reader-core-runtime-protocol`
Gate: `bash ./scripts/check-ios-swift-wrapper.sh`
Gate status: **green** — verified `bash ./scripts/check-ios-swift-wrapper.sh`
builds `target/ios/ReaderCore.xcframework`, type-checks the Swift wrapper
against the simulator slice, builds the macOS host `reader-ffi` static library,
links the Swift smoke, and prints `swift client smoke passed`.

## Closed loop

The checked-in `ReaderCoreClient` covers the minimal loop an iOS host needs to
drive Core:

| Capability | Implementation | Smoke coverage |
|---|---|---|
| Runtime create / cancel / destroy | `ReaderCoreRuntime` over `rc_runtime_create` / `rc_runtime_cancel` / `rc_runtime_destroy`; `ReaderCoreClient.cancel` forwards cancellation | `core.info` + `runtime.ping`; pending `runtime.hostSmoke` cancel surfaces `CANCELLED` |
| Send command | `ReaderCoreRuntime.send` / `ReaderCoreClient.send` / `request` | command round-trips and malformed JSON send failure |
| Poll / parse event | `ReaderCoreEventBuffer.poll` + `ReaderCoreEvent` parsing | `pollEvent` drains `core.info` and host-request events non-blocking |
| `http.execute` host.request | `ReaderCoreHostTransport` protocol; `URLSessionHostTransport` default with timeout | local `URLProtocol` verifies method/header/status/body and timeout handling; `book.search` host HTTP loop returns books |
| `host.complete` / `host.error` | `ReaderCoreClient.sendHostComplete` / `sendHostError`; automatic completion inside `request` | manual `runtime.hostSmoke` completion resumes original request; internal command IDs avoid requestId `1001`; failing transport routes through `host.error` |
| Error exposure | `ReaderCoreCoreError` for Core `error` events; coarse `Int32` status for FFI failures | `UNKNOWN_METHOD`, `CANCELLED`, transport failure, malformed send |

## ABI-gap notes

This branch's ABI v1 header does not expose `rc_last_error` or any other
structured synchronous FFI error accessor. The Swift wrapper therefore exposes
only coarse `Int32` status for `rc_runtime_create`, `rc_runtime_send`, and
`rc_runtime_cancel` failures. Structured error detail is available for async
Core `error` events through `ReaderCoreCoreError`.

No C ABI changes are made in this lane. If a future ABI adds structured
synchronous error access, the Swift wrapper can enrich `createFailed` /
`sendFailed` / `cancelFailed` without changing host command/event flow.

## Threading notes

- The `rc_event_callback` fires on a Core-owned background thread. The wrapper
  copies event bytes into `Data` immediately and enqueues them on a thread-safe
  `NSCondition` buffer.
- `ReaderCoreClient.request` and `URLSessionHostTransport.perform` block the
  calling thread. Call them from a host-owned task/thread, not from the Core
  callback thread.
- `URLSessionHostTransport` bridges URLSession's async completion onto the
  synchronous host transport protocol with a configurable timeout.

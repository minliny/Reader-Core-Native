# iOS Swift Wrapper — Integration Status

Last updated: 2026-06-24
Branch: `codex/data-subsystem` (carried forward from `codex/ios-wrapper-integration`)
Gate: `bash ./scripts/check-ios-swift-wrapper.sh` (typecheck + link + runtime smoke)
Gate status: **blocked in current worktree before iOS binding checks** —
`scripts/build-ios-xcframework.sh` fails while compiling
`crates/reader-storage/src/lib.rs`: Rust reports `borrow of moved value:
remaining` at line 921. That Rust source is outside this lane's allowed edit
scope, so it is recorded here rather than fixed from `bindings/ios`.

Swift wrapper local checks that do not rebuild Rust from source:
- **pass** — `READER_CORE_IOS_SWIFT_ONLY=1 bash ./scripts/check-ios-swift-wrapper.sh`
  type-checks `bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift`
  against the current `include/reader_core.h` + module map, links the Swift
  smoke against existing `target/debug/libreader_core.a`, and prints
  `swift client smoke passed`. This mode is diagnostic-only and does not replace
  the default full gate.

## Closed loop

The checked-in `ReaderCoreClient` now covers the minimal loop an iOS app needs
to integrate Core end-to-end. All of the following are exercised by the gate's
runtime smoke (macOS executable linked against `target/debug/libreader_core.a`):

| Capability | Implementation | Smoke coverage |
|---|---|---|
| Runtime create / cancel / destroy | `ReaderCoreRuntime` over `rc_runtime_create` / `rc_runtime_cancel` / `rc_runtime_destroy`; `ReaderCoreClient.cancel` forwards cancellation | `core.info` + `runtime.ping` round-trip; pending `runtime.hostSmoke` cancel surfaces `CANCELLED` |
| Send command | `ReaderCoreRuntime.send` / `ReaderCoreClient.send` / `request` | every test |
| Poll / parse event | `ReaderCoreEventBuffer.poll` + `ReaderCoreEvent` parsing | `pollEvent` drains a `core.info` result non-blocking |
| `http.execute` host.request | `ReaderCoreHostTransport` protocol; `URLSessionHostTransport` default with configurable timeout | local `URLProtocol` verifies `URLSessionHostTransport` method/header/status/body mapping and timeout handling; `book.search` with `searchRequest` → stub transport → `host.complete` → books |
| `host.complete` / `host.error` | `ReaderCoreClient.sendHostComplete` / `sendHostError` (auto in `request`, manual API too) | manual `runtime.hostSmoke` → `host.request` → `sendHostComplete` → original `result`; default internal host-complete command IDs avoid collision with requestId `1001`; transport-failure path routes through `host.error` and surfaces as a core error |
| Error exposure | `ReaderCoreCoreError` (typed `code`/`message`/`retryable`) for `error` events; `ReaderCoreFFIError(code,message)` plus coarse ABI status on FFI failures | `UNKNOWN_METHOD` event error; malformed-JSON `sendFailed(status: 3, lastError: RC_ERR_INVALID_MESSAGE)`; invalid config `createFailed(status: 4, lastError: RC_ERR_INVALID_MESSAGE)` |

This iOS wrapper batch requires no additional changes to `crates/`,
`include/reader_core.h`, `protocol/`, or other platform bindings. The loop is
built entirely on the ABI v1 surface (`rc_abi_version` /
`rc_runtime_create` / `rc_runtime_send` / `rc_runtime_cancel` /
`rc_runtime_destroy`) plus `rc_last_error` for structured FFI-failure text.

## ABI-gap notes (recorded, not fixed)

**Scope rule (2026-06-24):** going forward this lane only edits `bindings/ios/`
and `scripts/check-ios-swift-wrapper.sh`. The C ABI
(`include/reader_core.h`, `crates/reader-ffi/`) is not modified here. Any
feature the ABI cannot express is recorded below rather than patched into the
ABI. (The `rc_last_error` extension already in the working tree predates this
rule; it is consumed read-only by the wrapper and requires no further ABI
edits.)

These are constraints of ABI v1 as observed from the Swift host. None block the
closed loop; they are recorded here so a future ABI revision can revisit them.

1. **`rc_last_error` is per-thread and read-only.** Reading does not clear the
   slot; the next successful FFI call on the same thread does. The wrapper
   captures the structured code/message immediately after a failing
   `rc_runtime_create` / `rc_runtime_send` and before any other FFI call, so the
   slot is still populated. `rc_runtime_cancel` returns `0` for all outcomes
   (including not-found), so it never populates the slot — `cancelFailed` is
   therefore unreachable from Swift today; the case is retained for
   forward-compatibility.

2. **Events are callback-only; there is no `rc_runtime_poll` / try-recv.**
   `pollEvent` is a Swift-layer construct over an internal thread-safe buffer
   fed by the `rc_event_callback`. A true non-blocking ABI poll (returning a
   borrowed event or "empty") would let a host drain without an always-on
   callback sink and without copying every event into Swift storage. Not a
   defect — the callback model is intentional — but polling is not an ABI
   feature today.

3. **Host completion is protocol-level, not a dedicated FFI entry.** There is
   no `rc_host_complete` function; the host answers `host.request` by sending a
   `host.complete` / `host.error` command through `rc_runtime_send`. This is by
   design (Core never owns sockets/TLS) and is fully supported. Recorded only
   because a host expecting a host-specific FFI call will not find one.

## Threading notes for host integrators

- The `rc_event_callback` fires on a Core-owned background thread. The wrapper
  copies event bytes into `Data` immediately and enqueues them on a
  thread-safe `NSCondition` buffer, so the callback never blocks on
  cross-thread marshalling.
- `ReaderCoreClient.request` / `URLSessionHostTransport.perform` block the
  calling thread. Call them from a task/thread the host controls, **not** from
  the Core event-callback thread.
- `URLSessionHostTransport` bridges URLSession's async completion onto a
  `DispatchSemaphore` so it fits the synchronous send/event model. Hosts that
  prefer async can implement `ReaderCoreHostTransport` directly.

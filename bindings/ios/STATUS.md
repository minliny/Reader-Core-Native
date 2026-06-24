# iOS Swift Wrapper — Integration Status

Last updated: 2026-06-24
Branch: `codex/ios-wrapper-integration`
Gate: `./scripts/check-ios-swift-wrapper.sh` (typecheck + link + runtime smoke)

## Closed loop

The checked-in `ReaderCoreClient` now covers the minimal loop an iOS app needs
to integrate Core end-to-end. All of the following are exercised by the gate's
runtime smoke (macOS executable linked against `target/debug/libreader_core.a`):

| Capability | Implementation | Smoke coverage |
|---|---|---|
| Runtime create / destroy | `ReaderCoreRuntime` over `rc_runtime_create` / `rc_runtime_destroy` | `core.info` + `runtime.ping` round-trip |
| Send command | `ReaderCoreRuntime.send` / `ReaderCoreClient.send` / `request` | every test |
| Poll / parse event | `ReaderCoreEventBuffer.poll` + `ReaderCoreEvent` parsing | `pollEvent` drains a `core.info` result non-blocking |
| `http.execute` host.request | `ReaderCoreHostTransport` protocol; `URLSessionHostTransport` default | `book.search` with `searchRequest` → stub transport → `host.complete` → books |
| `host.complete` / `host.error` | `ReaderCoreClient.sendHostComplete` / `sendHostError` (auto in `request`, manual API too) | transport-failure path routes through `host.error` and surfaces as a core error |
| Error exposure | `ReaderCoreCoreError` (typed `code`/`message`/`retryable`) for `error` events; coarse ABI status on FFI failures | `UNKNOWN_METHOD` event error; malformed-JSON `sendFailed(3)` |

No changes to `crates/`, `include/reader_core.h`, `protocol/`, or other
platform bindings were required. The loop is built entirely on the existing
ABI v1 surface (`rc_abi_version` / `rc_runtime_create` / `rc_runtime_send` /
`rc_runtime_cancel` / `rc_runtime_destroy`).

## ABI-gap notes (recorded, not fixed)

These are constraints of ABI v1 as observed from the Swift host. None block the
closed loop; they are recorded here so a future ABI revision can revisit them.
No cross-directory changes were made for any of them.

1. **No structured FFI error accessor on this branch.** The committed header on
   `codex/ios-wrapper-integration` does not declare `rc_last_error`, so FFI
   failures (`createFailed` / `sendFailed` / `cancelFailed`) carry only the
   coarse `int32_t` status. A structured `rc_last_error` ABI is being added in
   parallel (uncommitted, out of scope for this lane); once it lands on this
   branch, the wrapper can enrich those cases with per-thread error text. Until
   then, structured error detail is available only for Core `error` events
   (`ReaderCoreCoreError`).

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

4. **`rc_runtime_cancel` carries no structured error.** It returns `0` for all
   outcomes in ABI v1 (including not-found), so `cancelFailed` is currently
   unreachable from Swift. The case is retained for forward-compatibility.

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

# ReaderCore iOS Binding

This binding lane packages the C ABI static library and `reader_core.h` into an
XCFramework, plus a Swift wrapper (`ReaderCoreClient`) that implements the
minimal closed loop an iOS app needs to drive Core: runtime create/destroy,
command send, event poll/parse, `http.execute` host transport, `host.complete`
reporting, and structured error exposure.

WebView login and app UI live in the iOS host repository; the wrapper is
intentionally transport-agnostic so a host can inject its own
`ReaderCoreHostTransport`.

## Build

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/build-ios-xcframework.sh
```

Output:

```text
target/ios/ReaderCore.xcframework
```

Each XCFramework slice includes:

- `Headers/reader_core.h`
- `Headers/module.modulemap`

The build script also type-checks the Swift wrapper with `import ReaderCore`
against the simulator slice.

## Swift wrapper smoke

`bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift` wraps the ABI v1
runtime handle. The public surface:

**Runtime handle** — `ReaderCoreRuntime`
- `abiVersion`
- `init(configJSON:onEvent:)`
- `send(json:)` / `send(jsonString:)`
- `cancel(requestId:)`
- `destroy()`

**High-level client** — `ReaderCoreClient`
- `init(configJSON:hostTransport:)` — an optional `ReaderCoreHostTransport`
  handles `host.request` events (defaults to `nil`; pass
  `URLSessionHostTransport()` for live HTTP).
- `coreInfo(requestId:timeout:)`
- `ping(requestId:timeout:)`
- `request(method:requestId:params:timeout:)` — sends a command and resolves it
  to a `result` event, transparently driving any `host.request` through the
  configured transport before returning.
- `send(method:requestId:params:)` — non-blocking send; pair with `pollEvent`.
- `pollEvent(requestId:)` — non-blocking drain of the next buffered event
  (`Result<ReaderCoreEvent, ReaderCoreClientError>?`).
- `sendHostComplete(operationId:result:requestId:)` /
  `sendHostError(operationId:code:message:retryable:requestId:)` — manual host
  completion for hosts that drive `host.request` themselves.
- `destroy()`

**Events** — `ReaderCoreEvent`
- `type`, `requestId`, `data`, `error`
- `isHostRequest`, `operationId`, `capability`, `hostRequestParams`
- `coreError` — typed `ReaderCoreCoreError` (`code`/`message`/`retryable`) for
  `error` events.

**Host transport**
- `ReaderCoreHostTransport` protocol.
- `ReaderCoreHostRequest` (`operationId`, `capability`, `url`, `method`,
  `headers`, `body`, `rawParams`).
- `ReaderCoreHostResponse` (`status`, `headers`, `body`).
- `URLSessionHostTransport` — default `http.execute` implementation backed by
  `URLSession`, bridged onto the synchronous send/event model with a
  `DispatchSemaphore`.

**Errors** — `ReaderCoreClientError`
- `createFailed(Int32)`, `sendFailed(Int32)`, `cancelFailed(Int32)` carry the
  coarse ABI status from `rc_runtime_create` / `rc_runtime_send` /
  `rc_runtime_cancel`.
- `coreError(ReaderCoreCoreError)` for Core `error` events.
- `missingHostTransport`, `hostTransportFailed(String)`, `requestTimedOut`,
  `invalidCommandJSON`, `invalidEventJSON`, `runtimeDestroyed`.

Validate it with:

```bash
./scripts/check-ios-swift-wrapper.sh
```

The gate first type-checks the wrapper against the
`arm64-apple-ios13.0-simulator` XCFramework slice, then builds the host
`reader-ffi` static library and runs a macOS Swift executable that exercises
`core.info`, `runtime.ping`, event polling, the `http.execute` host loop,
`host.error` propagation, and structured-error exposure through
`ReaderCoreClient`. This keeps the checked-in smoke free of a full iOS app
project while proving wrapper compile/link/runtime behavior.

The XCFramework exposes the ABI v1 functions declared in
`include/reader_core.h`:

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

The event callback buffers are borrowed for the callback duration only. Swift
or Objective-C wrappers must copy event bytes before returning from the callback
(the wrapper copies into `Data` immediately on receipt).

See `STATUS.md` for the integration state and ABI-gap notes.

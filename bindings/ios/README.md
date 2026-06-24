# ReaderCore iOS Binding

This binding lane packages the C ABI static library and `reader_core.h` into an
XCFramework, plus a Swift wrapper (`ReaderCoreClient`) that drives Core through
runtime lifecycle, command/event polling, host HTTP request completion, and
typed async error events.

WebView login and app UI live in the iOS host repository. The wrapper is
transport-agnostic so a host can inject its own `ReaderCoreHostTransport`, while
`URLSessionHostTransport` provides a default `http.execute` implementation.

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
- `init(configJSON:hostTransport:)`
- `coreInfo(requestId:timeout:)`
- `ping(requestId:timeout:)`
- `send(method:requestId:params:)`
- `request(method:requestId:params:timeout:)`
- `pollEvent(requestId:)`
- `cancel(requestId:)`
- `sendHostComplete(operationId:result:requestId:)`
- `sendHostError(operationId:code:message:retryable:requestId:)`
- `destroy()`

**Host transport**
- `ReaderCoreHostTransport`
- `ReaderCoreHostRequest`
- `ReaderCoreHostResponse`
- `URLSessionHostTransport(session:timeout:)`

**Errors**
- `ReaderCoreCoreError` for Core `error` events.
- `createFailed(Int32)`, `sendFailed(Int32)`, and `cancelFailed(Int32)` for
  synchronous FFI failures. ABI v1 on this branch has no structured
  `rc_last_error`; see `STATUS.md`.

Validate it with:

```bash
bash ./scripts/check-ios-swift-wrapper.sh
```

The default gate rebuilds the iOS XCFramework, type-checks the wrapper against
the simulator slice, builds the host `reader-ffi` static library, then runs a
macOS Swift executable. The smoke covers `core.info`, `runtime.ping`, polling,
cancellation, manual `host.complete`, internal host-command request ID
allocation, `URLSessionHostTransport` success and timeout paths via local
`URLProtocol` handlers, the `http.execute` host loop, `host.error`
propagation, and async structured Core errors.

When unrelated Rust work temporarily breaks the full gate, the Swift wrapper can
be checked against the current header and existing host static library with:

```bash
bash ./scripts/check-ios-swift-wrapper.sh --swift-only
```

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

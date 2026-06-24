# ReaderCore iOS Binding

This binding lane packages the C ABI static library and `reader_core.h` into an
XCFramework. It is intentionally a smoke-level platform artifact: the checked-in
Swift wrapper covers ABI lifecycle, `core.info`, and `runtime.ping`; Swift
concurrency adapters, URLSession host transport, WebView login, and app UI live
in the iOS host repository.

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

The build script also type-checks a Swift smoke file with `import ReaderCore`
against the simulator slice.

## Swift wrapper smoke

`bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift` contains a
minimal Swift wrapper around the ABI v1 runtime handle:

- `ReaderCoreRuntime.abiVersion`
- `ReaderCoreRuntime(configJSON:onEvent:)`
- `send(json:)`
- `send(jsonString:)`
- `cancel(requestId:)`
- `destroy()`
- `ReaderCoreClient.coreInfo(requestId:timeout:)`
- `ReaderCoreClient.ping(requestId:timeout:)`

Validate it with:

```bash
./scripts/check-ios-swift-wrapper.sh
```

The gate first type-checks the wrapper against the `arm64-apple-ios13.0-simulator`
XCFramework slice, then builds the host `reader-ffi` static library and runs a
macOS Swift executable that calls `core.info` and `runtime.ping` through
`ReaderCoreClient`. This keeps the checked-in smoke free of a full iOS app
project while still proving wrapper compile/link/runtime behavior.

The XCFramework exposes the ABI v1 functions declared in
`include/reader_core.h`:

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

The event callback buffers are borrowed for the callback duration only. Swift
or Objective-C wrappers must copy event bytes before returning from the callback.

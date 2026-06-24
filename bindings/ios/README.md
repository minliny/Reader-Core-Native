# ReaderCore iOS Binding

This binding lane packages the C ABI static library and `reader_core.h` into an
XCFramework. It is intentionally a smoke-level platform artifact: Swift wrapper,
Swift concurrency adapters, URLSession host transport, WebView login, and app UI
live in the iOS host repository.

## Build

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/build-ios-xcframework.sh
```

Output:

```text
target/ios/ReaderCore.xcframework
```

Current status: this is a Core-side artifact smoke. The XCFramework/header
packaging and Swift wrapper typecheck gate are covered in this repository; iOS
App runtime loading, URLSession host transport, WebView login, and UI
integration remain host-repository work.

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

Validate it with:

```bash
./scripts/check-ios-swift-wrapper.sh
```

The smoke target is `arm64-apple-ios13.0-simulator`. This gate type-checks the
wrapper against the Core ABI; it is not an App/device runtime smoke.

The XCFramework exposes the ABI v1 functions declared in
`include/reader_core.h`:

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

The event callback buffers are borrowed for the callback duration only. Swift
or Objective-C wrappers must copy event bytes before returning from the callback.

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

Each XCFramework slice includes:

- `Headers/reader_core.h`
- `Headers/module.modulemap`

The build script also type-checks a Swift smoke file with `import ReaderCore`
against the simulator slice.

The XCFramework exposes the ABI v1 functions declared in
`include/reader_core.h`:

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

The event callback buffers are borrowed for the callback duration only. Swift
or Objective-C wrappers must copy event bytes before returning from the callback.

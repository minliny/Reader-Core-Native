# ReaderCore iOS Binding

该 binding lane 将 C ABI 静态库和 `reader_core.h` 打包成 XCFramework，并提供 Swift
wrapper `ReaderCoreClient`。wrapper 通过 runtime lifecycle、command/event polling、
host HTTP request completion 和 typed async error event 驱动 Core。

WebView login 和 App UI 位于 iOS host repository。wrapper 与 transport 解耦，host 可
注入自己的 `ReaderCoreHostTransport`；默认 `URLSessionHostTransport` 提供
`http.execute` 实现。

## 构建

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/build-ios-xcframework.sh
```

输出：

```text
target/ios/ReaderCore.xcframework
```

每个 XCFramework slice 包含：

- `Headers/reader_core.h`
- `Headers/module.modulemap`

构建脚本还会用 simulator slice 对 Swift wrapper 做 `import ReaderCore` type-check。

## Swift wrapper smoke 验证

`bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift` 封装 ABI v1 runtime
handle。public surface：

**Runtime handle：`ReaderCoreRuntime`**

- `abiVersion`
- `init(configJSON:onEvent:)`
- `send(json:)` / `send(jsonString:)`
- `cancel(requestId:)`
- `destroy()`

**High-level client：`ReaderCoreClient`**

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

**Error**

- `ReaderCoreCoreError` 表示 Core `error` event。
- `createFailed(Int32)`、`sendFailed(Int32)`、`cancelFailed(Int32)` 表示同步 FFI
  failure。ABI v1 在此 lane 中没有 structured `rc_last_error`；详见 `STATUS.md`。

验证：

```bash
bash ./scripts/check-ios-swift-wrapper.sh
```

默认 gate 会重建 iOS XCFramework，对 simulator slice 做 wrapper type-check，构建 host
`reader-ffi` static library，然后运行 macOS Swift executable。smoke 覆盖
`core.info`、`runtime.ping`、polling、cancellation、manual `host.complete`、internal
host-command request ID allocation、`URLSessionHostTransport` 的 success/timeout path、
`http.execute` host loop、`host.error` propagation、async structured Core errors。

当无关 Rust work 临时打断完整 gate 时，可只用当前 header 和现有 host static library
检查 Swift wrapper：

```bash
bash ./scripts/check-ios-swift-wrapper.sh --swift-only
```

XCFramework 暴露 `include/reader_core.h` 中声明的 ABI v1 functions：

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

Event callback buffer 只在 callback 生命周期内借用。Swift 或 Objective-C wrapper 必须在
callback 返回前复制 event bytes；当前 wrapper 收到后立即复制到 `Data`。

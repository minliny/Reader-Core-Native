# ReaderCore Android JNI Binding

该 binding lane 将 C ABI 静态库和 `reader_core.h` 打包成 Android JNI shared
library，并提供一个最小 Java wrapper。wrapper 可以驱动 Core runtime 生命周期、
发送 JSON command、轮询 callback 传回的 event、取消 request，并用 `host.complete`
或 `host.error` 回复 `host.request`。

Android wrapper 不新增也不修改 Core ABI。它消费 ABI v1：

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`

## 构建

安装 Android NDK、CMake 和所需 Rust Android target 后执行：

```bash
./build-android-jni.sh
```

默认值：

- ABI：`arm64-v8a x86_64`
- Android API：`24`
- 输出：`target/android-jni/libs/<abi>/libreader_core_jni.so`

可通过环境变量覆盖：

```bash
ANDROID_ABIS="arm64-v8a armeabi-v7a x86_64 x86" ANDROID_API=24 ./build-android-jni.sh
```

脚本在缺少 Android NDK、CMake 或 Rust target 时 fail-closed，不会在没有真实 NDK
link 的情况下把 Android JNI 标成已构建。native gate 前会编译 Java wrapper 和 Java
sample；如果 `kotlinc` 可用，也会编译 Kotlin sample。

合并后的 SDK 只保留一个 JNI class：`com.reader.core.NativeCoreBridge`。早期分支曾有
Java 与 Kotlin 两个同名定义；当前分支保留 Java 定义作为 canonical JNI owner，并从中
暴露两种 API 形态。

## Java API

`bindings/android/src/main/java/com/reader/core/ReaderCoreRuntime.java` 是最小 SDK surface：

- `abiVersion()`
- `new ReaderCoreRuntime(configJson)`
- `send(jsonCommand)` / `sendCommand(method, requestId, paramsJson)`
- `pollEvent(timeout, unit)` / `pollEventBytes(timeout, unit)`
- `cancel(requestId)`
- `sendHostComplete(operationId, resultJson, requestId)`
- `sendHostError(operationId, code, message, retryable, requestId)`
- `close()`

ABI 层 event 只通过 callback 传递。JNI 层在 callback 内复制 event bytes，并放入 native
queue，供 Java 轮询。

示例：

- `bindings/android/samples/java/MinimalReaderCore.java`
- `bindings/android/samples/kotlin/MinimalReaderCore.kt`

## Direct Kotlin API

需要 callback-style event 的 Kotlin host 可直接调用 `NativeCoreBridge` 的 public API：

- `pingSmoke()`
- `runtimeCreate(configJson, listener)`
- `runtimeSend(handle, commandJson)`
- `runtimeCancel(handle, requestId)`
- `runtimeDestroy(handle)`

`bindings/android/src/main/kotlin/com/reader/core/ReaderEventListener.kt` 定义 listener
contract，`bindings/android/sample/ReaderCoreSample.kt` 展示 direct listener flow。
这只是 wrapper 形态；host completion 仍通过 `runtimeSend(... host.complete ...)` 完成，
不会新增 C ABI symbol。

## Host request

Core 会为 `http.execute` 等 platform-owned capability 发出 `host.request` event。
Android host 应：

1. 轮询 event。
2. 在 Java/Kotlin 侧解析 `operationId`、`capability`、`params`。
3. 执行平台操作。
4. 调用 `sendHostComplete(...)` 或 `sendHostError(...)`。

ABI v1 有意不提供 `rc_host_complete`。host completion 是通过 `rc_runtime_send` 发送的
普通 JSON command。

当前 gate 状态和 ABI gap 见 `STATUS.md`。

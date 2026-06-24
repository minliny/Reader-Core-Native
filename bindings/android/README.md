# ReaderCore Android JNI Binding

This binding lane packages the C ABI static library and `reader_core.h` into an
Android JNI shared library, plus a small Java wrapper that can drive the Core
runtime lifecycle, send JSON commands, poll callback-delivered events, cancel
requests, and answer `host.request` operations with `host.complete` or
`host.error`.

The Android wrapper does not add or change the Core ABI. It uses only ABI v1:

- `rc_abi_version`
- `rc_runtime_create`
- `rc_runtime_send`
- `rc_runtime_cancel`
- `rc_runtime_destroy`
- `rc_last_error`

## Build

Install the Android NDK, CMake, and the Rust Android targets you want to build,
then run:

```bash
./build-android-jni.sh
```

Defaults:

- ABIs: `arm64-v8a x86_64`
- Android API: `24`
- Output: `target/android-jni/libs/<abi>/libreader_core_jni.so`

Override with:

```bash
ANDROID_ABIS="arm64-v8a armeabi-v7a x86_64 x86" ANDROID_API=24 ./build-android-jni.sh
```

The script fails closed when the Android NDK, CMake, or a requested Rust target
is missing. It does not mark Android JNI as built without a real NDK link.
Before the native gate, it compiles the Java wrapper and Java sample. When
`kotlinc` is available (including Android Studio's bundled compiler), it also
compiles the Kotlin sample.

## Java API

`bindings/android/src/main/java/com/reader/core/ReaderCoreRuntime.java` is the
minimal SDK surface:

- `abiVersion()`
- `new ReaderCoreRuntime(configJson)`
- `send(jsonCommand)` / `sendCommand(method, requestId, paramsJson)`
- `pollEvent(timeout, unit)` / `pollEventBytes(timeout, unit)`
- `cancel(requestId)`
- `sendHostComplete(operationId, resultJson, requestId)`
- `sendHostError(operationId, code, message, retryable, requestId)`
- `close()`

Events are callback-only at the ABI layer. The JNI layer copies the event bytes
inside the callback and buffers them in a native queue for Java polling.

See:

- `bindings/android/samples/java/MinimalReaderCore.java`
- `bindings/android/samples/kotlin/MinimalReaderCore.kt`

## Host requests

Core emits a `host.request` event for platform-owned capabilities such as
`http.execute`. The Android host should:

1. Poll the event.
2. Parse `operationId`, `capability`, and `params` on the Java/Kotlin side.
3. Execute the platform operation.
4. Call `sendHostComplete(...)` or `sendHostError(...)`.

There is intentionally no `rc_host_complete` FFI function in ABI v1. Host
completion is a normal JSON command sent through `rc_runtime_send`.

See `STATUS.md` for the current gate state and ABI-gap notes.

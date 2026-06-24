# Android JNI SDK

This directory contains the Android JNI bridge for Reader-Core (ABI v1).

The shared library `libreader_core_jni.so` exposes the Kotlin entry point
`com.reader.core.NativeCoreBridge` plus the `ReaderEventListener` interface.

## Kotlin API

```kotlin
package com.reader.core

object NativeCoreBridge {
    init { System.loadLibrary("reader_core_jni") }

    @JvmStatic external fun abiVersion(): Int
    @JvmStatic external fun pingSmoke(): String
    @JvmStatic external fun runtimeCreate(configJson: String, listener: ReaderEventListener): Long
    @JvmStatic external fun runtimeSend(handle: Long, commandJson: String): Int
    @JvmStatic external fun runtimeCancel(handle: Long, requestId: Long): Int
    @JvmStatic external fun runtimeDestroy(handle: Long)
}

interface ReaderEventListener {
    // Invoked on a Core background thread for every event (result / error / host.request).
    fun onEvent(eventJson: String)
}
```

See [`src/main/kotlin/com/reader/core/`](src/main/kotlin/com/reader/core/) for the
canonical declarations and [`sample/ReaderCoreSample.kt`](sample/ReaderCoreSample.kt)
for an end-to-end example (create → `runtime.ping` → answer `host.request` with
`host.complete` → destroy).

## Lifecycle & host bridge

- `runtimeCreate` returns an opaque `Long` handle. All subsequent calls key off it.
- Events are delivered on a Core-owned background thread; the bridge attaches that
  thread to the JVM and calls `ReaderEventListener.onEvent`. Implementations must be
  thread-safe.
- `host.complete` / `host.error` are **commands** the platform sends back via
  `runtimeSend` in response to a `host.request` event — no ABI change required.
  See `protocol/fixtures/conformance/host/`.

`pingSmoke()` is retained as a one-shot smoke (create → `runtime.ping` → first
event → destroy) for the build gate; it is not a business API.

## Build

```bash
./scripts/build-android-jni.sh
```

Driven by CMake with the NDK toolchain
(`bindings/android/CMakeLists.txt`). Produces
`target/android-jni/<abi>/libreader_core_jni.so` (currently `arm64-v8a` only).

The script fails closed when the Android NDK, CMake, or the Rust Android target is
missing — Android JNI smoke is not considered passed without a real NDK build.

Status and ABI-gap ledger: [`STATUS.md`](STATUS.md).

# Android JNI Smoke

This directory contains the Core-side Android JNI smoke shim for phase 2.

The smoke library exports the Java class shape below:

```kotlin
package com.reader.core

object NativeCoreBridge {
    init {
        System.loadLibrary("reader_core_jni")
    }

    @JvmStatic external fun abiVersion(): Int
    @JvmStatic external fun pingSmoke(): String
}
```

`pingSmoke()` creates a `reader_core` runtime, sends a `runtime.ping` command,
captures the first JSON event, destroys the runtime, and returns the event JSON.
It is intentionally not a business API and does not implement remote reading.

Build with:

```bash
./scripts/build-android-jni.sh
```

The script fails closed when the Android NDK or Rust Android target is missing.
It does not mark Android JNI smoke as passed without a real NDK build.

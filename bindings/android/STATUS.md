# Android JNI SDK - Integration Status

Last updated: 2026-06-25
Baseline: consolidated from `codex/reader-core-runtime-protocol`,
`codex/reader-core-c-abi-stable-boundary`, and `codex/android-jni-sdk`.
Scope rule: Android files consume the C ABI; Android wrapper work must not set
Core semantics.

## Closed Loop

| Capability | Implementation | Current coverage |
|---|---|---|
| Runtime create / destroy | `ReaderCoreRuntime` over `rc_runtime_create` / `rc_runtime_destroy` through JNI handle ownership | Java wrapper plus native lifecycle implementation |
| Send command | `ReaderCoreRuntime.send` / `sendCommand` over `rc_runtime_send` | Java/Kotlin compile gate; native C++ syntax gate; NDK build gate |
| Poll / parse event | JNI callback copies event bytes into a native queue; Java polls `byte[]` or UTF-8 `String` | Java wrapper and samples |
| Cancel | `ReaderCoreRuntime.cancel` over `rc_runtime_cancel` | Java wrapper |
| `host.complete` / `host.error` | Java helpers build protocol JSON and send it through `rc_runtime_send` | Java/Kotlin samples parse `operationId` from `host.request` before completing |
| CMake / NDK build | `bindings/android/jni/CMakeLists.txt` plus root `build-android-jni.sh` | `arm64-v8a` and `x86_64` real NDK links passed |
| Java/Kotlin minimal calls | Samples under `bindings/android/samples/` plus direct listener sample under `bindings/android/sample/` | Java sample compile gate; Kotlin sample compile gate when `kotlinc` is available |
| Direct listener API | Public methods on the single Java `NativeCoreBridge` class plus Kotlin `ReaderEventListener` | Consolidated to avoid duplicate `com.reader.core.NativeCoreBridge` classes |

## Local Validation (2026-06-24)

Passed:

- `git diff --check -- bindings/android build-android-jni.sh`
- `bash -n build-android-jni.sh`
- `javac --release 8` for the Java wrapper and Java sample
- Android Studio bundled `kotlinc -jvm-target 1.8` for the Kotlin sample
- `c++ -std=c++17 -fsyntax-only` for `bindings/android/jni/reader_jni.cpp`
  using the Homebrew OpenJDK 17 JNI headers
- `./build-android-jni.sh`
  - Java wrapper/sample gate: passed
  - Kotlin sample gate: passed
  - Rust `reader-ffi` staticlib cross-build: passed for `aarch64-linux-android`
    and `x86_64-linux-android`
  - CMake/NDK JNI shared-library link: passed for `arm64-v8a` and `x86_64`
- Output verification:
  - `target/android-jni/libs/arm64-v8a/libreader_core_jni.so`: ELF64 AArch64
  - `target/android-jni/libs/x86_64/libreader_core_jni.so`: ELF64 x86-64
  - both `.so` files export `Java_com_reader_core_NativeCoreBridge_native*`
    JNI entry points and the Core ABI symbols used by the bridge

Consolidated-branch note:

- The JNI source now also exports direct listener entry points:
  `pingSmoke`, `runtimeCreate`, `runtimeSend`, `runtimeCancel`, and
  `runtimeDestroy`.
- Validation above was from the original lane. The consolidated branch still
  requires a fresh NDK build before Android can be treated as release evidence.

Toolchain installed during this lane:

- Android SDK CMake `3.22.1`
- Android NDK `26.3.11579264` (`android-ndk-r26d-darwin.zip`, direct download
  because `sdkmanager` hit a TLS handshake failure)
- Rust targets `aarch64-linux-android` and `x86_64-linux-android`

## ABI-gap Notes (recorded, not fixed)

1. **Events are callback-only.** ABI v1 has no `rc_runtime_poll`; the JNI layer
   therefore owns a native event queue populated from `rc_event_callback`.

2. **Host completion is protocol-level.** ABI v1 has no `rc_host_complete`;
   Android completes host work by sending `host.complete` / `host.error` JSON
   commands through `rc_runtime_send`.

3. **Structured last-error is now available in the consolidated C ABI, but not
   surfaced by Android yet.** `include/reader_core.h` exposes `rc_last_error`
   after the C ABI merge. Android still reports coarse statuses in
   `ReaderCoreException` until a follow-up wrapper change exposes that text.

4. **No Android app project is claimed.** This lane provides the JNI library,
   Java wrapper, and minimal Java/Kotlin call samples. Gradle packaging,
   Android UI, WebView, CookieManager, keystore, and network policy integration
   remain host-app work.

## Threading Notes

- Core invokes `rc_event_callback` from a Core-owned background thread. The
  queue-style Java wrapper copies event bytes immediately. The direct listener
  API invokes `ReaderEventListener.onEvent` from that Core-owned thread, so
  listener implementations must be thread-safe.
- `ReaderCoreRuntime.pollEvent(...)` can block the calling thread until an event
  arrives or the timeout elapses. Hosts should not call it on the UI thread for
  long waits.
- `ReaderCoreRuntime` synchronizes lifecycle, send, cancel, and poll calls on
  the Java object. Hosts needing higher throughput can add their own dispatcher
  above this minimal wrapper without changing the ABI.

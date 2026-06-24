# Android JNI SDK - Integration Status

Last updated: 2026-06-24
Baseline: `origin/codex/android-integration` (`084aed52879add17138d9849b0f58c23368e15f6`)
Scope rule: only `bindings/android/` and `build-android-jni.sh` are modified in
this lane. Core/FFI files are read-only.

## Closed loop

| Capability | Implementation | Current coverage |
|---|---|---|
| Runtime create / destroy | `ReaderCoreRuntime` over `rc_runtime_create` / `rc_runtime_destroy` through JNI handle ownership | Java wrapper plus native lifecycle implementation |
| Send command | `ReaderCoreRuntime.send` / `sendCommand` over `rc_runtime_send` | Java compile gate; native C++ syntax gate; NDK gate when toolchain is installed |
| Poll / parse event | JNI callback copies event bytes into a native queue; Java polls `byte[]` or UTF-8 `String` | Java wrapper and samples |
| Cancel | `ReaderCoreRuntime.cancel` over `rc_runtime_cancel` | Java wrapper |
| `host.complete` / `host.error` | Java helpers build protocol JSON and send it through `rc_runtime_send` | Java wrapper and samples |
| CMake / NDK build | `bindings/android/jni/CMakeLists.txt` plus root `build-android-jni.sh` | Fails closed without a real Android NDK link |
| Java/Kotlin minimal calls | Samples under `bindings/android/samples/` | Java sample compile gate; Kotlin sample compile gate when `kotlinc` is available |

## Local validation (2026-06-24)

Passed:

- `git diff --check -- bindings/android build-android-jni.sh`
- `bash -n build-android-jni.sh`
- `./build-android-jni.sh` reaches the script's Java/Kotlin gates, then
  fails closed at the native gate:
  - `checked Android Java wrapper/sample sources`
  - `checked Android Kotlin sample sources`
  - `missing CMake`
- `javac --release 8` for the Java wrapper and Java sample
- Android Studio bundled `kotlinc -jvm-target 1.8` for the Kotlin sample
- `c++ -std=c++17 -fsyntax-only` for `bindings/android/jni/reader_jni.cpp`
  using the Homebrew OpenJDK 17 JNI headers

Blocked:

- Native shared-library linking fails closed with `missing CMake`. This machine has
  `ANDROID_HOME=/Users/minliny/Library/Android/sdk`, but the SDK currently has
  no `cmake/` or `ndk/` directory, so the real NDK shared-library link was not
  executed in this pass.

## ABI-gap notes (recorded, not fixed)

1. **Events are callback-only.** ABI v1 has no `rc_runtime_poll`; the JNI layer
   therefore owns a native event queue populated from `rc_event_callback`.

2. **Host completion is protocol-level.** ABI v1 has no `rc_host_complete`;
   Android completes host work by sending `host.complete` / `host.error` JSON
   commands through `rc_runtime_send`.

3. **`rc_last_error` is per-thread.** The Java wrapper reads it immediately
   after a failing `rc_runtime_create` / `rc_runtime_send` JNI call. Successful
   ABI calls clear the slot on the calling thread.

4. **No Android app project is claimed.** This lane provides the JNI library,
   Java wrapper, and minimal Java/Kotlin call samples. Gradle packaging,
   Android UI, WebView, CookieManager, keystore, and network policy integration
   remain host-app work.

## Threading notes

- Core invokes `rc_event_callback` from a Core-owned background thread. JNI
  copies event bytes immediately and never calls Java from that callback.
- `ReaderCoreRuntime.pollEvent(...)` can block the calling thread until an event
  arrives or the timeout elapses. Hosts should not call it on the UI thread for
  long waits.
- `ReaderCoreRuntime` synchronizes lifecycle, send, cancel, and poll calls on
  the Java object. Hosts needing higher throughput can add their own dispatcher
  above this minimal wrapper without changing the ABI.

# Android JNI SDK — STATUS

Long-term goal: a stable Android JNI SDK that drives Reader-Core.

**Baseline:** `origin/codex/android-integration` (commit `084aed5`).
**Allowed paths:** `bindings/android/`, `scripts/build-android-jni.sh`.
**Hard constraint:** never modify `crates/reader-*` (Core/FFI). ABI gaps are recorded here, not patched into Core.

## C ABI surface consumed (include/reader_core.h, ABI v1)

| Symbol | Purpose |
|---|---|
| `rc_abi_version()` | compile/load-time version check |
| `rc_runtime_create(config, len, cb, ctx, &out)` | lifecycle: create |
| `rc_runtime_send(rt, cmd, len)` | platform → Core command (incl. `host.complete`, `host.error`) |
| `rc_runtime_cancel(rt, request_id)` | cancel pending request |
| `rc_runtime_destroy(rt)` | lifecycle: destroy |

Note: `rc_last_error` / `rc_error_code_t` are NOT available on the baseline —
see the ABI-gap ledger below.

Event callback (`rc_event_callback`) delivers three event types from Core:
`result`, `error`, `host.request` (see `protocol/reader-event.schema.json`).

## Deliverables roadmap

1. **JNI lifecycle** — persistent handle across calls (create/send/cancel/destroy). ✅ this increment
2. **Command/event bridge** — Java `ReaderEventListener` receives every Core event on the Core thread; `runtimeSend` lets Java issue any command. ✅ this increment
3. **`host.complete` wiring** — `host.request` events surface to Java; Java answers via `runtimeSend` with a `host.complete`/`host.error` command. ✅ supported this increment (no ABI change needed)
4. **CMake/NDK build** — `bindings/android/CMakeLists.txt` + `scripts/build-android-jni.sh` driven by CMake. ✅ this increment
5. **Minimal Java/Kotlin sample** — `bindings/android/sample/`. ✅ this increment
6. **Extra ABIs** (armeabi-v7a, x86_64) — ⛔ not yet; build script currently gates to `arm64-v8a`.
7. **CI gate / instrumented test** — ⛔ not yet; only the fail-closed build script exists.

## ABI-gap ledger

Gaps that block Android but must NOT be fixed in Core/FFI. Recorded here, not
patched into Core.

| ID | Gap | Impact | Workaround |
|---|---|---|---|
| ANDROID-ABI-1 | Baseline FFI (`origin/codex/android-integration`) does not expose `rc_last_error` / `rc_error_code_t`. (It was added later on `codex/rule-engine-parity`.) | The JNI layer cannot surface a structured, human-readable message for synchronous call failures (`runtimeCreate`/`runtimeSend` returning non-zero). | Java branches on the coarse `int` return code. Async failures still arrive as structured `error` events on the listener (`{code,message,retryable}`). `NativeCoreBridge.lastError()` is intentionally NOT exposed until Core exposes `rc_last_error`; do not re-add it without removing this gap. |

### Non-gaps (confirmed achievable with ABI v1)

- **`host.complete`** is NOT an ABI gap. Core emits a `host.request` event
  (`{type:"host.request", operationId, capability, params}`); the platform
  answers by sending a `host.complete` (`{method:"host.complete",
  params:{operationId, result}}`) or `host.error` command back through
  `rc_runtime_send`. The JNI bridge exposes both directions, so this is
  fully implementable without touching Core.
- **Threaded event delivery**: `rc_event_callback` fires on a Core-owned
  background thread. The JNI layer attaches that thread to the JVM
  (`AttachCurrentThread`) and holds a global ref to the listener; no Core
  change required.

## Verification

- `scripts/build-android-jni.sh` fails closed when the NDK or Rust Android
  target is missing (unchanged contract). With both present it produces
  `target/android-jni/<abi>/libreader_core_jni.so` via CMake.
- No Android instrumented test runs in CI yet (tracked above).

# Harmony NAPI Status

## Scope

This status file tracks the Harmony wrapper only. The current work is limited
to `bindings/harmony/**`, `scripts/build-harmony-napi.sh`, and
`scripts/build-ohos.sh`.

## Closed In This Line

- Runtime lifecycle: NAPI can create and release a Reader-Core runtime handle.
- Lifecycle smoke: NAPI exposes `lifecycleSmoke(iterations)` to repeatedly
  create a runtime, send `runtime.ping`, read a result event, and destroy the
  runtime.
- Command/event path: NAPI can send JSON commands and read copied Core events
  from a thread-safe queue.
- Cancellation: NAPI exposes `cancelRequest`, backed by `rc_runtime_cancel`.
- Host bus minimum loop: `host.request` can be read and answered with
  `host.complete`; the SDK helper can auto-complete host requests while waiting
  for the original request result.
- Interleaved event handling: the SDK keeps unrelated events queued while
  waiting for a specific request result, so a pending `host.request` from
  another request does not break the current command/result flow.
- Host error path: NAPI exposes `failHostRequest`, backed by the `host.error`
  JSON command path; the SDK sends `host.error` automatically if a host request
  handler throws.
- SDK behavior smoke: `bindings/harmony/sdk/reader_core.test.ts` uses a fake
  native module to verify `runtime.ping`, `host.complete`, handler failure to
  `host.error`, unrelated event queuing, and `cancelRequest` dispatch.
- ArkTS package entry: `bindings/harmony/Index.ets` imports
  `libreader_core_napi.so` and exposes `createReaderCoreRuntime` plus
  `runHarmonyNapiSmoke`.
- Build evidence: OHOS and Harmony scripts emit deterministic artifact paths,
  SHA-256 hashes, byte sizes, tool versions, NAPI symbol evidence, and a
  package-ready Harmony directory manifest.

## Current SDK Surface

- Native NAPI exports: `abiVersion`, `createRuntime`, `releaseRuntime`,
  `sendCommand`, `cancelRequest`, `readEvent`, `pendingEventCount`,
  `completeHostRequest`, `failHostRequest`, `pingSmoke`, `hostSmoke`, and
  `lifecycleSmoke`.
- TypeScript/ArkTS wrapper: `bindings/harmony/sdk/reader_core.ts` wraps native
  exports into `ReaderCoreRuntime`, including `coreInfo`, `ping`, `hostSmoke`,
  generic `request`, explicit `readEvent`, explicit `completeHostRequest`, and
  explicit `failHostRequest`.
- Package entry: `bindings/harmony/oh-package.json5` points to `Index.ets`.
- Package artifact: `scripts/build-harmony-napi.sh` assembles
  `target/harmony-napi/arm64-v8a/package` with the `.so`, ArkTS entry, SDK, and
  status/readme files, then emits `harmony-package-manifest.sha256`.

## ABI Constraints

- ABI v1 returns only integer status codes from `rc_runtime_create`,
  `rc_runtime_send`, and `rc_runtime_cancel`. Harmony cannot expose a structured
  synchronous failure object unless Core/FFI adds a last-error or direct
  out-buffer ABI. This is not changed here because Core/FFI edits are forbidden.
- ABI v1 events are callback-only borrowed buffers. Harmony copies event bytes
  into a NAPI-owned queue before the callback returns, then exposes polling
  through `readEvent`.
- `host.complete` is intentionally sent through the JSON command protocol via
  `rc_runtime_send`; there is no separate C ABI function for host completion in
  v1.
- `host.error` follows the same v1 constraint: Harmony sends it through
  `rc_runtime_send`; there is no separate C ABI function for host failure.

## Open Harmony Work

- Add device-side smoke tests that import the `.so`, run `coreInfo`, `ping`, and
  `hostSmoke`, and archive the script output beside the build evidence. The
  repo now provides `runHarmonyNapiSmoke`; the remaining work is running it in
  a signed HAP on device.

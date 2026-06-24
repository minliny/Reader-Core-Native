# Reader-Core Harmony Binding

This package is the Harmony-side wrapper for `libreader_core_napi.so`.

## Files

- `native/reader_napi.cpp`: NAPI bridge that owns runtime handles, event queue
  copies, command send, cancellation, and host request completion/failure.
- `sdk/reader_core.ts`: typed SDK wrapper around the native exports.
- `sdk/reader_core.test.ts`: fake-native SDK smoke tests runnable with Bun.
- `Index.ets`: ArkTS entry point that imports `libreader_core_napi.so` and
  exposes `createReaderCoreRuntime` plus `runHarmonyNapiSmoke`.
- `STATUS.md`: current integration status and ABI constraints.

## Device Smoke Entry

After packaging `libreader_core_napi.so` with the Harmony app, call:

```ts
import { runHarmonyNapiSmoke } from '@reader/core-harmony';

const result = await runHarmonyNapiSmoke();
```

The smoke creates a runtime, calls `core.info`, calls `runtime.ping`, exercises
`runtime.hostSmoke` through `host.request` and `host.complete`, then releases
the runtime.

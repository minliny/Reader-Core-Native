# Reader-Core Harmony Binding

This package is the Harmony-side wrapper for `libreader_core_napi.so`.

## Files

- `native/reader_napi.cpp`: NAPI bridge that owns runtime handles, event queue
  copies, command send, cancellation, and host request completion/failure.
- `sdk/reader_core.ts`: typed SDK wrapper around the native exports.
- `sdk/smoke_report.ts`: validates smoke output and formats a device-log report.
- `sdk/reader_core.test.ts`: fake-native SDK smoke tests runnable with Bun.
- `Index.ets`: ArkTS entry point that imports `libreader_core_napi.so` and
  exposes `createReaderCoreRuntime`, `runHarmonyNapiSmoke`,
  `captureHarmonyNapiSmokeReport`, and `runHarmonyNapiSmokeReport`.
- `STATUS.md`: current integration status and ABI constraints.

## Build Output

`scripts/build-harmony-napi.sh` emits a package-ready directory at:

```text
target/harmony-napi/arm64-v8a/package
```

The directory contains `oh-package.json5`, `Index.ets`, non-test SDK `.ts`
files, `README.md`, `STATUS.md`, and `libs/arm64-v8a/libreader_core_napi.so`.
The same build also writes
`target/harmony-napi/arm64-v8a/harmony-package-manifest.sha256` with a
deterministic SHA-256 and byte-size line for every package file.

## Device Smoke Entry

After packaging `libreader_core_napi.so` with the Harmony app, call:

```ts
import {
  captureHarmonyNapiSmokeReport,
  formatHarmonyNapiSmokeReport,
} from '@reader/core-harmony';

const report = await captureHarmonyNapiSmokeReport();
const output = formatHarmonyNapiSmokeReport(report);
```

The smoke creates a runtime, runs native `lifecycleSmoke`, calls `core.info`,
calls `runtime.ping`, exercises `runtime.hostSmoke` through `host.request` and
`host.complete`, validates the result shape, then releases the runtime.
`captureHarmonyNapiSmokeReport` returns a structured failure report if native
loading or runtime execution throws, so device logs can still archive a JSON
result. Use `runHarmonyNapiSmokeReport` when the caller should throw on any
failed smoke check. Archive the formatted report output next to the local build
evidence when the signed HAP is run on device.

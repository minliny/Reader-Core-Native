import readerCoreNapi from 'libreader_core_napi.so';

import {
  ReaderCoreRuntime,
  type JsonObject,
  type NativeReaderCoreModule,
} from './sdk/reader_core';
import {
  assertHarmonyNapiSmokeReport,
  buildHarmonyNapiSmokeReport,
  type HarmonyNapiSmokeReport,
  type HarmonyNapiSmokeResult,
} from './sdk/smoke_report';

export {
  ReaderCoreRequestError,
  ReaderCoreRuntime,
  parseReaderCoreEvent,
  type HostRequestHandler,
  type JsonObject,
  type NativeReaderCoreModule,
  type NativeRuntimeHandle,
  type ReaderCoreCommand,
  type ReaderCoreError,
  type ReaderCoreErrorEvent,
  type ReaderCoreEvent,
  type ReaderCoreHostRequestEvent,
  type ReaderCoreResultEvent,
  type RequestOptions,
} from './sdk/reader_core';
export {
  assertHarmonyNapiSmokeReport,
  buildHarmonyNapiSmokeReport,
  formatHarmonyNapiSmokeReport,
  type HarmonyNapiSmokeCheck,
  type HarmonyNapiSmokeCheckName,
  type HarmonyNapiSmokeReport,
  type HarmonyNapiSmokeResult,
} from './sdk/smoke_report';

const nativeReaderCore = readerCoreNapi as NativeReaderCoreModule;

export function createReaderCoreRuntime(config: JsonObject = {}): ReaderCoreRuntime {
  return new ReaderCoreRuntime(nativeReaderCore, config);
}

export async function runHarmonyNapiSmoke(
  config: JsonObject = {}
): Promise<HarmonyNapiSmokeResult> {
  const runtime = createReaderCoreRuntime(config);
  try {
    const nativeLifecycle = JSON.parse(nativeReaderCore.lifecycleSmoke(8)) as JsonObject;
    const coreInfo = await runtime.coreInfo();
    const ping = await runtime.ping();
    const hostSmoke = await runtime.hostSmoke();
    return {
      abiVersion: runtime.abiVersion,
      nativeLifecycle,
      coreInfo,
      ping,
      hostSmoke,
    };
  } finally {
    runtime.close();
  }
}

export async function runHarmonyNapiSmokeReport(
  config: JsonObject = {}
): Promise<HarmonyNapiSmokeReport> {
  const report = buildHarmonyNapiSmokeReport(await runHarmonyNapiSmoke(config));
  assertHarmonyNapiSmokeReport(report);
  return report;
}

import type { JsonObject, ReaderCoreResultEvent } from "./reader_core";

export type HarmonyNapiSmokeResult = {
  abiVersion: number;
  nativeLifecycle: JsonObject;
  coreInfo: ReaderCoreResultEvent;
  ping: ReaderCoreResultEvent;
  hostSmoke: ReaderCoreResultEvent;
};

export type HarmonyNapiSmokeCheckName =
  | "execution"
  | "abiVersion"
  | "native.lifecycle"
  | "core.info"
  | "runtime.ping"
  | "runtime.hostSmoke";

export type HarmonyNapiSmokeCheck = {
  name: HarmonyNapiSmokeCheckName;
  pass: boolean;
  detail: string;
};

export type HarmonyNapiSmokeError = {
  name: string;
  message: string;
};

export type HarmonyNapiSmokeReport = {
  schemaVersion: 1;
  status: "pass" | "fail";
  checks: HarmonyNapiSmokeCheck[];
  result?: HarmonyNapiSmokeResult;
  error?: HarmonyNapiSmokeError;
};

export function buildHarmonyNapiSmokeReport(
  result: HarmonyNapiSmokeResult
): HarmonyNapiSmokeReport {
  const checks: HarmonyNapiSmokeCheck[] = [
    check(
      "abiVersion",
      Number.isSafeInteger(result.abiVersion) && result.abiVersion > 0,
      `abiVersion=${String(result.abiVersion)}`
    ),
    buildNativeLifecycleCheck(result.nativeLifecycle),
    check(
      "core.info",
      isResultEvent(result.coreInfo),
      eventDetail(result.coreInfo)
    ),
    check(
      "runtime.ping",
      isResultEvent(result.ping) && result.ping.data.pong === true,
      eventDetail(result.ping)
    ),
    check(
      "runtime.hostSmoke",
      isResultEvent(result.hostSmoke) &&
        result.hostSmoke.data.status === "ok" &&
        result.hostSmoke.data.capability === "host.smoke.echo",
      eventDetail(result.hostSmoke)
    ),
  ];

  return {
    schemaVersion: 1,
    status: checks.every((item) => item.pass) ? "pass" : "fail",
    checks,
    result,
  };
}

export function buildHarmonyNapiSmokeErrorReport(error: unknown): HarmonyNapiSmokeReport {
  const normalized = normalizeSmokeError(error);
  return {
    schemaVersion: 1,
    status: "fail",
    checks: [
      check(
        "execution",
        false,
        `${normalized.name}: ${normalized.message}`
      ),
    ],
    error: normalized,
  };
}

export function assertHarmonyNapiSmokeReport(report: HarmonyNapiSmokeReport): void {
  if (report.status === "pass" && report.checks.every((item) => item.pass)) {
    return;
  }

  const failed = report.checks
    .filter((item) => !item.pass)
    .map((item) => `${item.name}: ${item.detail}`)
    .join("; ");
  throw new Error(`Harmony NAPI smoke failed: ${failed}`);
}

export function formatHarmonyNapiSmokeReport(report: HarmonyNapiSmokeReport): string {
  return JSON.stringify(report, null, 2);
}

function buildNativeLifecycleCheck(nativeLifecycle: JsonObject): HarmonyNapiSmokeCheck {
  const iterations = nativeLifecycle.iterations;
  const lastEvent = nativeLifecycle.lastEvent;
  const pass =
    Number.isSafeInteger(iterations) &&
    typeof iterations === "number" &&
    iterations > 0 &&
    isResultEvent(lastEvent) &&
    lastEvent.requestId === iterations &&
    lastEvent.data.pong === true;

  return check(
    "native.lifecycle",
    pass,
    `iterations=${String(iterations)} lastEvent=${eventDetail(lastEvent)}`
  );
}

function check(
  name: HarmonyNapiSmokeCheckName,
  pass: boolean,
  detail: string
): HarmonyNapiSmokeCheck {
  return { name, pass, detail };
}

function isResultEvent(value: unknown): value is ReaderCoreResultEvent {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const event = value as Partial<ReaderCoreResultEvent>;
  return (
    event.protocolVersion === 1 &&
    Number.isSafeInteger(event.requestId) &&
    event.type === "result" &&
    typeof event.data === "object" &&
    event.data !== null
  );
}

function eventDetail(value: unknown): string {
  if (typeof value !== "object" || value === null) {
    return "event=<invalid>";
  }
  const event = value as Partial<ReaderCoreResultEvent>;
  return `type=${String(event.type)} requestId=${String(event.requestId)}`;
}

function normalizeSmokeError(error: unknown): HarmonyNapiSmokeError {
  if (error instanceof Error) {
    return {
      name: error.name || "Error",
      message: error.message || "unknown error",
    };
  }

  return {
    name: "Error",
    message: typeof error === "string" ? error : String(error),
  };
}

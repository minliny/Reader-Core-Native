import { describe, expect, test } from "bun:test";

import {
  assertHarmonyNapiSmokeReport,
  buildHarmonyNapiSmokeErrorReport,
  buildHarmonyNapiSmokeReport,
  formatHarmonyNapiSmokeReport,
  type HarmonyNapiSmokeResult,
} from "./smoke_report";

function passingSmokeResult(): HarmonyNapiSmokeResult {
  return {
    abiVersion: 1,
    nativeLifecycle: {
      iterations: 8,
      lastEvent: {
        protocolVersion: 1,
        requestId: 8,
        type: "result",
        data: { pong: true },
      },
    },
    coreInfo: {
      protocolVersion: 1,
      requestId: 1,
      type: "result",
      data: { name: "Reader-Core" },
    },
    ping: {
      protocolVersion: 1,
      requestId: 2,
      type: "result",
      data: { pong: true },
    },
    hostSmoke: {
      protocolVersion: 1,
      requestId: 3,
      type: "result",
      data: { status: "ok", capability: "host.smoke.echo" },
    },
  };
}

describe("Harmony NAPI smoke report", () => {
  test("marks a complete runtime smoke result as pass", () => {
    const report = buildHarmonyNapiSmokeReport(passingSmokeResult());

    expect(report.schemaVersion).toBe(1);
    expect(report.status).toBe("pass");
    expect(report.checks.every((item) => item.pass)).toBe(true);
    expect(() => assertHarmonyNapiSmokeReport(report)).not.toThrow();
  });

  test("marks an invalid host smoke result as fail", () => {
    const result = passingSmokeResult();
    result.hostSmoke.data.status = "failed";

    const report = buildHarmonyNapiSmokeReport(result);

    expect(report.status).toBe("fail");
    expect(report.checks.find((item) => item.name === "runtime.hostSmoke")).toMatchObject({
      pass: false,
    });
    expect(() => assertHarmonyNapiSmokeReport(report)).toThrow("runtime.hostSmoke");
  });

  test("formats a parseable report for device log archival", () => {
    const report = buildHarmonyNapiSmokeReport(passingSmokeResult());
    const formatted = formatHarmonyNapiSmokeReport(report);

    expect(JSON.parse(formatted)).toMatchObject({
      schemaVersion: 1,
      status: "pass",
    });
  });

  test("builds a structured failure report when smoke execution throws", () => {
    const report = buildHarmonyNapiSmokeErrorReport(new Error("native module unavailable"));

    expect(report).toMatchObject({
      schemaVersion: 1,
      status: "fail",
      checks: [{ name: "execution", pass: false }],
      error: {
        name: "Error",
        message: "native module unavailable",
      },
    });
    expect(() => assertHarmonyNapiSmokeReport(report)).toThrow("execution");
  });
});

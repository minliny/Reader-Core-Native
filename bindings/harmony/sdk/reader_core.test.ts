import { describe, expect, test } from "bun:test";

import {
  parseReaderCoreEvent,
  ReaderCoreRequestError,
  ReaderCoreRuntime,
  type JsonObject,
  type NativeReaderCoreModule,
  type NativeRuntimeHandle,
  type ReaderCoreCommand,
  type ReaderCoreEvent,
  type ReaderCoreHostRequestEvent,
} from "./reader_core";

type FakeRuntime = {
  released: boolean;
  events: string[];
  operations: Map<number, number>;
  nextOperationId: number;
};

class FakeNativeReaderCore implements NativeReaderCoreModule {
  readonly runtimes = new Set<FakeRuntime>();
  readonly cancelled: number[] = [];

  abiVersion(): number {
    return 1;
  }

  createRuntime(_config?: JsonObject | string): NativeRuntimeHandle {
    const runtime: FakeRuntime = {
      released: false,
      events: [],
      operations: new Map(),
      nextOperationId: 1,
    };
    this.runtimes.add(runtime);
    return runtime;
  }

  releaseRuntime(runtime: NativeRuntimeHandle): void {
    this.asRuntime(runtime).released = true;
  }

  sendCommand(runtime: NativeRuntimeHandle, command: JsonObject | string): void {
    const rt = this.asRuntime(runtime);
    const parsed = typeof command === "string" ? JSON.parse(command) : command;
    const request = parsed as ReaderCoreCommand;

    if (request.method === "runtime.ping") {
      this.pushEvent(rt, {
        protocolVersion: 1,
        requestId: request.requestId,
        type: "result",
        data: { pong: true },
      });
      return;
    }

    if (request.method === "runtime.hostSmoke") {
      const operationId = rt.nextOperationId++;
      rt.operations.set(operationId, request.requestId);
      this.pushEvent(rt, {
        protocolVersion: 1,
        requestId: request.requestId,
        type: "host.request",
        operationId,
        capability: "host.smoke.echo",
        params: { source: "fake-native" },
      });
      return;
    }

    throw new Error(`unexpected command: ${request.method}`);
  }

  cancelRequest(_runtime: NativeRuntimeHandle, requestId: number): void {
    this.cancelled.push(requestId);
  }

  readEvent(runtime: NativeRuntimeHandle, _timeoutMs?: number): string | null {
    return this.asRuntime(runtime).events.shift() ?? null;
  }

  pendingEventCount(runtime: NativeRuntimeHandle): number {
    return this.asRuntime(runtime).events.length;
  }

  completeHostRequest(
    runtime: NativeRuntimeHandle,
    operationId: number,
    result: JsonObject | string,
    _requestId?: number
  ): void {
    const rt = this.asRuntime(runtime);
    const originalRequestId = this.takeOriginalRequestId(rt, operationId);
    const data = typeof result === "string" ? JSON.parse(result) : result;
    this.pushEvent(rt, {
      protocolVersion: 1,
      requestId: originalRequestId,
      type: "result",
      data,
    });
  }

  failHostRequest(
    runtime: NativeRuntimeHandle,
    operationId: number,
    error: JsonObject | string,
    _requestId?: number
  ): void {
    const rt = this.asRuntime(runtime);
    const originalRequestId = this.takeOriginalRequestId(rt, operationId);
    const parsed = typeof error === "string" ? JSON.parse(error) : error;
    this.pushEvent(rt, {
      protocolVersion: 1,
      requestId: originalRequestId,
      type: "error",
      error: {
        code: String(parsed.code ?? "INTERNAL"),
        message: String(parsed.message ?? "host request failed"),
        retryable: Boolean(parsed.retryable),
      },
    });
  }

  pingSmoke(): string {
    return JSON.stringify({
      protocolVersion: 1,
      requestId: 1,
      type: "result",
      data: { pong: true },
    });
  }

  hostSmoke(): string {
    return JSON.stringify({
      hostRequest: {
        protocolVersion: 1,
        requestId: 1,
        type: "host.request",
        operationId: 1,
        capability: "host.smoke.echo",
        params: {},
      },
      completion: {
        protocolVersion: 1,
        requestId: 1,
        type: "result",
        data: { status: "ok" },
      },
    });
  }

  lifecycleSmoke(iterations = 8): string {
    return JSON.stringify({
      iterations,
      lastEvent: {
        protocolVersion: 1,
        requestId: iterations,
        type: "result",
        data: { pong: true },
      },
    });
  }

  private pushEvent(runtime: FakeRuntime, event: ReaderCoreEvent): void {
    runtime.events.push(JSON.stringify(event));
  }

  private asRuntime(runtime: NativeRuntimeHandle): FakeRuntime {
    return runtime as FakeRuntime;
  }

  private takeOriginalRequestId(runtime: FakeRuntime, operationId: number): number {
    const originalRequestId = runtime.operations.get(operationId);
    if (originalRequestId === undefined) {
      throw new Error(`unknown operation: ${operationId}`);
    }
    runtime.operations.delete(operationId);
    return originalRequestId;
  }
}

describe("ReaderCoreRuntime", () => {
  test("requests runtime.ping and returns the matching result", async () => {
    const native = new FakeNativeReaderCore();
    const runtime = new ReaderCoreRuntime(native);

    const event = await runtime.ping();

    expect(event.type).toBe("result");
    expect(event.data.pong).toBe(true);
  });

  test("auto-completes host.request through host.complete", async () => {
    const native = new FakeNativeReaderCore();
    const runtime = new ReaderCoreRuntime(native);

    const event = await runtime.hostSmoke();

    expect(event.type).toBe("result");
    expect(event.data.status).toBe("ok");
    expect(event.data.capability).toBe("host.smoke.echo");
  });

  test("keeps unrelated host.request queued while waiting for a result", async () => {
    const native = new FakeNativeReaderCore();
    const runtime = new ReaderCoreRuntime(native);

    const hostRequestId = runtime.send("runtime.hostSmoke");
    const pingRequestId = runtime.send("runtime.ping");

    const event = await runtime.waitForResult(pingRequestId);

    expect(event.type).toBe("result");
    expect(event.data.pong).toBe(true);

    const queued = runtime.readEvent();
    expect(queued).toMatchObject({
      type: "host.request",
      requestId: hostRequestId,
      capability: "host.smoke.echo",
    });
  });

  test("turns host handler failures into host.error and surfaces core error", async () => {
    const native = new FakeNativeReaderCore();
    const runtime = new ReaderCoreRuntime(native);

    const promise = runtime.request(
      "runtime.hostSmoke",
      {},
      {
        hostRequest: (_event: ReaderCoreHostRequestEvent) => {
          throw new Error("network unavailable");
        },
      }
    );

    await expect(promise).rejects.toBeInstanceOf(ReaderCoreRequestError);
    await expect(promise).rejects.toMatchObject({
      event: {
        error: {
          code: "INTERNAL",
          message: "network unavailable",
          retryable: false,
        },
      },
    });
  });

  test("passes cancellation through to native runtime", () => {
    const native = new FakeNativeReaderCore();
    const runtime = new ReaderCoreRuntime(native);

    runtime.cancel(42);

    expect(native.cancelled).toEqual([42]);
  });

  test("exposes native lifecycle smoke result shape", () => {
    const native = new FakeNativeReaderCore();

    const result = JSON.parse(native.lifecycleSmoke(3));

    expect(result.iterations).toBe(3);
    expect(result.lastEvent.type).toBe("result");
    expect(result.lastEvent.requestId).toBe(3);
  });

  test("rejects malformed native event payloads at the SDK boundary", () => {
    expect(() =>
      parseReaderCoreEvent(
        JSON.stringify({
          protocolVersion: 1,
          requestId: 1,
          type: "result",
        })
      )
    ).toThrow("invalid Reader-Core result event");

    expect(() =>
      parseReaderCoreEvent(
        JSON.stringify({
          protocolVersion: 1,
          requestId: 1,
          type: "error",
          error: { code: "INTERNAL", message: "failed" },
        })
      )
    ).toThrow("invalid Reader-Core error event");

    expect(() =>
      parseReaderCoreEvent(
        JSON.stringify({
          protocolVersion: 1,
          requestId: 1,
          type: "host.request",
          operationId: 1,
          capability: "",
          params: {},
        })
      )
    ).toThrow("invalid Reader-Core host.request event");
  });
});

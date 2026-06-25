export type JsonObject = { [key: string]: unknown };

export type NativeRuntimeHandle = unknown;

export type NativeReaderCoreModule = {
  abiVersion(): number;
  createRuntime(config?: JsonObject | string): NativeRuntimeHandle;
  releaseRuntime(runtime: NativeRuntimeHandle): void;
  sendCommand(runtime: NativeRuntimeHandle, command: JsonObject | string): void;
  cancelRequest(runtime: NativeRuntimeHandle, requestId: number): void;
  readEvent(runtime: NativeRuntimeHandle, timeoutMs?: number): string | null;
  pendingEventCount(runtime: NativeRuntimeHandle): number;
  completeHostRequest(
    runtime: NativeRuntimeHandle,
    operationId: number,
    result: JsonObject | string,
    requestId?: number
  ): void;
  failHostRequest(
    runtime: NativeRuntimeHandle,
    operationId: number,
    error: ReaderCoreError | JsonObject | string,
    requestId?: number
  ): void;
  pingSmoke(): string;
  hostSmoke(): string;
  lifecycleSmoke(iterations?: number): string;
};

export type ReaderCoreCommand = {
  protocolVersion: 1;
  requestId: number;
  method: string;
  params?: JsonObject;
};

export type ReaderCoreResultEvent = {
  protocolVersion: 1;
  requestId: number;
  type: "result";
  data: JsonObject;
};

export type ReaderCoreError = {
  code: string;
  message: string;
  retryable: boolean;
  details?: JsonObject;
};

export type ReaderCoreErrorEvent = {
  protocolVersion: 1;
  requestId: number;
  type: "error";
  error: ReaderCoreError;
};

export type ReaderCoreHostRequestEvent = {
  protocolVersion: 1;
  requestId: number;
  type: "host.request";
  operationId: number;
  capability: string;
  params: JsonObject;
};

export type ReaderCoreEvent =
  | ReaderCoreResultEvent
  | ReaderCoreErrorEvent
  | ReaderCoreHostRequestEvent;

export type HostRequestHandler = (
  event: ReaderCoreHostRequestEvent
) => JsonObject | Promise<JsonObject>;

export type RequestOptions = {
  timeoutMs?: number;
  pollMs?: number;
  hostRequest?: HostRequestHandler;
};

export class ReaderCoreRuntime {
  static readonly protocolVersion = 1;

  private readonly native: NativeReaderCoreModule;
  private readonly runtime: NativeRuntimeHandle;
  private readonly pendingEvents: ReaderCoreEvent[] = [];
  private nextRequestId = 1;
  private closed = false;

  constructor(nativeModule: NativeReaderCoreModule, config: JsonObject = {}) {
    this.native = nativeModule;
    this.runtime = nativeModule.createRuntime(config);
  }

  get abiVersion(): number {
    return this.native.abiVersion();
  }

  get pendingEventCount(): number {
    return this.pendingEvents.length + this.native.pendingEventCount(this.runtime);
  }

  close(): void {
    if (this.closed) {
      return;
    }
    this.native.releaseRuntime(this.runtime);
    this.pendingEvents.length = 0;
    this.closed = true;
  }

  send(method: string, params: JsonObject = {}, requestId = this.allocateRequestId()): number {
    this.ensureOpen();
    assertCommandMethod(method);
    assertJsonObjectValue(params, "params");
    assertNonNegativeSafeInteger(requestId, "requestId");
    const command: ReaderCoreCommand = {
      protocolVersion: ReaderCoreRuntime.protocolVersion,
      requestId,
      method,
      params,
    };
    this.native.sendCommand(this.runtime, command);
    return requestId;
  }

  cancel(requestId: number): void {
    this.ensureOpen();
    assertNonNegativeSafeInteger(requestId, "requestId");
    this.native.cancelRequest(this.runtime, requestId);
  }

  readEvent(timeoutMs = 0): ReaderCoreEvent | null {
    this.ensureOpen();
    assertNonNegativeSafeInteger(timeoutMs, "timeoutMs");
    const queued = this.pendingEvents.shift();
    if (queued !== undefined) {
      return queued;
    }

    return this.readNativeEvent(timeoutMs);
  }

  completeHostRequest(
    eventOrOperationId: ReaderCoreHostRequestEvent | number,
    result: JsonObject,
    requestId?: number
  ): void {
    this.ensureOpen();
    const operationId =
      typeof eventOrOperationId === "number"
        ? eventOrOperationId
        : eventOrOperationId.operationId;
    assertNonNegativeSafeInteger(operationId, "operationId");
    if (requestId !== undefined) {
      assertNonNegativeSafeInteger(requestId, "requestId");
    }
    assertJsonObjectValue(result, "host.complete result");
    this.native.completeHostRequest(this.runtime, operationId, result, requestId);
  }

  failHostRequest(
    eventOrOperationId: ReaderCoreHostRequestEvent | number,
    error: ReaderCoreError | Error | string,
    requestId?: number
  ): void {
    this.ensureOpen();
    const operationId =
      typeof eventOrOperationId === "number"
        ? eventOrOperationId
        : eventOrOperationId.operationId;
    assertNonNegativeSafeInteger(operationId, "operationId");
    if (requestId !== undefined) {
      assertNonNegativeSafeInteger(requestId, "requestId");
    }
    this.native.failHostRequest(this.runtime, operationId, normalizeHostError(error), requestId);
  }

  async request(
    method: string,
    params: JsonObject = {},
    options: RequestOptions = {}
  ): Promise<ReaderCoreResultEvent> {
    const requestId = this.send(method, params);
    return this.waitForResult(requestId, options);
  }

  async waitForResult(
    requestId: number,
    options: RequestOptions = {}
  ): Promise<ReaderCoreResultEvent> {
    this.ensureOpen();
    const timeoutMs = readTimeoutMs(options.timeoutMs);
    const pollMs = readPollMs(options.pollMs);
    const deadline = Date.now() + timeoutMs;

    while (Date.now() <= deadline) {
      const event =
        this.takePendingForRequest(requestId) ??
        this.readNativeEvent(Math.min(pollMs, Math.max(0, deadline - Date.now())));
      if (event === null) {
        await delay(0);
        continue;
      }

      if (event.type === "host.request") {
        if (event.requestId !== requestId) {
          this.pendingEvents.push(event);
          await delay(0);
          continue;
        }
        if (options.hostRequest === undefined) {
          this.pendingEvents.push(event);
          throw new Error(`Reader-Core host.request requires a handler: ${event.operationId}`);
        }
        try {
          const result = await options.hostRequest(event);
          this.completeHostRequest(event, result);
        } catch (error) {
          this.failHostRequest(event, normalizeHostError(error));
        }
        continue;
      }

      if (event.requestId === requestId) {
        if (event.type === "error") {
          throw new ReaderCoreRequestError(event);
        }
        return event;
      }

      this.pendingEvents.push(event);
      await delay(0);
    }

    throw new Error(`Reader-Core request timed out: ${requestId}`);
  }

  async coreInfo(timeoutMs = 2000): Promise<ReaderCoreResultEvent> {
    return this.request("core.info", {}, { timeoutMs });
  }

  async ping(timeoutMs = 2000): Promise<ReaderCoreResultEvent> {
    return this.request("runtime.ping", {}, { timeoutMs });
  }

  async hostSmoke(timeoutMs = 2000): Promise<ReaderCoreResultEvent> {
    return this.request(
      "runtime.hostSmoke",
      { capability: "host.smoke.echo", params: { source: "harmony-sdk" } },
      {
        timeoutMs,
        hostRequest: (event) => ({
          status: "ok",
          capability: event.capability,
          params: event.params,
        }),
      }
    );
  }

  private allocateRequestId(): number {
    return this.nextRequestId++;
  }

  private readNativeEvent(timeoutMs: number): ReaderCoreEvent | null {
    const raw = this.native.readEvent(this.runtime, timeoutMs);
    if (raw === null) {
      return null;
    }
    return parseReaderCoreEvent(raw);
  }

  private takePendingForRequest(requestId: number): ReaderCoreEvent | null {
    const index = this.pendingEvents.findIndex((event) => event.requestId === requestId);
    if (index < 0) {
      return null;
    }

    const event = this.pendingEvents[index] as ReaderCoreEvent;
    this.pendingEvents.splice(index, 1);
    return event;
  }

  private ensureOpen(): void {
    if (this.closed) {
      throw new Error("Reader-Core runtime is closed");
    }
  }
}

export class ReaderCoreRequestError extends Error {
  readonly event: ReaderCoreErrorEvent;

  constructor(event: ReaderCoreErrorEvent) {
    super(event.error.message);
    this.name = "ReaderCoreRequestError";
    this.event = event;
  }
}

export function parseReaderCoreEvent(raw: string): ReaderCoreEvent {
  const value = JSON.parse(raw) as unknown;
  if (!isJsonObject(value)) {
    throw new Error("invalid Reader-Core event envelope");
  }

  const requestId = value.requestId;
  if (value.protocolVersion !== 1 || !isNonNegativeSafeInteger(requestId)) {
    throw new Error("invalid Reader-Core event envelope");
  }

  if (value.type === "result") {
    if (!isJsonObject(value.data)) {
      throw new Error("invalid Reader-Core result event");
    }
    return value as ReaderCoreResultEvent;
  }

  if (value.type === "error") {
    if (!isReaderCoreError(value.error)) {
      throw new Error("invalid Reader-Core error event");
    }
    return value as ReaderCoreErrorEvent;
  }

  if (value.type === "host.request") {
    if (
      !isNonNegativeSafeInteger(value.operationId) ||
      typeof value.capability !== "string" ||
      value.capability.length === 0 ||
      !isJsonObject(value.params)
    ) {
      throw new Error("invalid Reader-Core host.request event");
    }
    return value as ReaderCoreHostRequestEvent;
  }

  throw new Error(`unknown Reader-Core event type: ${String(value.type)}`);
}

function assertNonNegativeSafeInteger(value: number, name: string): void {
  if (!isNonNegativeSafeInteger(value)) {
    throw new Error(`${name} must be a non-negative safe integer`);
  }
}

function assertCommandMethod(value: unknown): asserts value is string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error("method must be a non-empty string");
  }
}

function assertJsonObjectValue(value: unknown, name: string): asserts value is JsonObject {
  if (!isJsonObject(value)) {
    throw new Error(`${name} must be a JSON object`);
  }
}

function readTimeoutMs(value: number | undefined): number {
  const timeoutMs = value ?? 2000;
  assertNonNegativeSafeInteger(timeoutMs, "timeoutMs");
  return timeoutMs;
}

function readPollMs(value: number | undefined): number {
  const pollMs = value ?? 10;
  if (!isNonNegativeSafeInteger(pollMs) || pollMs === 0) {
    throw new Error("pollMs must be a positive safe integer");
  }
  return pollMs;
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function normalizeHostError(error: unknown): ReaderCoreError {
  if (isReaderCoreError(error)) {
    return error;
  }

  if (error instanceof Error) {
    return {
      code: "INTERNAL",
      message: error.message,
      retryable: false,
      details: { name: error.name },
    };
  }

  const normalized: ReaderCoreError = {
    code: "INTERNAL",
    message: typeof error === "string" ? error : "host request failed",
    retryable: false,
  };
  if (typeof error === "object" && error !== null) {
    normalized.details = { cause: error };
  }
  return normalized;
}

function isReaderCoreError(value: unknown): value is ReaderCoreError {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<ReaderCoreError>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.message === "string" &&
    typeof candidate.retryable === "boolean" &&
    (candidate.details === undefined || isJsonObject(candidate.details))
  );
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

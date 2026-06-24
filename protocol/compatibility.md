# Protocol Compatibility

## Current Version

- **C ABI Version:** 1
- **JSON Protocol Version:** 1

## Versioning Policy

1. C ABI version is incremented when the function signatures of `reader_core.h` change.
2. Protocol version is incremented when the JSON command/event schema changes in a non-backward-compatible way.
3. Protocol v1 is accepted only when `protocolVersion == 1`; a different
   integer returns `INVALID_PROTOCOL_VERSION`.
4. Both ABI and protocol versions are checked at runtime via `core.info` on startup.

## Compatibility Guarantees (v1)

- Adding new optional fields to request `params` → backward compatible (no version bump).
- Adding new event `type` values → backward compatible (no version bump).
- Adding new `method` values → backward compatible (no version bump).
- Removing or renaming fields → **protocol version bump required**.
- Changing field semantics → **protocol version bump required**.
- Unknown top-level command fields and runtime config fields are rejected as
  `INVALID_MESSAGE`; hosts must not depend on silent field dropping.
- `params` must be a JSON object. Method-specific invalid params return
  `INVALID_PARAMS`.
- `runtime.ping` is the advertised v1 ping method. `core.ping` is accepted as a
  bootstrap alias for current ABI smoke harnesses, but it is not advertised in
  `core.info.capabilities`.
- `runtime.cancel` is the JSON-protocol counterpart of the C ABI
  `rc_runtime_cancel`. It lets a host driving Core purely over the JSON
  protocol (no direct FFI handle) cancel an in-flight request. Params:
  `{ "requestId": <integer> }`. Result data: `{ "cancelled": <bool> }`.
  The cancelled original request receives a separate `CANCELLED` error event
  on its own `requestId`. Unknown / already-completed IDs return
  `{ "cancelled": false }` (idempotent). Self-cancellation (target equals the
  command's own `requestId`) is rejected with `INVALID_PARAMS`.

## Platform Contract

Platforms MUST call `core.info` after `rc_runtime_create` and verify:
- `abiVersion` matches expected value
- `protocolVersion` is compatible (v1 currently requires exact value `1`)
- `capabilities` contains every method/capability the host plans to use

Platforms MUST NOT rely on undocumented fields.
Platforms MUST handle unknown event `type`s gracefully (ignore or log).

## Runtime Config

Runtime config is JSON validated against `reader-runtime-config.schema.json`.
The v1 type contains:

- `dataDirectory`: optional non-empty string for persistent Core data.
- `cacheDirectory`: optional non-empty string for disposable Core cache data.

Unknown config fields, invalid JSON, or wrong value types return structured
`INVALID_MESSAGE` / `INVALID_PARAMS` errors at the runtime boundary that parses
the config.

## Host Bus Semantics

Host capability calls are represented as events and completion commands:

1. Core emits `type: "host.request"` with the original `requestId`, a generated
   `operationId`, `capability`, and object `params`.
2. The host answers with `method: "host.complete"` and params
   `{ "operationId": ..., "result": ... }`, or `method: "host.error"` and params
   `{ "operationId": ..., "error": ... }`.
3. Core routes the completion back to the original `requestId`. Unknown
   operation IDs return `INVALID_PARAMS` on the completion command request.
4. Cancelling the original request cancels its pending host operation and emits
   a `CANCELLED` error for that original request. Cancellation is reachable via
   the C ABI (`rc_runtime_cancel`) or the JSON `runtime.cancel` command; both
   share the same semantics.

### HTTP Transport Capability

Remote-reading commands may emit `capability: "http.execute"` when their
prefetched response body is omitted and a `searchRequest` / `detailRequest` /
`tocRequest` / `chapterRequest` is supplied.

The host request params are:

```json
{
  "url": "https://example.test/path",
  "method": "GET",
  "headers": {},
  "body": null
}
```

The host completes the operation with an object result containing string
`body`. Additional fields such as `status`, `headers`, or final URL are allowed
for host diagnostics, but Core v1 only consumes `body`:

```json
{
  "operationId": 1,
  "result": {
    "status": 200,
    "body": "{\"books\":[]}"
  }
}
```

## Memory & Lifetime Contract (ABI v1)

- Event JSON buffers passed to `rc_event_callback` are **borrowed**: valid only
  for the duration of the callback, `const`, and never owned by the platform.
- There is no `rc_buffer_free`. To retain an event, the platform copies the bytes.
- This avoids cross-language ownership/GC races and keeps the ABI minimal.

---

*This document is the authoritative source for protocol compatibility. All prior handoff, integration, or contract documents are archived in their respective `_archived_planning_2026-06-24/` directories.*

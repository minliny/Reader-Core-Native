# Protocol Compatibility

This document is the compatibility contract for the current Reader-Core-Native
baseline.

## Current Versions

- C ABI version: 1
- JSON protocol version: 1

Evidence:

- `include/reader_core.h`
- `crates/reader-ffi/src/runtime.rs`
- `crates/reader-contract/src/lib.rs`
- `protocol/reader-command.schema.json`
- `protocol/reader-event.schema.json`

## Versioning Policy

1. C ABI version increments when exported function signatures, callback
   ownership, or runtime-handle lifetime rules change.
2. JSON protocol version increments when command/event schema semantics change
   in a non-backward-compatible way.
3. Protocol v1 accepts only `protocolVersion == 1`; any other integer returns
   `INVALID_PROTOCOL_VERSION`.
4. Hosts must call `core.info` after runtime creation and verify ABI version,
   protocol version, and advertised capabilities before sending product
   commands.

## Advertised V1 Capabilities

`core.info.capabilities` currently contains:

- `core.info`
- `runtime.ping`
- `runtime.hostSmoke`
- `host.complete`
- `host.error`
- `host.bus.v1`
- `http.execute`
- `runtime.config.v1`
- `remote.reading.v1`

Evidence command:

```bash
cargo run -p reader-cli -- --info
```

`core.ping` is accepted only as a bootstrap alias in runtime dispatch. It is not
advertised in `core.info.capabilities` and is not listed in
`protocol/reader-command.schema.json` method examples.

## Command Compatibility

Command JSON shape:

- Required top-level fields: `protocolVersion`, `requestId`, `method`.
- `params` defaults to `{}` when omitted.
- `params` must be a JSON object.
- Unknown top-level command fields are rejected because
  `crates/reader-contract/src/command.rs` uses `deny_unknown_fields`.
- Unknown method names return `UNKNOWN_METHOD`.
- Method-specific shape errors return `INVALID_PARAMS`.

Verification:

```bash
cargo run -p reader-cli -- --conformance
cargo test -p reader-contract
```

Backward-compatible additions:

- New optional fields inside method-specific `params`.
- New event `type` values that hosts ignore/log safely.
- New method names and capability names advertised by `core.info`.

Breaking changes requiring a protocol bump:

- Removing or renaming required fields.
- Changing field semantics.
- Changing error code meanings.
- Changing event ownership or correlation rules.

## Event Compatibility

Current event types:

- `result`
- `error`
- `host.request`

`host.request.requestId` is the original Core command request ID blocked on a
host operation. `operationId` identifies the host operation and is used by
`host.complete` or `host.error`.

`CoreError.code` enum:

- `UNKNOWN_METHOD`
- `INVALID_PARAMS`
- `INVALID_PROTOCOL_VERSION`
- `CANCELLED`
- `INVALID_MESSAGE`
- `INTERNAL`

Evidence:

- `crates/reader-contract/src/event.rs`
- `crates/reader-contract/src/error.rs`
- `protocol/reader-event.schema.json`

## Host Bus Semantics

Host capability calls are represented as events and completion commands:

1. Core emits `type: "host.request"` with original `requestId`, generated
   `operationId`, `capability`, and object `params`.
2. Host answers with `method: "host.complete"` and params
   `{ "operationId": ..., "result": ... }`, or `method: "host.error"` and
   params `{ "operationId": ..., "error": ... }`.
3. Core routes completion back to the original request.
4. Unknown or already completed operation IDs return `INVALID_PARAMS` on the
   completion command request.
5. Cancelling a request blocked on host work removes the pending operation and
   emits `CANCELLED` for the original request.

Verification:

```bash
cargo run -p reader-cli -- --host-smoke
cargo run -p reader-cli -- --conformance
```

## HTTP Transport Capability

Remote-reading commands may emit `capability: "http.execute"` when their
prefetched response body is omitted and a request descriptor is supplied.

Host request params:

```json
{
  "url": "https://example.test/path",
  "method": "GET",
  "headers": {},
  "body": null
}
```

Host completion result must contain a string `body`. Additional fields such as
`status`, `headers`, or final URL are allowed for host diagnostics, but Core v1
only consumes `body`.

```json
{
  "operationId": 1,
  "result": {
    "status": 200,
    "body": "{\"books\":[]}"
  }
}
```

Core v1 does not open sockets, perform TLS, own platform cookie jars, or run
WebView login. Those are host/app-owned capabilities.

Verification:

```bash
cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json
```

## Runtime Config

Schema:

- `dataDirectory`: optional non-empty string.
- `cacheDirectory`: optional non-empty string.
- Unknown config fields are rejected by the typed Rust parser.

Evidence:

- `protocol/reader-runtime-config.schema.json`
- `crates/reader-contract/src/config.rs`
- `tools/reader-cli/src/main.rs`

Current limitation:

- Pure Rust runtime creation can parse config through
  `Runtime::new_with_config_json`.
- The C ABI create path currently ignores `_config_json` and `_config_length`
  and calls `Runtime::new(sink)` in `crates/reader-ffi/src/runtime.rs`.
- Therefore persistent directory config is protocol/CLI-validated but not yet
  applied by ABI-hosted runtimes.

## Memory And Lifetime Contract

ABI v1 callback buffers are borrowed:

- Valid only for the duration of `rc_event_callback`.
- `const` and must not be mutated or freed by the host.
- Hosts must copy bytes if events need to outlive the callback.
- No `rc_buffer_free` exists in ABI v1.

Evidence:

- `include/reader_core.h`
- `crates/reader-ffi/src/sink.rs`

## Verification Set

Use these commands before changing the protocol contract:

```bash
cargo fmt --check
cargo test --workspace
cargo run -p reader-cli -- --conformance
cargo run -p reader-cli -- --host-smoke
cargo run -p reader-cli -- --fixture-vertical tests/fixtures/remote_source/basic_source.json
./scripts/ffi-smoke.sh
git diff --check
```

Last updated: 2026-06-24.

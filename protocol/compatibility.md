# Protocol Compatibility

## Current Version

- **C ABI Version:** 1
- **JSON Protocol Version:** 1

## Versioning Policy

1. C ABI version is incremented when the function signatures of `reader_core.h` change.
2. Protocol version is incremented when the JSON command/event schema changes in a non-backward-compatible way.
3. Both are checked at runtime via `core.info` on startup.

## Compatibility Guarantees (v1)

- Adding new optional fields to request `params` → backward compatible (no version bump).
- Adding new event `type` values → backward compatible (no version bump).
- Adding new `method` values → backward compatible (no version bump).
- Removing or renaming fields → **protocol version bump required**.
- Changing field semantics → **protocol version bump required**.

## Platform Contract

Platforms MUST call `core.info` after `rc_runtime_create` and verify:
- `abiVersion` matches expected value
- `protocolVersion` is compatible (same major)

Platforms MUST NOT rely on undocumented fields.
Platforms MUST handle unknown event `type`s gracefully (ignore or log).

## Memory & Lifetime Contract (ABI v1)

- Event JSON buffers passed to `rc_event_callback` are **borrowed**: valid only
  for the duration of the callback, `const`, and never owned by the platform.
- There is no `rc_buffer_free`. To retain an event, the platform copies the bytes.
- This avoids cross-language ownership/GC races and keeps the ABI minimal.

---

*This document is the authoritative source for protocol compatibility. All prior handoff, integration, or contract documents are archived in their respective `_archived_planning_2026-06-24/` directories.*

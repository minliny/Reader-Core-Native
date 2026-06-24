// reader_core.h — Reader-Core C ABI
// Version 1, 2026-06-24
// Single runtime handle + JSON message protocol + callback-based events.

#ifndef READER_CORE_H
#define READER_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// ---------------------------------------------------------------------------
// Opaque runtime handle
// ---------------------------------------------------------------------------

typedef struct rc_runtime rc_runtime_t;

// ---------------------------------------------------------------------------
// Event callback signature
// ---------------------------------------------------------------------------

/// Called by Core with JSON-encoded events.
///
/// Ownership & lifetime contract (frozen, ABI v1):
///   - The buffer is borrowed from Core and is valid ONLY for the duration
///     of this callback invocation. Core may free or reuse it immediately
///     after the callback returns.
///   - `json` is `const`: the platform MUST NOT write through it or free it.
///   - To retain the event beyond the callback, the platform MUST copy the
///     bytes into its own storage.
///   - Core NEVER hands ownership of event buffers to the platform, so there
///     is no corresponding free function. (Synchronous out-buffers returned
///     by a future direct-call API would carry their own ownership rule.)
///
/// The callback is invoked from a Core-owned background thread; the platform
/// sink must be thread-safe and must not block on UI/cross-thread marshalling
/// synchronously — it should enqueue the bytes for delivery instead.
typedef void (*rc_event_callback)(
    void *context,
    const uint8_t *json,
    size_t json_length
);

// ---------------------------------------------------------------------------
// ABI version
// ---------------------------------------------------------------------------

/// Returns the C ABI version for compile/load-time checks.
uint32_t rc_abi_version(void);

// ---------------------------------------------------------------------------
// Runtime lifecycle
// ---------------------------------------------------------------------------

/// Create a new runtime.
/// config_json: platform config (data directory, cache directory, etc.)
/// callback: event sink for all Core-to-platform communication
/// callback_context: opaque pointer passed through to every callback
/// out_runtime: set on success
/// Returns 0 on success, non-zero error code on failure.
int32_t rc_runtime_create(
    const uint8_t *config_json,
    size_t config_length,
    rc_event_callback callback,
    void *callback_context,
    rc_runtime_t **out_runtime
);

/// Send a command (JSON-encoded) to Core.
/// Returns 0 on success, non-zero error code on failure.
int32_t rc_runtime_send(
    rc_runtime_t *runtime,
    const uint8_t *command_json,
    size_t command_length
);

/// Cancel a pending request by its request ID.
/// Returns 0 on success (including when request is not found).
int32_t rc_runtime_cancel(
    rc_runtime_t *runtime,
    uint64_t request_id
);

/// Destroy the runtime, freeing all resources.
/// After this call, no further callbacks will fire.
/// Pending operations are cancelled.
void rc_runtime_destroy(rc_runtime_t *runtime);

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------
//
// There is intentionally no buffer-free function in ABI v1. Event buffers are
// borrowed for the duration of the callback only (see rc_event_callback) and
// are never owned by the platform. See protocol/compatibility.md.

#ifdef __cplusplus
}
#endif

#endif // READER_CORE_H

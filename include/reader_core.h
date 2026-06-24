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
/// Returns 0 on success, non-zero status on failure:
///   2 = `out_runtime` is NULL
///   3 = `callback` is NULL
///   4 = `config_json` is invalid (malformed JSON, unknown field, or invalid
///       value). The structured error is available via `rc_last_error`.
/// `config_json` may be NULL when `config_length` is 0 (defaults applied).
int32_t rc_runtime_create(
    const uint8_t *config_json,
    size_t config_length,
    rc_event_callback callback,
    void *callback_context,
    rc_runtime_t **out_runtime
);

/// Send a command (JSON-encoded) to Core.
/// Returns 0 on success, non-zero status on failure:
///   1 = `runtime` is NULL
///   2 = `command_json` is NULL with non-zero length
///   3 = malformed command JSON / message structure
///   4 = protocol-version mismatch, duplicate active requestId, or runtime
///       shutting down. The structured error is available via `rc_last_error`.
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
// Structured error reporting
// ---------------------------------------------------------------------------
//
// Every entry point returns a coarse `int32_t` status (0 = success, non-zero
// = failure category, documented per function). When a failure originates
// from the JSON protocol layer — config parsing, command parsing, protocol
// version mismatch, duplicate requestId, shutdown — Core also records a
// structured error on the calling thread, retrievable via `rc_last_error`.
//
// This mirrors the `error.code` strings of `reader-event.schema.json` so a C
// host can branch on the same machine-readable codes the async `error` events
// carry, plus read a human-readable message. The slot is errno-style: it is
// set by every failing call and cleared by the next successful call on the
// same thread.

/// Structured error codes. Values are stable for ABI v1 and match the
/// `error.code` enum in `reader-event.schema.json` (SCREAMING_SNAKE_CASE),
/// exposed here as integers so C hosts can switch on them.
typedef enum rc_error_code {
    RC_OK = 0,
    RC_ERR_UNKNOWN_METHOD = 1,
    RC_ERR_INVALID_PARAMS = 2,
    RC_ERR_INVALID_PROTOCOL_VERSION = 3,
    RC_ERR_CANCELLED = 4,
    RC_ERR_INVALID_MESSAGE = 5,
    RC_ERR_INTERNAL = 6,
} rc_error_code_t;

/// Read the structured error recorded by the most recent failed FFI call on
/// the calling thread.
///
/// Returns the error code (`RC_OK` if no error is pending). The slot is NOT
/// cleared by reading; it is cleared by the next successful FFI call.
///
/// If `out_message` is non-NULL and `message_capacity` > 0, writes a
/// NUL-terminated human-readable message into `out_message`, truncated to fit.
/// `out_message` is owned by the caller; Core never aliases it.
///
/// Thread-safety: the slot is per-thread. Only the thread that issued the
/// failing call can read its error.
int32_t rc_last_error(char *out_message, size_t message_capacity);

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

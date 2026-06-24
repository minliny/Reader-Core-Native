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
/// synchronously — it should enqueue the bytes for delivery instead. The
/// callback MUST NOT call `rc_runtime_destroy` reentrantly; schedule teardown on
/// a host-owned thread after the callback returns.
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

// Stable ABI v1 status codes returned by lifecycle entry points.
//
// These enums intentionally remain per function. For example, integer status
// `3` means `RC_CREATE_NULL_CALLBACK` for `rc_runtime_create`, but
// `RC_SEND_INVALID_COMMAND` for `rc_runtime_send`. Hosts should compare against
// the enum for the function they called instead of hard-coded numbers.

typedef enum rc_runtime_create_status {
    RC_CREATE_PANIC = -1,
    RC_CREATE_OK = 0,
    RC_CREATE_NULL_OUT_RUNTIME = 2,
    RC_CREATE_NULL_CALLBACK = 3,
    RC_CREATE_INVALID_CONFIG = 4,
} rc_runtime_create_status_t;

typedef enum rc_runtime_send_status {
    RC_SEND_PANIC = -1,
    RC_SEND_OK = 0,
    RC_SEND_NULL_RUNTIME = 1,
    RC_SEND_NULL_COMMAND = 2,
    RC_SEND_INVALID_COMMAND = 3,
    RC_SEND_PROTOCOL_ERROR = 4,
} rc_runtime_send_status_t;

typedef enum rc_runtime_cancel_status {
    RC_CANCEL_PANIC = -1,
    RC_CANCEL_OK = 0,
    RC_CANCEL_NULL_RUNTIME = 1,
} rc_runtime_cancel_status_t;

/// Create a new runtime.
/// config_json: platform config (data directory, cache directory, etc.)
///   Borrowed only for this call. Core parses/copies what it needs and never
///   retains the pointer after `rc_runtime_create` returns.
/// config_length: byte length of `config_json`; `config_json` may be NULL only
///   when `config_length == 0`.
/// callback: event sink for all Core-to-platform communication
/// callback_context: opaque pointer passed through to every callback
/// out_runtime: set to a runtime handle on success. If `out_runtime` itself is
/// non-NULL, Core writes `NULL` before validating the remaining inputs, so all
/// create failures leave `*out_runtime == NULL`.
/// Returns `RC_CREATE_OK` on success, or one of:
///   `RC_CREATE_NULL_OUT_RUNTIME` = `out_runtime` is NULL
///   `RC_CREATE_NULL_CALLBACK` = `callback` is NULL
///   `RC_CREATE_INVALID_CONFIG` = `config_json` is invalid (malformed JSON,
///       unknown field, or invalid value). The structured error is available
///       via `rc_last_error`.
///   `RC_CREATE_PANIC` = an internal Rust panic was caught by the ABI guard.
/// `config_json` may be NULL when `config_length` is 0 (defaults applied).
int32_t rc_runtime_create(
    const uint8_t *config_json,
    size_t config_length,
    rc_event_callback callback,
    void *callback_context,
    rc_runtime_t **out_runtime
);

/// Send a command (JSON-encoded) to Core.
/// command_json: borrowed only for this call. Core parses/copies what it needs
///   before `rc_runtime_send` returns and never retains the pointer.
/// command_length: byte length of `command_json`; `command_json` may be NULL
///   only when `command_length == 0`.
///
/// Threading: may be called from any host thread while `runtime` is alive.
/// The host must serialize all send/cancel calls with `rc_runtime_destroy`.
///
/// Returns `RC_SEND_OK` on success, or one of:
///   `RC_SEND_NULL_RUNTIME` = `runtime` is NULL
///   `RC_SEND_NULL_COMMAND` = `command_json` is NULL with non-zero length
///   `RC_SEND_INVALID_COMMAND` = malformed command JSON / message structure
///   `RC_SEND_PROTOCOL_ERROR` = protocol-version mismatch, duplicate active
///       requestId, or runtime shutting down. The structured error is
///       available via `rc_last_error`.
///   `RC_SEND_PANIC` = an internal Rust panic was caught by the ABI guard.
int32_t rc_runtime_send(
    rc_runtime_t *runtime,
    const uint8_t *command_json,
    size_t command_length
);

/// Cancel a pending request by its request ID.
///
/// Threading: may be called from any host thread while `runtime` is alive.
/// The host must serialize all send/cancel calls with `rc_runtime_destroy`.
///
/// Returns `RC_CANCEL_OK` on success (including when request is not found),
/// `RC_CANCEL_NULL_RUNTIME` when `runtime` is NULL, or `RC_CANCEL_PANIC` if an
/// internal Rust panic was caught by the ABI guard.
int32_t rc_runtime_cancel(
    rc_runtime_t *runtime,
    uint64_t request_id
);

/// Destroy the runtime, freeing all resources.
/// After this call, no further callbacks will fire.
/// Pending operations are cancelled without requiring host completion.
///
/// Threading: the host must call this at most once for a non-NULL handle and
/// must not call it concurrently with `rc_runtime_send`, `rc_runtime_cancel`,
/// or from inside `rc_event_callback`. `NULL` is a no-op.
/// Successful destroy, including the `NULL` no-op path, clears `rc_last_error`
/// on the calling thread.
void rc_runtime_destroy(rc_runtime_t *runtime);

// ---------------------------------------------------------------------------
// Structured error reporting
// ---------------------------------------------------------------------------
//
// Runtime entry points return a coarse status or no-op completion (0 = success
// for status-returning functions, non-zero = failure category, documented per
// function). When a failure originates from the JSON protocol layer — config
// parsing, command parsing, protocol version mismatch, duplicate requestId,
// shutdown — Core also records a structured error on the calling thread,
// retrievable via `rc_last_error`.
//
// This mirrors the `error.code` strings of `reader-event.schema.json` so a C
// host can branch on the same machine-readable codes the async `error` events
// carry, plus read a human-readable message. The slot is errno-style: it is
// set by every failing runtime call and cleared by the next successful
// runtime create/send/cancel/destroy call on the same thread. `rc_abi_version`
// does not touch the slot.

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
/// cleared by reading; it is cleared by the next successful runtime
/// create/send/cancel/destroy call.
///
/// If `out_message` is non-NULL and `message_capacity` > 0, writes a
/// NUL-terminated human-readable message into `out_message`, truncated to fit.
/// When no error is pending, writes an empty NUL-terminated string so stale
/// host-side text is cleared. `out_message` is owned by the caller; Core never
/// aliases it.
///
/// Thread-safety: the slot is per-thread. Only the thread that issued the
/// failing call can read its error.
///
/// If the error reader itself trips the panic guard, returns
/// `RC_ERR_INTERNAL` and records an internal diagnostic in the same slot.
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

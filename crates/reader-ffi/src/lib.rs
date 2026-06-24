//! Reader-Core C ABI implementation for `include/reader_core.h`.
//!
//! Safety invariants:
//! - Every `extern "C"` entry is wrapped in `panic::catch_unwind` so a Rust
//!   panic can never cross the FFI boundary (UB). On panic we return a
//!   non-zero status code.
//! - The runtime owns its worker thread and a C-backed event sink. The sink
//!   serializes events to JSON and invokes the C `rc_event_callback` from the
//!   worker thread, so the callback MUST be thread-safe (documented in the
//!   header).
//! - On [`rc_runtime_destroy`] the [`reader_runtime::Runtime`] is dropped,
//!   which joins the worker; no callback can fire after `destroy` returns.

mod panic_guard;
mod runtime;
mod sink;

use std::os::raw::c_int;

pub use runtime::{RuntimeHandle, ABI_VERSION};

/// `rc_abi_version` — C ABI version for compile/load-time checks.
#[no_mangle]
pub extern "C" fn rc_abi_version() -> u32 {
    ABI_VERSION
}

/// `rc_runtime_create`. Returns 0 on success, non-zero on failure.
#[no_mangle]
pub unsafe extern "C" fn rc_runtime_create(
    config_json: *const u8,
    config_length: usize,
    callback: sink::CEventCallback,
    callback_context: *mut std::ffi::c_void,
    out_runtime: *mut *mut RuntimeHandle,
) -> c_int {
    panic_guard::guard(|| {
        runtime::create_runtime(
            config_json,
            config_length,
            callback,
            callback_context,
            out_runtime,
        )
    })
}

/// `rc_runtime_send`. Returns 0 on success, non-zero on failure.
#[no_mangle]
pub unsafe extern "C" fn rc_runtime_send(
    runtime: *mut RuntimeHandle,
    command_json: *const u8,
    command_length: usize,
) -> c_int {
    panic_guard::guard(|| runtime::send(runtime, command_json, command_length))
}

/// `rc_runtime_cancel`. Returns 0 on success (including unknown request ID).
#[no_mangle]
pub unsafe extern "C" fn rc_runtime_cancel(runtime: *mut RuntimeHandle, request_id: u64) -> c_int {
    panic_guard::guard(|| runtime::cancel(runtime, request_id))
}

/// `rc_runtime_destroy`. After this returns, no further callbacks fire.
#[no_mangle]
pub unsafe extern "C" fn rc_runtime_destroy(runtime: *mut RuntimeHandle) {
    let _ = panic_guard::guard(|| runtime::destroy(runtime));
}

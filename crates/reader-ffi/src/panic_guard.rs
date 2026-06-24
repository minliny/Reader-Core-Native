use std::panic::{self, AssertUnwindSafe};

use reader_contract::CoreError;

use crate::last_error;

/// Run a closure, catching any panic so it cannot unwind across the FFI
/// boundary (undefined behavior for `extern "C"`).
///
/// Returns the closure's result on success, `-1` on panic.
///
/// We wrap the closure in [`AssertUnwindSafe`] rather than requiring
/// `UnwindSafe`, because FFI entry points take raw pointers and own thread
/// handles — none of which implement `UnwindSafe`. The unwind boundary is
/// *this* function: anything captured is never observed in a half-updated
/// state by Rust again, since a panic means we return to C immediately.
///
/// On panic we record an `INTERNAL` structured error so a C host can read a
/// diagnostic via `rc_last_error`. (Under the release profile `panic =
/// "abort"` a panic aborts the process before this runs; the guard still
/// applies in test/dev builds.)
pub fn guard<F: FnOnce() -> i32>(f: F) -> i32 {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(code) => code,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&'static str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic in FFI entry".to_string());
            last_error::set(CoreError::internal(message));
            -1
        }
    }
}

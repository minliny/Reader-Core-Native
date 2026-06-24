use std::panic::{self, AssertUnwindSafe};

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
pub fn guard<F: FnOnce() -> i32>(f: F) -> i32 {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(code) => code,
        Err(_) => -1,
    }
}

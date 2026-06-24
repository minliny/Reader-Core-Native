use std::{
    any::Any,
    panic::{self, AssertUnwindSafe},
};

use reader_contract::{CoreError, ErrorCode};

use crate::{last_error, status};

/// Run a closure, catching any panic so it cannot unwind across the FFI
/// boundary (undefined behavior for `extern "C"`).
///
/// Returns the closure's result on success, `RC_*_PANIC` (`-1`) on panic.
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
            record_panic(payload);
            status::PANIC
        }
    }
}

/// Guard an entry point whose return value is itself an `rc_error_code_t`.
///
/// `rc_last_error` cannot return a per-entry `RC_*_PANIC` status because its
/// return domain is the structured error-code enum, so a caught panic maps to
/// `RC_ERR_INTERNAL`.
pub fn guard_error_code<F: FnOnce() -> i32>(f: F) -> i32 {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(code) => code,
        Err(payload) => {
            record_panic(payload);
            last_error::code_of(ErrorCode::Internal)
        }
    }
}

fn record_panic(payload: Box<dyn Any + Send>) {
    let message = payload
        .downcast_ref::<&'static str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic in FFI entry".to_string());
    last_error::set(CoreError::internal(message));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_converts_panic_to_status_and_last_error() {
        crate::last_error::clear();

        let code = guard(|| panic!("ffi panic guard smoke"));

        assert_eq!(code, crate::status::PANIC);
        let mut buf = [0u8; 128];
        let err = unsafe { crate::last_error::read(buf.as_mut_ptr(), buf.len()) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let msg = std::str::from_utf8(&buf[..end]).unwrap();
        assert_eq!(
            err,
            crate::last_error::code_of(reader_contract::ErrorCode::Internal)
        );
        assert!(msg.contains("ffi panic guard smoke"));
    }

    #[test]
    fn guard_error_code_converts_panic_to_internal_error_code() {
        crate::last_error::clear();

        let code = guard_error_code(|| panic!("ffi last_error guard smoke"));

        assert_eq!(
            code,
            crate::last_error::code_of(reader_contract::ErrorCode::Internal)
        );
        let mut buf = [0u8; 128];
        let err = unsafe { crate::last_error::read(buf.as_mut_ptr(), buf.len()) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let msg = std::str::from_utf8(&buf[..end]).unwrap();
        assert_eq!(err, code);
        assert!(msg.contains("ffi last_error guard smoke"));
    }
}

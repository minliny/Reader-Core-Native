//! Thread-local, errno-style structured error reporting for the C ABI.
//!
//! The FFI entry points return a coarse `int32_t` status (0 = success, a small
//! fixed set of non-zero codes for argument/protocol categories). When a
//! failure originates from the JSON protocol layer — runtime config parsing,
//! command parsing, protocol-version mismatch, duplicate `requestId`, or
//! shutdown — the same structured [`CoreError`] the pure-Rust runtime produces
//! is recorded here so a C host can read the machine-readable code and a
//! human-readable message via [`rc_last_error`](crate::last_error::read).
//!
//! Semantics (frozen for ABI v1):
//! - The slot is per-thread. Only the thread that issued the failing call can
//!   read its error — matches the synchronous nature of the entry points.
//! - Runtime lifecycle/command entry points (`create`, `send`, `cancel`,
//!   `destroy`) call [`set`] on failure and [`clear`] on success. Reading does
//!   not consume the slot (peek, not take), so a host may inspect it more than
//!   once; the next successful runtime call clears it.
//! - Argument-level failures (null pointers) are also recorded, mapped to the
//!   closest protocol code, so the host always has a message to log.
//!
//! `rc_abi_version` is a pure version query and does not touch the slot.
//! `rc_last_error` is a peek API and does not clear the slot. Async
//! command-processing errors are NOT recorded here: they are delivered as
//! `error` events through `rc_event_callback`, exactly as in the pure-Rust
//! runtime. This slot covers only synchronous call-site failures.

use std::cell::RefCell;

use reader_contract::{CoreError, ErrorCode};

thread_local! {
    static LAST_ERROR: RefCell<Option<CoreError>> = const { RefCell::new(None) };
}

/// Record a structured error on the calling thread.
pub fn set(err: CoreError) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(err));
}

/// Clear any recorded error on the calling thread. Called on every successful
/// FFI entry so a stale error can never be observed after a success.
pub fn clear() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Map a protocol [`ErrorCode`] to the stable C `rc_error_code` integer
/// declared in `reader_core.h`. Authoritative for ABI v1.
pub fn code_of(code: ErrorCode) -> i32 {
    match code {
        ErrorCode::UnknownMethod => 1,
        ErrorCode::InvalidParams => 2,
        ErrorCode::InvalidProtocolVersion => 3,
        ErrorCode::Cancelled => 4,
        ErrorCode::InvalidMessage => 5,
        ErrorCode::Internal => 6,
    }
}

/// Peek the calling thread's last error without consuming it.
///
/// Returns `RC_OK` (0) when no error is pending. In that case, when
/// `out_message` is a non-null buffer of positive `capacity`, writes an empty
/// NUL-terminated string so host-side stale text is cleared. Otherwise returns
/// the structured code and writes a NUL-terminated copy of the message
/// (truncated to fit).
///
/// # Safety
/// `out_message` must point to at least `capacity` writable bytes when non-null.
pub unsafe fn read(out_message: *mut u8, capacity: usize) -> i32 {
    LAST_ERROR.with(|slot| {
        let guard = slot.borrow();
        let Some(err) = guard.as_ref() else {
            if !out_message.is_null() && capacity > 0 {
                // SAFETY: caller guarantees `out_message` points to
                // `capacity` writable bytes; writing one NUL byte is within
                // bounds because capacity is positive.
                unsafe {
                    *out_message = 0;
                }
            }
            return 0;
        };
        let code = code_of(err.code);
        let bytes = err.message.as_bytes();
        if !out_message.is_null() && capacity > 0 {
            let copy = bytes.len().min(capacity.saturating_sub(1));
            // SAFETY: caller guarantees `out_message` points to `capacity`
            // writable bytes; we write at most `copy + 1` (copy + NUL) and
            // `copy < capacity`.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_message, copy);
                *out_message.add(copy) = 0;
            }
        }
        code
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_returns_zero_when_empty() {
        clear();
        let code = unsafe { read(std::ptr::null_mut(), 0) };
        assert_eq!(code, 0);
    }

    #[test]
    fn read_clears_message_buffer_when_empty() {
        clear();
        let mut buf = *b"stale";
        let code = unsafe { read(buf.as_mut_ptr(), buf.len()) };
        assert_eq!(code, 0);
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn set_then_read_round_trips_code_and_message() {
        clear();
        set(CoreError::invalid_protocol_version(2));
        let mut buf = [0u8; 64];
        let code = unsafe { read(buf.as_mut_ptr(), buf.len()) };
        assert_eq!(code, code_of(ErrorCode::InvalidProtocolVersion));
        let msg = std::str::from_utf8(&buf).unwrap().trim_end_matches('\0');
        assert!(msg.contains("unsupported protocolVersion"));
    }

    #[test]
    fn read_does_not_consume_the_slot() {
        clear();
        set(CoreError::cancelled());
        let code1 = unsafe { read(std::ptr::null_mut(), 0) };
        let code2 = unsafe { read(std::ptr::null_mut(), 64) };
        let mut buf = [0u8; 16];
        let code3 = unsafe { read(buf.as_mut_ptr(), buf.len()) };
        assert_eq!(code1, code_of(ErrorCode::Cancelled));
        assert_eq!(code2, code_of(ErrorCode::Cancelled));
        assert_eq!(code3, code_of(ErrorCode::Cancelled));
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        assert!(end > 0);
    }

    #[test]
    fn read_with_zero_capacity_writes_nothing() {
        clear();
        set(CoreError::internal("boom"));
        let mut buf = *b"stale";
        let code = unsafe { read(buf.as_mut_ptr(), 0) };
        assert_eq!(code, code_of(ErrorCode::Internal));
        assert_eq!(&buf, b"stale");
    }

    #[test]
    fn clear_removes_the_slot() {
        set(CoreError::internal("boom"));
        clear();
        assert_eq!(unsafe { read(std::ptr::null_mut(), 0) }, 0);
    }

    #[test]
    fn message_is_truncated_and_nul_terminated() {
        clear();
        set(CoreError::internal(
            "a very long message that does not fit in a tiny buffer",
        ));
        let mut buf = [0u8; 8];
        let code = unsafe { read(buf.as_mut_ptr(), buf.len()) };
        assert_eq!(code, code_of(ErrorCode::Internal));
        // NUL terminator is within the buffer at index 7.
        assert_eq!(buf[7], 0);
        let msg = std::str::from_utf8(&buf[..7]).unwrap();
        assert_eq!(msg.len(), 7);
    }
}

use std::ffi::c_void;
use std::sync::Arc;

use reader_contract::Command;
use reader_runtime::Runtime;

use crate::sink::{CEventCallback, CEventSink};

/// C ABI version this build advertises. Authoritative source of truth.
pub const ABI_VERSION: u32 = 1;

/// Opaque runtime handle returned to the platform. Boxed so the pointer stays
/// stable and `destroy` can reclaim it.
pub struct RuntimeHandle {
    runtime: Runtime,
}

/// Validate a borrowed byte slice from the platform. Returns `None` if the
/// pointer is null (length must then be 0) or the length is nonsensical.
unsafe fn borrow_bytes<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    if ptr.is_null() {
        return if len == 0 { Some(&[]) } else { None };
    }
    Some(std::slice::from_raw_parts(ptr, len))
}

pub unsafe fn create_runtime(
    _config_json: *const u8,
    _config_length: usize,
    callback: CEventCallback,
    context: *mut c_void,
    out_runtime: *mut *mut RuntimeHandle,
) -> i32 {
    if out_runtime.is_null() {
        return 2; // invalid out-pointer
    }
    if callback.is_none() {
        return 3; // a callback is required
    }
    let sink = Arc::new(CEventSink::new(callback, context));
    let runtime = Runtime::new(sink);
    let handle = Box::new(RuntimeHandle { runtime });
    *out_runtime = Box::into_raw(handle);
    0
}

pub unsafe fn send(
    runtime: *mut RuntimeHandle,
    command_json: *const u8,
    command_length: usize,
) -> i32 {
    let Some(handle) = runtime.as_ref() else {
        return 1;
    };
    let Some(bytes) = borrow_bytes(command_json, command_length) else {
        return 2;
    };
    let command: Command = match serde_json::from_slice(bytes) {
        Ok(c) => c,
        Err(_) => return 3, // malformed JSON / command
    };
    match handle.runtime.send(command) {
        Ok(()) => 0,
        Err(_) => 4, // protocol mismatch or shutting down
    }
}

pub unsafe fn cancel(runtime: *mut RuntimeHandle, request_id: u64) -> i32 {
    let Some(handle) = runtime.as_ref() else {
        return 1;
    };
    handle.runtime.cancel(request_id);
    0
}

pub unsafe fn destroy(runtime: *mut RuntimeHandle) -> i32 {
    if runtime.is_null() {
        return 0;
    }
    // Reclaim the box; dropping it joins the worker, so no callback fires after.
    let _ = Box::from_raw(runtime);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;
    use std::sync::{Arc, Mutex};

    struct CapturedEvents(Mutex<Vec<Vec<u8>>>);

    extern "C" fn capture(ctx: *mut c_void, json: *const u8, len: usize) {
        let events = unsafe { &*(ctx as *const CapturedEvents) };
        let slice = unsafe { std::slice::from_raw_parts(json, len) };
        events.0.lock().unwrap().push(slice.to_vec());
    }

    #[test]
    fn full_lifecycle_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;

        let mut handle: *mut RuntimeHandle = ptr::null_mut();
        let config = b"{}";
        let code = unsafe {
            create_runtime(
                config.as_ptr(),
                config.len(),
                Some(capture),
                ctx_ptr,
                &mut handle,
            )
        };
        assert_eq!(code, 0);
        assert!(!handle.is_null());

        // core.ping
        let cmd = br#"{"protocolVersion":1,"requestId":42,"method":"core.ping","params":{}}"#;
        let code = unsafe { send(handle, cmd.as_ptr(), cmd.len()) };
        assert_eq!(code, 0);

        std::thread::sleep(std::time::Duration::from_millis(50));

        let code = unsafe { destroy(handle) };
        assert_eq!(code, 0);

        let evs = events.0.lock().unwrap();
        assert_eq!(evs.len(), 1);
        let v: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(v["type"], "result");
        assert_eq!(v["requestId"], 42);
        assert_eq!(v["data"]["pong"], true);
    }

    #[test]
    fn create_destroy_cycle_1000_times() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;
        for _ in 0..1000 {
            let mut handle: *mut RuntimeHandle = ptr::null_mut();
            unsafe {
                create_runtime(b"{}".as_ptr(), 2, Some(capture), ctx_ptr, &mut handle);
                destroy(handle);
            }
        }
    }

    #[test]
    fn create_rejects_invalid_out_pointer_or_missing_callback() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;

        let code =
            unsafe { create_runtime(b"{}".as_ptr(), 2, Some(capture), ctx_ptr, ptr::null_mut()) };
        assert_eq!(code, 2);

        let mut handle: *mut RuntimeHandle = ptr::null_mut();
        let code = unsafe { create_runtime(b"{}".as_ptr(), 2, None, ctx_ptr, &mut handle) };
        assert_eq!(code, 3);
        assert!(handle.is_null());
    }

    #[test]
    fn send_rejects_null_runtime_null_payload_and_malformed_json() {
        let mut handle: *mut RuntimeHandle = ptr::null_mut();
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;
        let code =
            unsafe { create_runtime(b"{}".as_ptr(), 2, Some(capture), ctx_ptr, &mut handle) };
        assert_eq!(code, 0);
        assert!(!handle.is_null());

        let code = unsafe { send(ptr::null_mut(), b"{}".as_ptr(), 2) };
        assert_eq!(code, 1);

        let code = unsafe { send(handle, ptr::null(), 1) };
        assert_eq!(code, 2);

        let malformed = b"{";
        let code = unsafe { send(handle, malformed.as_ptr(), malformed.len()) };
        assert_eq!(code, 3);

        let protocol_error =
            br#"{"protocolVersion":2,"requestId":9,"method":"runtime.ping","params":{}}"#;
        let code = unsafe { send(handle, protocol_error.as_ptr(), protocol_error.len()) };
        assert_eq!(code, 4);

        let code = unsafe { destroy(handle) };
        assert_eq!(code, 0);
    }

    #[test]
    fn cancel_and_destroy_accept_null_runtime_contracts() {
        assert_eq!(unsafe { cancel(ptr::null_mut(), 42) }, 1);
        assert_eq!(unsafe { destroy(ptr::null_mut()) }, 0);
    }
}

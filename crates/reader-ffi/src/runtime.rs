use std::ffi::c_void;
use std::sync::Arc;

use reader_contract::{Command, CoreError};
use reader_runtime::Runtime;

use crate::sink::{CEventCallback, CEventSink};
use crate::{last_error, status};

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
    config_json: *const u8,
    config_length: usize,
    callback: CEventCallback,
    context: *mut c_void,
    out_runtime: *mut *mut RuntimeHandle,
) -> i32 {
    if out_runtime.is_null() {
        last_error::set(CoreError::invalid_message("out_runtime pointer is null"));
        return status::create::NULL_OUT_RUNTIME;
    }
    *out_runtime = std::ptr::null_mut();
    if callback.is_none() {
        last_error::set(CoreError::invalid_message("event callback is null"));
        return status::create::NULL_CALLBACK;
    }
    let Some(bytes) = borrow_bytes(config_json, config_length) else {
        last_error::set(CoreError::invalid_message(
            "config_json is null with non-zero length",
        ));
        return status::create::INVALID_CONFIG;
    };
    let sink = Arc::new(CEventSink::new(callback, context));
    let runtime = match Runtime::new_with_config_json(sink, bytes) {
        Ok(runtime) => runtime,
        Err(err) => {
            last_error::set(err);
            return status::create::INVALID_CONFIG;
        }
    };
    let handle = Box::new(RuntimeHandle { runtime });
    *out_runtime = Box::into_raw(handle);
    last_error::clear();
    status::create::OK
}

pub unsafe fn send(
    runtime: *mut RuntimeHandle,
    command_json: *const u8,
    command_length: usize,
) -> i32 {
    let Some(handle) = runtime.as_ref() else {
        last_error::set(CoreError::invalid_message("runtime handle is null"));
        return status::send::NULL_RUNTIME;
    };
    let Some(bytes) = borrow_bytes(command_json, command_length) else {
        last_error::set(CoreError::invalid_message(
            "command_json is null with non-zero length",
        ));
        return status::send::NULL_COMMAND;
    };
    // Parse JSON + structure only. Protocol-version / duplicate-requestId /
    // shutdown checks happen in `Runtime::send` so they map to status 4, not
    // the parse-failure status 3 — keeping the documented status contract.
    let command: Command = match serde_json::from_slice(bytes) {
        Ok(c) => c,
        Err(err) => {
            last_error::set(
                CoreError::invalid_message("invalid command JSON")
                    .with_details(serde_json::json!({ "source": err.to_string() })),
            );
            return status::send::INVALID_COMMAND;
        }
    };
    match handle.runtime.send(command) {
        Ok(()) => {
            last_error::clear();
            status::send::OK
        }
        Err(err) => {
            last_error::set(err);
            status::send::PROTOCOL_ERROR
        }
    }
}

pub unsafe fn cancel(runtime: *mut RuntimeHandle, request_id: u64) -> i32 {
    let Some(handle) = runtime.as_ref() else {
        last_error::set(CoreError::invalid_message("runtime handle is null"));
        return status::cancel::NULL_RUNTIME;
    };
    handle.runtime.cancel(request_id);
    last_error::clear();
    status::cancel::OK
}

pub unsafe fn destroy(runtime: *mut RuntimeHandle) -> i32 {
    if runtime.is_null() {
        last_error::clear();
        return status::OK;
    }
    // Reclaim the box; dropping it joins the worker, so no callback fires after.
    let _ = Box::from_raw(runtime);
    last_error::clear();
    status::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_contract::ErrorCode;
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

        let mut stale = ptr::dangling_mut::<RuntimeHandle>();
        let code = unsafe { create_runtime(b"{}".as_ptr(), 2, None, ctx_ptr, &mut stale) };
        assert_eq!(code, 3);
        assert!(stale.is_null());

        let bad = br#"{not json"#;
        stale = ptr::dangling_mut::<RuntimeHandle>();
        let code =
            unsafe { create_runtime(bad.as_ptr(), bad.len(), Some(capture), ctx_ptr, &mut stale) };
        assert_eq!(code, 4);
        assert!(stale.is_null());

        stale = ptr::dangling_mut::<RuntimeHandle>();
        let code = unsafe { create_runtime(ptr::null(), 1, Some(capture), ctx_ptr, &mut stale) };
        assert_eq!(code, 4);
        assert!(stale.is_null());
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

        let code = unsafe { send(handle, ptr::null(), 0) };
        assert_eq!(code, 3);

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
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );
        assert_eq!(unsafe { destroy(ptr::null_mut()) }, 0);
        assert_eq!(last_error_code(), 0);
    }

    #[test]
    fn successful_destroy_clears_last_error() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        crate::last_error::set(CoreError::internal("stale"));
        assert_eq!(unsafe { destroy(handle) }, 0);
        assert_eq!(last_error_code(), 0);
    }

    // --- last_error integration -------------------------------------------

    /// Read the structured code + message from the thread-local last-error slot.
    fn last_error_code() -> i32 {
        let mut buf = [0u8; 128];
        unsafe { crate::last_error::read(buf.as_mut_ptr(), buf.len()) }
    }

    fn last_error_message() -> String {
        let mut buf = [0u8; 128];
        unsafe { crate::last_error::read(buf.as_mut_ptr(), buf.len()) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).to_string()
    }

    fn make_runtime(events: &Arc<CapturedEvents>) -> *mut RuntimeHandle {
        let ctx_ptr = Arc::as_ptr(events) as *mut c_void;
        let mut handle: *mut RuntimeHandle = ptr::null_mut();
        let code =
            unsafe { create_runtime(b"{}".as_ptr(), 2, Some(capture), ctx_ptr, &mut handle) };
        assert_eq!(code, 0);
        assert!(!handle.is_null());
        handle
    }

    #[test]
    fn last_error_reports_structured_code_for_send_failures() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        // A successful send clears the slot.
        crate::last_error::set(CoreError::internal("stale"));
        let ok = br#"{"protocolVersion":1,"requestId":1,"method":"runtime.ping","params":{}}"#;
        assert_eq!(unsafe { send(handle, ok.as_ptr(), ok.len()) }, 0);
        assert_eq!(last_error_code(), 0);

        // Null runtime → status 1, structured INVALID_MESSAGE.
        assert_eq!(unsafe { send(ptr::null_mut(), ok.as_ptr(), ok.len()) }, 1);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );

        // Malformed JSON → status 3, INVALID_MESSAGE.
        assert_eq!(unsafe { send(handle, b"{".as_ptr(), 1) }, 3);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );

        // Protocol-version mismatch → status 4, INVALID_PROTOCOL_VERSION.
        let proto = br#"{"protocolVersion":2,"requestId":2,"method":"runtime.ping","params":{}}"#;
        assert_eq!(unsafe { send(handle, proto.as_ptr(), proto.len()) }, 4);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidProtocolVersion)
        );
        assert!(last_error_message().contains("protocolVersion"));

        // Duplicate active requestId → status 4, INVALID_MESSAGE with details.
        let dup_a =
            br#"{"protocolVersion":1,"requestId":7,"method":"runtime.hostSmoke","params":{}}"#;
        let dup_b = br#"{"protocolVersion":1,"requestId":7,"method":"runtime.ping","params":{}}"#;
        assert_eq!(unsafe { send(handle, dup_a.as_ptr(), dup_a.len()) }, 0);
        assert_eq!(unsafe { send(handle, dup_b.as_ptr(), dup_b.len()) }, 4);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn create_rejects_invalid_config_and_records_last_error() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;
        let mut handle: *mut RuntimeHandle = ptr::null_mut();

        // Malformed config JSON → status 4, INVALID_MESSAGE.
        let bad = br#"{not json"#;
        let code =
            unsafe { create_runtime(bad.as_ptr(), bad.len(), Some(capture), ctx_ptr, &mut handle) };
        assert_eq!(code, 4);
        assert!(handle.is_null());
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );

        // Unknown config field → INVALID_MESSAGE.
        let bad = br#"{"bogus":true}"#;
        let code =
            unsafe { create_runtime(bad.as_ptr(), bad.len(), Some(capture), ctx_ptr, &mut handle) };
        assert_eq!(code, 4);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );

        // Empty data directory → INVALID_PARAMS.
        let bad = br#"{"dataDirectory":"  "}"#;
        let code =
            unsafe { create_runtime(bad.as_ptr(), bad.len(), Some(capture), ctx_ptr, &mut handle) };
        assert_eq!(code, 4);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidParams)
        );
    }

    #[test]
    fn config_is_parsed_and_applied_to_runtime() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;
        let mut handle: *mut RuntimeHandle = ptr::null_mut();
        let config =
            br#"{"dataDirectory":"/tmp/reader-ffi-data","cacheDirectory":"/tmp/reader-ffi-cache"}"#;
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
        let config = unsafe { (&*handle).runtime.config() };
        assert_eq!(
            config.data_directory.as_deref(),
            Some("/tmp/reader-ffi-data")
        );
        assert_eq!(
            config.cache_directory.as_deref(),
            Some("/tmp/reader-ffi-cache")
        );
        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    // --- host bus round trip via the C ABI --------------------------------

    /// Poll the captured events until `n` have arrived, or time out.
    fn wait_events(events: &Arc<CapturedEvents>, n: usize) -> Vec<Vec<u8>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let snapshot = events.0.lock().unwrap().clone();
            if snapshot.len() >= n {
                return snapshot;
            }
            if std::time::Instant::now() >= deadline {
                panic!("timed out waiting for {n} events; got {}", snapshot.len());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn host_request_then_host_complete_round_trips_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        // runtime.hostSmoke → Core emits host.request.
        let smoke = br#"{"protocolVersion":1,"requestId":100,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"hello":"world"}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);

        let evs = wait_events(&events, 1);
        let req: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["requestId"], 100);
        assert_eq!(req["capability"], "host.smoke.echo");
        let operation_id = req["operationId"].as_u64().unwrap();

        // host.complete → Core routes the result back to requestId 100.
        let complete = serde_json::json!({
            "protocolVersion": 1,
            "requestId": 101,
            "method": "host.complete",
            "params": {
                "operationId": operation_id,
                "result": { "echoed": true }
            }
        })
        .to_string();
        assert_eq!(
            unsafe { send(handle, complete.as_ptr(), complete.len()) },
            0
        );

        let evs = wait_events(&events, 2);
        let res: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(res["type"], "result");
        assert_eq!(res["requestId"], 100);
        assert_eq!(res["data"]["echoed"], true);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn host_error_routes_error_to_original_request_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let smoke = br#"{"protocolVersion":1,"requestId":200,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);
        let req: serde_json::Value = serde_json::from_slice(&wait_events(&events, 1)[0]).unwrap();
        let operation_id = req["operationId"].as_u64().unwrap();

        let error = serde_json::json!({
            "protocolVersion": 1,
            "requestId": 201,
            "method": "host.error",
            "params": {
                "operationId": operation_id,
                "error": {
                    "code": "INTERNAL",
                    "message": "host failed",
                    "retryable": true
                }
            }
        })
        .to_string();
        assert_eq!(unsafe { send(handle, error.as_ptr(), error.len()) }, 0);

        let evs = wait_events(&events, 2);
        let err: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["requestId"], 200);
        assert_eq!(err["error"]["code"], "INTERNAL");
        assert_eq!(err["error"]["retryable"], true);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn unknown_method_emits_error_event_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let cmd = br#"{"protocolVersion":1,"requestId":300,"method":"no.such.method","params":{}}"#;
        assert_eq!(unsafe { send(handle, cmd.as_ptr(), cmd.len()) }, 0);

        let evs = wait_events(&events, 1);
        let err: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["requestId"], 300);
        assert_eq!(err["error"]["code"], "UNKNOWN_METHOD");

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn cancelling_pending_host_request_emits_cancelled_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let smoke = br#"{"protocolVersion":1,"requestId":400,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);
        wait_events(&events, 1);

        assert_eq!(unsafe { cancel(handle, 400) }, 0);

        let evs = wait_events(&events, 2);
        let err: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["requestId"], 400);
        assert_eq!(err["error"]["code"], "CANCELLED");

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn destroy_with_pending_host_request_stops_callbacks_after_return() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let smoke = br#"{"protocolVersion":1,"requestId":450,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);
        wait_events(&events, 1);

        assert_eq!(unsafe { destroy(handle) }, 0);
        std::thread::sleep(std::time::Duration::from_millis(25));

        let evs = events.0.lock().unwrap();
        assert_eq!(evs.len(), 1);
        let req: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["requestId"], 450);
    }

    #[test]
    fn host_complete_for_unknown_operation_emits_invalid_params_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let complete =
            br#"{"protocolVersion":1,"requestId":500,"method":"host.complete","params":{"operationId":404,"result":{"ok":true}}}"#;
        assert_eq!(
            unsafe { send(handle, complete.as_ptr(), complete.len()) },
            0
        );

        let evs = wait_events(&events, 1);
        let err: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["requestId"], 500);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");

        assert_eq!(unsafe { destroy(handle) }, 0);
    }
}

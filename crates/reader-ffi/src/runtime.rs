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
    // Parse JSON + top-level message structure only. Protocol-version /
    // duplicate-requestId / shutdown checks happen in `Runtime::send` so they
    // map to status 4, not the parse/shape-failure status 3 — keeping the
    // documented status contract.
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
    if !command.params.is_object() {
        last_error::set(
            CoreError::invalid_params("command params must be a JSON object")
                .with_details(serde_json::json!({ "method": command.method.clone() })),
        );
        return status::send::INVALID_COMMAND;
    }
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
        assert_eq!(v["protocolVersion"], 1);
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
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );
        assert!(last_error_message().contains("out_runtime"));

        let mut handle: *mut RuntimeHandle = ptr::null_mut();
        let code = unsafe { create_runtime(b"{}".as_ptr(), 2, None, ctx_ptr, &mut handle) };
        assert_eq!(code, 3);
        assert!(handle.is_null());
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );
        assert!(last_error_message().contains("event callback"));

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

        let empty = b"";
        let code = unsafe { send(handle, empty.as_ptr(), 0) };
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
        assert!(last_error_message().contains("runtime handle"));
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
    fn cancel_missing_request_is_noop_success_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        crate::last_error::set(CoreError::internal("stale"));
        assert_eq!(unsafe { cancel(handle, 404) }, 0);
        assert_eq!(last_error_code(), 0);
        std::thread::sleep(std::time::Duration::from_millis(25));
        assert!(events.0.lock().unwrap().is_empty());

        assert_eq!(unsafe { destroy(handle) }, 0);
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

        // Empty JSON payload → status 3, INVALID_MESSAGE.
        let empty = b"";
        assert_eq!(unsafe { send(handle, empty.as_ptr(), 0) }, 3);
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidMessage)
        );

        // Non-object params → status 3, structured INVALID_PARAMS.
        let invalid_params =
            br#"{"protocolVersion":1,"requestId":2,"method":"runtime.ping","params":[]}"#;
        assert_eq!(
            unsafe { send(handle, invalid_params.as_ptr(), invalid_params.len()) },
            3
        );
        assert_eq!(
            last_error_code(),
            last_error::code_of(ErrorCode::InvalidParams)
        );
        assert!(last_error_message().contains("params"));

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

    #[test]
    fn null_config_pointer_with_zero_length_uses_defaults() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let ctx_ptr = Arc::as_ptr(&events) as *mut c_void;
        let mut handle: *mut RuntimeHandle = ptr::null_mut();

        crate::last_error::set(CoreError::internal("stale"));
        let code = unsafe { create_runtime(ptr::null(), 0, Some(capture), ctx_ptr, &mut handle) };
        assert_eq!(code, 0);
        assert!(!handle.is_null());
        assert_eq!(last_error_code(), 0);

        let config = unsafe { (&*handle).runtime.config() };
        assert!(config.data_directory.is_none());
        assert!(config.cache_directory.is_none());

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
        assert_eq!(req["protocolVersion"], 1);
        assert_eq!(req["requestId"], 100);
        assert_eq!(req["capability"], "host.smoke.echo");
        assert_eq!(req["params"]["hello"], "world");
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
        assert_eq!(res["protocolVersion"], 1);
        assert_eq!(res["requestId"], 100);
        assert_eq!(res["data"]["echoed"], true);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn runtime_status_reports_pending_host_operation_metadata_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let smoke = br#"{"protocolVersion":1,"requestId":140,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"hidden":"status payload"}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);

        let evs = wait_events(&events, 1);
        let req: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["protocolVersion"], 1);
        assert_eq!(req["requestId"], 140);
        assert_eq!(req["capability"], "host.smoke.echo");
        assert_eq!(req["params"]["hidden"], "status payload");
        let operation_id = req["operationId"].as_u64().unwrap();

        let status =
            br#"{"protocolVersion":1,"requestId":141,"method":"runtime.status","params":{}}"#;
        assert_eq!(unsafe { send(handle, status.as_ptr(), status.len()) }, 0);
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 2);
        let result: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(result["type"], "result");
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(result["requestId"], 141);
        assert_eq!(result["data"]["activeRequestCount"], 1);
        assert_eq!(result["data"]["activeRequestIds"], serde_json::json!([140]));
        assert_eq!(result["data"]["pendingHostOperationCount"], 1);
        assert_eq!(result["data"]["shuttingDown"], false);
        let operations = result["data"]["pendingHostOperations"]
            .as_array()
            .expect("pendingHostOperations");
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0]["operationId"], operation_id);
        assert_eq!(operations[0]["requestId"], 140);
        assert_eq!(operations[0]["capability"], "host.smoke.echo");
        assert_eq!(operations[0]["state"], "pending");
        assert!(operations[0].get("params").is_none());

        let complete = serde_json::json!({
            "protocolVersion": 1,
            "requestId": 142,
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

        let evs = wait_events(&events, 3);
        let completed: serde_json::Value = serde_json::from_slice(&evs[2]).unwrap();
        assert_eq!(completed["type"], "result");
        assert_eq!(completed["protocolVersion"], 1);
        assert_eq!(completed["requestId"], 140);
        assert_eq!(completed["data"]["echoed"], true);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn runtime_cancel_command_cancels_pending_host_request_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let host_request =
            include_bytes!("../../../protocol/fixtures/conformance/host/request.json");
        assert_eq!(
            unsafe { send(handle, host_request.as_ptr(), host_request.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 1);
        let req: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["protocolVersion"], 1);
        assert_eq!(req["requestId"], 301);
        assert_eq!(req["capability"], "host.smoke.echo");
        assert_eq!(req["params"]["message"], "conformance host request");
        assert!(req["operationId"].as_u64().is_some());

        let cancel = include_bytes!(
            "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
        );
        assert_eq!(unsafe { send(handle, cancel.as_ptr(), cancel.len()) }, 0);
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 3);
        let cancelled: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(cancelled["type"], "error");
        assert_eq!(cancelled["protocolVersion"], 1);
        assert_eq!(cancelled["requestId"], 301);
        assert_eq!(cancelled["error"]["code"], "CANCELLED");

        let result: serde_json::Value = serde_json::from_slice(&evs[2]).unwrap();
        assert_eq!(result["type"], "result");
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(result["requestId"], 310);
        assert_eq!(result["data"]["cancelled"], true);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn runtime_cancel_command_reports_false_and_invalid_params_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let unknown_target =
            br#"{"protocolVersion":1,"requestId":313,"method":"runtime.cancel","params":{"requestId":999999}}"#;
        assert_eq!(
            unsafe { send(handle, unknown_target.as_ptr(), unknown_target.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 1);
        let result: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(result["type"], "result");
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(result["requestId"], 313);
        assert_eq!(result["data"]["cancelled"], false);

        let zero_target = include_bytes!(
            "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-target-zero.json"
        );
        assert_eq!(
            unsafe { send(handle, zero_target.as_ptr(), zero_target.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 2);
        let err: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 311);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(err["error"]["details"]["requestId"], 0);

        let unknown_field = include_bytes!(
            "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-unknown-field.json"
        );
        assert_eq!(
            unsafe { send(handle, unknown_field.as_ptr(), unknown_field.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 3);
        let err: serde_json::Value = serde_json::from_slice(&evs[2]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 312);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(
            err["error"]["details"]["method"],
            reader_contract::methods::RUNTIME_CANCEL
        );
        assert!(
            err["error"]["details"]["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected error event: {err}"
        );

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn host_error_routes_error_to_original_request_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let smoke = br#"{"protocolVersion":1,"requestId":200,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);
        let req: serde_json::Value = serde_json::from_slice(&wait_events(&events, 1)[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["protocolVersion"], 1);
        assert_eq!(req["requestId"], 200);
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
        assert_eq!(err["protocolVersion"], 1);
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
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 300);
        assert_eq!(err["error"]["code"], "UNKNOWN_METHOD");

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn method_specific_invalid_params_emit_error_event_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let cmd = include_bytes!(
            "../../../protocol/fixtures/conformance/commands/invalid-reading-progress-update-unknown-field.json"
        );
        assert_eq!(unsafe { send(handle, cmd.as_ptr(), cmd.len()) }, 0);
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 1);
        let err: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 412);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(
            err["error"]["details"]["method"],
            reader_contract::methods::READING_PROGRESS_UPDATE
        );
        assert!(
            err["error"]["details"]["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected error event: {err}"
        );

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn remote_http_completion_metadata_round_trips_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let search = br#"{"protocolVersion":1,"requestId":420,"method":"book.search","params":{"sourceId":"ffi-http-src","searchRequest":{"url":"https://books.example.test/search?q=abi","headers":{"Accept":"application/json"}},"source":{"sourceId":"ffi-http-src","name":"FFI HTTP Source","baseUrl":"https://books.example.test","rules":{"search":[{"kind":"jsonPath","path":"$.books[*]"}]}}}}"#;
        assert_eq!(unsafe { send(handle, search.as_ptr(), search.len()) }, 0);

        let evs = wait_events(&events, 1);
        let req: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["protocolVersion"], 1);
        assert_eq!(req["requestId"], 420);
        assert_eq!(req["capability"], "http.execute");
        assert_eq!(
            req["params"]["url"],
            "https://books.example.test/search?q=abi"
        );
        assert_eq!(req["params"]["method"], "GET");
        assert_eq!(req["params"]["headers"]["Accept"], "application/json");
        let operation_id = req["operationId"].as_u64().unwrap();

        let complete = serde_json::json!({
            "protocolVersion": 1,
            "requestId": 421,
            "method": "host.complete",
            "params": {
                "operationId": operation_id,
                "result": {
                    "status": 200,
                    "headers": { "content-type": "application/json" },
                    "body": "{\"books\":[]}"
                }
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
        assert_eq!(res["protocolVersion"], 1);
        assert_eq!(res["requestId"], 420);
        assert!(res["data"]["books"].as_array().is_some_and(Vec::is_empty));
        assert_eq!(res["data"]["http"]["status"], 200);
        assert_eq!(
            res["data"]["http"]["headers"]["content-type"],
            "application/json"
        );
        assert_eq!(last_error_code(), 0);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn runtime_shutdown_invalid_params_does_not_stop_runtime_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let invalid_shutdown = include_bytes!(
            "../../../protocol/fixtures/conformance/commands/invalid-runtime-shutdown-unknown-field.json"
        );
        assert_eq!(
            unsafe { send(handle, invalid_shutdown.as_ptr(), invalid_shutdown.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 1);
        let err: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 331);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(
            err["error"]["details"]["method"],
            reader_contract::methods::RUNTIME_SHUTDOWN
        );
        assert!(
            err["error"]["details"]["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected error event: {err}"
        );

        let ping = br#"{"protocolVersion":1,"requestId":433,"method":"runtime.ping","params":{}}"#;
        assert_eq!(unsafe { send(handle, ping.as_ptr(), ping.len()) }, 0);
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 2);
        let result: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(result["type"], "result");
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(result["requestId"], 433);
        assert_eq!(result["data"]["pong"], true);

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn runtime_shutdown_cancels_pending_and_blocks_future_sends_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let smoke = br#"{"protocolVersion":1,"requestId":430,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}}"#;
        assert_eq!(unsafe { send(handle, smoke.as_ptr(), smoke.len()) }, 0);
        let evs = wait_events(&events, 1);
        let req: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(req["type"], "host.request");
        assert_eq!(req["requestId"], 430);

        let shutdown =
            br#"{"protocolVersion":1,"requestId":431,"method":"runtime.shutdown","params":{}}"#;
        assert_eq!(
            unsafe { send(handle, shutdown.as_ptr(), shutdown.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 3);
        let cancelled: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(cancelled["type"], "error");
        assert_eq!(cancelled["requestId"], 430);
        assert_eq!(cancelled["error"]["code"], "CANCELLED");

        let result: serde_json::Value = serde_json::from_slice(&evs[2]).unwrap();
        assert_eq!(result["type"], "result");
        assert_eq!(result["requestId"], 431);
        assert_eq!(result["data"]["shuttingDown"], true);
        assert_eq!(
            result["data"]["cancelledRequestIds"],
            serde_json::json!([430])
        );

        let ping = br#"{"protocolVersion":1,"requestId":432,"method":"runtime.ping","params":{}}"#;
        assert_eq!(
            unsafe { send(handle, ping.as_ptr(), ping.len()) },
            status::send::PROTOCOL_ERROR
        );
        assert_eq!(last_error_code(), last_error::code_of(ErrorCode::Internal));
        assert!(last_error_message().contains("shutting down"));
        std::thread::sleep(std::time::Duration::from_millis(25));
        assert_eq!(events.0.lock().unwrap().len(), 3);

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
        assert_eq!(err["protocolVersion"], 1);
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
        assert_eq!(req["protocolVersion"], 1);
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
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 500);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn host_complete_invalid_params_emit_error_events_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let zero_operation = include_bytes!(
            "../../../protocol/fixtures/conformance/host/complete-operation-zero.json"
        );
        assert_eq!(
            unsafe { send(handle, zero_operation.as_ptr(), zero_operation.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 1);
        let err: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 305);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(err["error"]["details"]["operationId"], 0);

        let unknown_field = include_bytes!(
            "../../../protocol/fixtures/conformance/host/complete-unknown-field.json"
        );
        assert_eq!(
            unsafe { send(handle, unknown_field.as_ptr(), unknown_field.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 2);
        let err: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 314);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(err["error"]["details"]["method"], "host.complete");
        assert!(
            err["error"]["details"]["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected error event: {err}"
        );

        assert_eq!(unsafe { destroy(handle) }, 0);
    }

    #[test]
    fn host_error_invalid_params_emit_error_events_via_c_abi() {
        let events = Arc::new(CapturedEvents(Mutex::new(Vec::new())));
        let handle = make_runtime(&events);

        let zero_operation =
            include_bytes!("../../../protocol/fixtures/conformance/host/error-operation-zero.json");
        assert_eq!(
            unsafe { send(handle, zero_operation.as_ptr(), zero_operation.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 1);
        let err: serde_json::Value = serde_json::from_slice(&evs[0]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 306);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(err["error"]["details"]["operationId"], 0);

        let unknown_field =
            include_bytes!("../../../protocol/fixtures/conformance/host/error-unknown-field.json");
        assert_eq!(
            unsafe { send(handle, unknown_field.as_ptr(), unknown_field.len()) },
            0
        );
        assert_eq!(last_error_code(), 0);

        let evs = wait_events(&events, 2);
        let err: serde_json::Value = serde_json::from_slice(&evs[1]).unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["protocolVersion"], 1);
        assert_eq!(err["requestId"], 315);
        assert_eq!(err["error"]["code"], "INVALID_PARAMS");
        assert_eq!(err["error"]["details"]["method"], "host.error");
        assert!(
            err["error"]["details"]["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected error event: {err}"
        );

        assert_eq!(unsafe { destroy(handle) }, 0);
    }
}

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use reader_contract::{methods, Command, CoreError, ErrorCode, Event, RuntimeConfig};
use reader_runtime::{EventSink, Runtime};
use serde_json::{json, Value};

const EVENT_TIMEOUT: Duration = Duration::from_secs(2);
const NO_EVENT_TIMEOUT: Duration = Duration::from_millis(50);

const VALID_RUNTIME_PING: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-ping.json");
const VALID_CORE_INFO: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-core-info.json");
const INVALID_MALFORMED_COMMAND: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/invalid-malformed-json.json");
const INVALID_UNSUPPORTED_PROTOCOL: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-unsupported-protocol.json"
);
const INVALID_MISSING_REQUEST_ID: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/invalid-missing-request-id.json");
const INVALID_REQUEST_ID_ZERO: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/invalid-request-id-zero.json");
const INVALID_EMPTY_METHOD: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/invalid-empty-method.json");
const INVALID_METHOD_WHITESPACE: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/invalid-method-whitespace.json");
const INVALID_METHOD_EMPTY_SEGMENT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-method-empty-segment.json"
);
const INVALID_PARAMS_NOT_OBJECT: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/invalid-params-not-object.json");

const VALID_CONFIG_EMPTY: &str =
    include_str!("../../../protocol/fixtures/conformance/configs/valid-empty.json");
const VALID_CONFIG_DIRECTORIES: &str =
    include_str!("../../../protocol/fixtures/conformance/configs/valid-directories.json");
const INVALID_CONFIG_MALFORMED: &str =
    include_str!("../../../protocol/fixtures/conformance/configs/invalid-malformed-json.json");
const INVALID_CONFIG_UNKNOWN_FIELD: &str =
    include_str!("../../../protocol/fixtures/conformance/configs/invalid-unknown-field.json");
const INVALID_CONFIG_EMPTY_DATA_DIR: &str = include_str!(
    "../../../protocol/fixtures/conformance/configs/invalid-empty-data-directory.json"
);

const HOST_REQUEST: &str = include_str!("../../../protocol/fixtures/conformance/host/request.json");
const HOST_REQUEST_INVALID_CAPABILITY_WHITESPACE: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/request-invalid-capability-whitespace.json"
);
const HOST_REQUEST_INVALID_CAPABILITY_EMPTY_SEGMENT: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/request-invalid-capability-empty-segment.json"
);
const HOST_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete.json");
const HOST_ERROR: &str = include_str!("../../../protocol/fixtures/conformance/host/error.json");
const HOST_UNKNOWN_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/unknown-complete.json");
const HOST_COMPLETE_OPERATION_ZERO: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete-operation-zero.json");
const HOST_ERROR_OPERATION_ZERO: &str =
    include_str!("../../../protocol/fixtures/conformance/host/error-operation-zero.json");
const HOST_HTTP_COMPLETE_WITH_METADATA: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-with-metadata.json");
const HOST_HTTP_COMPLETE_INVALID_STATUS: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-status.json");

const VALID_RUNTIME_CANCEL: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json");
const VALID_RUNTIME_STATUS: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-status.json");
const INVALID_RUNTIME_CANCEL_TARGET_ZERO: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-target-zero.json"
);

const CANCEL_UNKNOWN: &str =
    include_str!("../../../protocol/fixtures/conformance/cancel/unknown.json");
const CANCEL_COMPLETED: &str =
    include_str!("../../../protocol/fixtures/conformance/cancel/completed.json");

struct ChannelSink {
    tx: mpsc::Sender<Event>,
}

impl EventSink for ChannelSink {
    fn emit(&self, event: &Event) {
        let _ = self.tx.send(event.clone());
    }
}

pub(crate) struct ConformanceReport {
    cases: Vec<CaseResult>,
}

struct CaseResult {
    name: &'static str,
    passed: bool,
    message: String,
}

impl ConformanceReport {
    pub(crate) fn failed_count(&self) -> usize {
        self.cases.iter().filter(|case| !case.passed).count()
    }

    pub(crate) fn to_json(&self) -> String {
        let failed = self.failed_count();
        let passed = self.cases.len() - failed;
        json!({
            "type": "conformance",
            "passed": passed,
            "failed": failed,
            "cases": self.cases.iter().map(|case| {
                json!({
                    "name": case.name,
                    "status": if case.passed { "passed" } else { "failed" },
                    "message": case.message,
                })
            }).collect::<Vec<_>>()
        })
        .to_string()
    }
}

pub(crate) fn run_conformance() -> ConformanceReport {
    let mut report = ConformanceReport { cases: Vec::new() };

    record(&mut report, "valid-command-runtime-ping", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_RUNTIME_PING)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 101 && data["pong"] == true => Ok(()),
            other => Err(format!("unexpected event {other:?}")),
        }
    });

    record(&mut report, "valid-command-core-info", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_CORE_INFO)?;
        match recv_event(&rx)? {
            Event::Result { data, .. } => {
                let capabilities = data["capabilities"]
                    .as_array()
                    .ok_or_else(|| "missing capabilities array".to_string())?;
                if capabilities
                    .iter()
                    .any(|value| value == methods::RUNTIME_PING)
                {
                    Ok(())
                } else {
                    Err(format!("runtime.ping missing from capabilities: {data}"))
                }
            }
            other => Err(format!("unexpected event {other:?}")),
        }
    });

    for (name, json, expected) in [
        (
            "invalid-command-malformed-json",
            INVALID_MALFORMED_COMMAND,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-command-unsupported-protocol",
            INVALID_UNSUPPORTED_PROTOCOL,
            ErrorCode::InvalidProtocolVersion,
        ),
        (
            "invalid-command-missing-request-id",
            INVALID_MISSING_REQUEST_ID,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-command-request-id-zero",
            INVALID_REQUEST_ID_ZERO,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-command-empty-method",
            INVALID_EMPTY_METHOD,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-command-method-whitespace",
            INVALID_METHOD_WHITESPACE,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-command-method-empty-segment",
            INVALID_METHOD_EMPTY_SEGMENT,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-command-params-not-object",
            INVALID_PARAMS_NOT_OBJECT,
            ErrorCode::InvalidParams,
        ),
    ] {
        record(&mut report, name, || expect_send_json_error(json, expected));
    }

    for (name, json) in [
        ("valid-config-empty", VALID_CONFIG_EMPTY),
        ("valid-config-directories", VALID_CONFIG_DIRECTORIES),
    ] {
        record(&mut report, name, || {
            RuntimeConfig::from_json_bytes(json.as_bytes())
                .map(|_| ())
                .map_err(|err| format!("expected valid config, got {err:?}"))
        });
    }

    for (name, json, expected) in [
        (
            "invalid-config-malformed-json",
            INVALID_CONFIG_MALFORMED,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-config-unknown-field",
            INVALID_CONFIG_UNKNOWN_FIELD,
            ErrorCode::InvalidMessage,
        ),
        (
            "invalid-config-empty-data-directory",
            INVALID_CONFIG_EMPTY_DATA_DIR,
            ErrorCode::InvalidParams,
        ),
    ] {
        record(&mut report, name, || {
            let err = RuntimeConfig::from_json_bytes(json.as_bytes())
                .err()
                .ok_or_else(|| "expected config parse error".to_string())?;
            expect_code(err, expected)
        });
    }

    record(&mut report, "host-request-event-shape", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
        match recv_event(&rx)? {
            Event::HostRequest {
                request_id,
                operation_id,
                capability,
                params,
                ..
            } if request_id == 301
                && operation_id == 1
                && capability == "host.smoke.echo"
                && params["message"] == "conformance host request" =>
            {
                Ok(())
            }
            other => Err(format!("unexpected host.request event {other:?}")),
        }
    });

    record(
        &mut report,
        "host-request-invalid-capability-whitespace",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_REQUEST_INVALID_CAPABILITY_WHITESPACE)?;
            expect_event_error(&rx, 307, ErrorCode::InvalidParams)
        },
    );

    record(
        &mut report,
        "host-request-invalid-capability-empty-segment",
        || {
            let (_runtime, rx) =
                send_to_fresh_runtime(HOST_REQUEST_INVALID_CAPABILITY_EMPTY_SEGMENT)?;
            expect_event_error(&rx, 308, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "host-complete-routes-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
        expect_host_request(&rx)?;
        runtime
            .send_json(HOST_COMPLETE.as_bytes())
            .map_err(|err| format!("host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 301 && data["status"] == "ok" => Ok(()),
            other => Err(format!("unexpected host.complete result {other:?}")),
        }
    });

    record(&mut report, "host-error-routes-error", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
        expect_host_request(&rx)?;
        runtime
            .send_json(HOST_ERROR.as_bytes())
            .map_err(|err| format!("host.error send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 301 && error.code == ErrorCode::Internal && error.retryable => {
                Ok(())
            }
            other => Err(format!("unexpected host.error event {other:?}")),
        }
    });

    record(&mut report, "host-complete-unknown-operation", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_UNKNOWN_COMPLETE)?;
        expect_event_error(&rx, 304, ErrorCode::InvalidParams)
    });

    record(&mut report, "host-complete-zero-operation-id", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_COMPLETE_OPERATION_ZERO)?;
        expect_event_error(&rx, 305, ErrorCode::InvalidParams)
    });

    record(&mut report, "host-error-zero-operation-id", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_OPERATION_ZERO)?;
        expect_event_error(&rx, 306, ErrorCode::InvalidParams)
    });

    record(
        &mut report,
        "host-complete-after-completed-operation",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
            expect_host_request(&rx)?;
            runtime
                .send_json(HOST_COMPLETE.as_bytes())
                .map_err(|err| format!("first host.complete send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result { .. } => {}
                other => return Err(format!("expected first completion result, got {other:?}")),
            }
            runtime
                .send_json(HOST_COMPLETE.as_bytes())
                .map_err(|err| format!("second host.complete send failed: {err:?}"))?;
            expect_event_error(&rx, 302, ErrorCode::InvalidParams)
        },
    );

    record(
        &mut report,
        "remote-http-complete-carries-status-headers",
        || {
            let (runtime, rx) = fresh_runtime();
            runtime
                .send(remote_http_search_command(501))
                .map_err(|err| format!("book.search send failed: {err:?}"))?;
            expect_http_host_request(&rx)?;
            runtime
                .send_json(HOST_HTTP_COMPLETE_WITH_METADATA.as_bytes())
                .map_err(|err| format!("http host.complete send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 501
                    && data["books"].as_array().is_some_and(Vec::is_empty)
                    && data["http"]["status"] == 200
                    && data["http"]["headers"]["content-type"] == "application/json" =>
                {
                    Ok(())
                }
                other => Err(format!("unexpected http completion result {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "remote-http-complete-rejects-invalid-status",
        || {
            let (runtime, rx) = fresh_runtime();
            runtime
                .send(remote_http_search_command(504))
                .map_err(|err| format!("book.search send failed: {err:?}"))?;
            expect_http_host_request(&rx)?;
            runtime
                .send_json(HOST_HTTP_COMPLETE_INVALID_STATUS.as_bytes())
                .map_err(|err| format!("invalid http host.complete send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 504
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("status") =>
                {
                    Ok(())
                }
                other => Err(format!("unexpected invalid-status event {other:?}")),
            }
        },
    );

    record(&mut report, "cancel-unknown-id-idempotent", || {
        let (runtime, rx) = fresh_runtime();
        let id = serde_json::from_str::<Value>(CANCEL_UNKNOWN)
            .map_err(|err| format!("cancel fixture parse failed: {err}"))?["requestId"]
            .as_u64()
            .ok_or_else(|| "cancel unknown fixture missing requestId".to_string())?;
        runtime.cancel(id);
        runtime.cancel(id);
        expect_no_event(&rx)
    });

    record(&mut report, "cancel-completed-id-idempotent", || {
        let fixture = serde_json::from_str::<Value>(CANCEL_COMPLETED)
            .map_err(|err| format!("cancel completed fixture parse failed: {err}"))?;
        let command_json = fixture["command"].to_string();
        let cancel_id = fixture["cancelRequestId"]
            .as_u64()
            .ok_or_else(|| "cancel completed fixture missing cancelRequestId".to_string())?;
        let (runtime, rx) = fresh_runtime();
        runtime
            .send_json(command_json.as_bytes())
            .map_err(|err| format!("completed command send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result { .. } => {}
            other => return Err(format!("expected completed command result, got {other:?}")),
        }
        runtime.cancel(cancel_id);
        runtime.cancel(cancel_id);
        expect_no_event(&rx)
    });

    // --- runtime.cancel JSON command ---------------------------------------

    record(
        &mut report,
        "runtime-cancel-cancels-pending-host-request",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
            expect_host_request(&rx)?;
            // Fixture cancels target requestId 301 via the JSON protocol.
            runtime
                .send_json(VALID_RUNTIME_CANCEL.as_bytes())
                .map_err(|err| format!("runtime.cancel send failed: {err:?}"))?;
            // First: CANCELLED error routed to the original request 301.
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 301 && error.code == ErrorCode::Cancelled => {}
                other => return Err(format!("expected cancelled error for 301, got {other:?}")),
            }
            // Second: result for the cancel command itself (requestId 310).
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 310 && data["cancelled"] == true => {}
                other => return Err(format!("expected cancel result true, got {other:?}")),
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "runtime-cancel-unknown-id-returns-false",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(VALID_RUNTIME_CANCEL)?;
            // Target 301 is not active here → cancelled:false, no other events.
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 310 && data["cancelled"] == false => Ok(()),
                other => Err(format!("expected cancelled:false result, got {other:?}")),
            }
        },
    );

    record(&mut report, "runtime-cancel-zero-target-id", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_RUNTIME_CANCEL_TARGET_ZERO)?;
        expect_event_error(&rx, 311, ErrorCode::InvalidParams)
    });

    // --- runtime.status JSON command ---------------------------------------

    record(
        &mut report,
        "runtime-status-empty-runtime-excludes-status-command",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(VALID_RUNTIME_STATUS)?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 320
                    && data["activeRequestCount"] == 0
                    && data["activeRequestIds"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                    && data["pendingHostOperationCount"] == 0
                    && data["pendingHostOperations"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                    && data["shuttingDown"] == false =>
                {
                    Ok(())
                }
                other => Err(format!("expected empty runtime.status, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "runtime-status-reports-pending-host-operation-metadata",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
            expect_host_request(&rx)?;
            runtime
                .send_json(VALID_RUNTIME_STATUS.as_bytes())
                .map_err(|err| format!("runtime.status send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 320
                    && data["activeRequestCount"] == 1
                    && data["activeRequestIds"] == json!([301])
                    && data["pendingHostOperationCount"] == 1 =>
                {
                    let operations = data["pendingHostOperations"]
                        .as_array()
                        .ok_or_else(|| "pendingHostOperations missing".to_string())?;
                    let Some(operation) = operations.first() else {
                        return Err("pendingHostOperations empty".to_string());
                    };
                    if operation["operationId"] != 1
                        || operation["requestId"] != 301
                        || operation["capability"] != "host.smoke.echo"
                        || operation["state"] != "pending"
                        || operation.get("params").is_some()
                    {
                        return Err(format!("unexpected pending operation {operation}"));
                    }
                    Ok(())
                }
                other => Err(format!("expected pending runtime.status, got {other:?}")),
            }
        },
    );

    report
}

fn record(
    report: &mut ConformanceReport,
    name: &'static str,
    run: impl FnOnce() -> Result<(), String>,
) {
    match run() {
        Ok(()) => report.cases.push(CaseResult {
            name,
            passed: true,
            message: "ok".to_string(),
        }),
        Err(message) => report.cases.push(CaseResult {
            name,
            passed: false,
            message,
        }),
    }
}

fn fresh_runtime() -> (Runtime, Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let sink = Arc::new(ChannelSink { tx });
    (Runtime::new(sink), rx)
}

fn send_to_fresh_runtime(command_json: &str) -> Result<(Runtime, Receiver<Event>), String> {
    let (runtime, rx) = fresh_runtime();
    runtime
        .send_json(command_json.as_bytes())
        .map_err(|err| format!("send_json failed: {err:?}"))?;
    Ok((runtime, rx))
}

fn expect_send_json_error(command_json: &str, expected: ErrorCode) -> Result<(), String> {
    let (runtime, _rx) = fresh_runtime();
    let err = runtime
        .send_json(command_json.as_bytes())
        .err()
        .ok_or_else(|| "expected send_json error".to_string())?;
    expect_code(err, expected)
}

fn expect_code(error: CoreError, expected: ErrorCode) -> Result<(), String> {
    if error.code == expected {
        Ok(())
    } else {
        Err(format!("expected {expected:?}, got {error:?}"))
    }
}

fn recv_event(rx: &Receiver<Event>) -> Result<Event, String> {
    rx.recv_timeout(EVENT_TIMEOUT)
        .map_err(|err| format!("timed out waiting for runtime event: {err}"))
}

fn expect_no_event(rx: &Receiver<Event>) -> Result<(), String> {
    match rx.recv_timeout(NO_EVENT_TIMEOUT) {
        Ok(event) => Err(format!("expected no event, got {event:?}")),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
        Err(err) => Err(format!("event channel closed: {err}")),
    }
}

fn expect_host_request(rx: &Receiver<Event>) -> Result<(), String> {
    match recv_event(rx)? {
        Event::HostRequest { .. } => Ok(()),
        other => Err(format!("expected host.request, got {other:?}")),
    }
}

fn expect_http_host_request(rx: &Receiver<Event>) -> Result<(), String> {
    match recv_event(rx)? {
        Event::HostRequest {
            request_id,
            operation_id,
            capability,
            params,
            ..
        } if request_id == 501 || request_id == 504 => {
            if operation_id != 1 {
                return Err(format!("expected operationId=1, got {operation_id}"));
            }
            if capability != "http.execute" {
                return Err(format!("expected http.execute, got {capability}"));
            }
            if params["url"] != "https://books.example.test/search?q=empty" {
                return Err(format!("unexpected http params {params}"));
            }
            Ok(())
        }
        other => Err(format!("expected http.execute host.request, got {other:?}")),
    }
}

fn remote_http_search_command(request_id: u64) -> Command {
    Command::new(
        request_id,
        methods::BOOK_SEARCH,
        json!({
            "sourceId": "conformance-src",
            "searchRequest": {
                "url": "https://books.example.test/search?q=empty",
                "headers": { "Accept": "application/json" }
            },
            "source": {
                "sourceId": "conformance-src",
                "name": "Conformance Source",
                "baseUrl": "https://books.example.test",
                "rules": {
                    "search": [ { "kind": "jsonPath", "path": "$.books[*]" } ]
                }
            }
        }),
    )
}

fn expect_event_error(
    rx: &Receiver<Event>,
    expected_request_id: u64,
    expected_code: ErrorCode,
) -> Result<(), String> {
    match recv_event(rx)? {
        Event::Error {
            request_id, error, ..
        } if request_id == expected_request_id && error.code == expected_code => Ok(()),
        other => Err(format!(
            "expected error requestId={expected_request_id} code={expected_code:?}, got {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conformance_suite_passes() {
        let report = super::run_conformance();
        assert_eq!(report.failed_count(), 0, "{}", report.to_json());
    }
}

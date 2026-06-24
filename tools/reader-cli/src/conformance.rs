use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use reader_contract::{methods, CoreError, ErrorCode, Event, RuntimeConfig};
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
const HOST_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete.json");
const HOST_ERROR: &str = include_str!("../../../protocol/fixtures/conformance/host/error.json");
const HOST_UNKNOWN_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/unknown-complete.json");

const VALID_RUNTIME_CANCEL: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json");

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
                other => {
                    return Err(format!("expected cancelled error for 301, got {other:?}"))
                }
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

    record(&mut report, "runtime-cancel-unknown-id-returns-false", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_RUNTIME_CANCEL)?;
        // Target 301 is not active here → cancelled:false, no other events.
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 310 && data["cancelled"] == false => Ok(()),
            other => Err(format!("expected cancelled:false result, got {other:?}")),
        }
    });

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

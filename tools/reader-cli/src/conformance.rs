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
const VALID_SOURCE_IMPORT: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-source-import.json");
const VALID_BOOK_SEARCH: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-book-search.json");
const VALID_BOOK_DETAIL: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-book-detail.json");
const VALID_BOOK_TOC: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-book-toc.json");
const VALID_CHAPTER_CONTENT: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-chapter-content.json");
const VALID_READING_PROGRESS_UPDATE: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/valid-reading-progress-update.json"
);
const INVALID_RUNTIME_PING_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-runtime-ping-unknown-field.json"
);
const INVALID_CORE_INFO_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-core-info-unknown-field.json"
);
const INVALID_SOURCE_IMPORT_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-source-import-unknown-field.json"
);
const INVALID_SOURCE_IMPORT_NAME_WHITESPACE: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-source-import-name-whitespace.json"
);
const INVALID_SOURCE_IMPORT_RULES_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-source-import-rules-not-object.json"
);
const INVALID_BOOK_SEARCH_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-unknown-field.json"
);
const INVALID_BOOK_SEARCH_REQUEST_METHOD_EMPTY: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-method-empty.json"
);
const INVALID_BOOK_SEARCH_REQUEST_HEADERS_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-headers-not-object.json"
);
const INVALID_BOOK_SEARCH_REQUEST_URL_WHITESPACE: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-url-whitespace.json"
);
const INVALID_BOOK_SEARCH_SOURCE_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-source-not-object.json"
);
const INVALID_BOOK_DETAIL_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-detail-unknown-field.json"
);
const INVALID_BOOK_DETAIL_BOOK_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-detail-book-not-object.json"
);
const INVALID_BOOK_TOC_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-toc-unknown-field.json"
);
const INVALID_CHAPTER_CONTENT_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-chapter-content-unknown-field.json"
);
const INVALID_READING_PROGRESS_UPDATE_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-reading-progress-update-unknown-field.json"
);
const INVALID_READING_PROGRESS_UPDATE_PROGRESS_OUT_OF_RANGE: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-reading-progress-update-progress-out-of-range.json"
);
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
const HOST_REQUEST_UNKNOWN_FIELD: &str =
    include_str!("../../../protocol/fixtures/conformance/host/request-unknown-field.json");
const HOST_REQUEST_INVALID_CAPABILITY_WHITESPACE: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/request-invalid-capability-whitespace.json"
);
const HOST_REQUEST_INVALID_CAPABILITY_EMPTY_SEGMENT: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/request-invalid-capability-empty-segment.json"
);
const HOST_REQUEST_PARAMS_NOT_OBJECT: &str =
    include_str!("../../../protocol/fixtures/conformance/host/request-params-not-object.json");
const HOST_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete.json");
const HOST_ERROR: &str = include_str!("../../../protocol/fixtures/conformance/host/error.json");
const HOST_UNKNOWN_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/unknown-complete.json");
const HOST_COMPLETE_UNKNOWN_FIELD: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete-unknown-field.json");
const HOST_COMPLETE_RESULT_NOT_OBJECT: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete-result-not-object.json");
const HOST_COMPLETE_OPERATION_ZERO: &str =
    include_str!("../../../protocol/fixtures/conformance/host/complete-operation-zero.json");
const HOST_ERROR_OPERATION_ZERO: &str =
    include_str!("../../../protocol/fixtures/conformance/host/error-operation-zero.json");
const HOST_ERROR_UNKNOWN_FIELD: &str =
    include_str!("../../../protocol/fixtures/conformance/host/error-unknown-field.json");
const HOST_ERROR_DETAILS_NOT_OBJECT: &str =
    include_str!("../../../protocol/fixtures/conformance/host/error-details-not-object.json");
const HOST_HTTP_COMPLETE_WITH_METADATA: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-with-metadata.json");
const HOST_HTTP_COMPLETE_INVALID_STATUS: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-status.json");
const HOST_HTTP_COMPLETE_INVALID_HEADERS: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-headers.json");

const VALID_RUNTIME_CANCEL: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json");
const VALID_RUNTIME_STATUS: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-status.json");
const VALID_RUNTIME_SHUTDOWN: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-shutdown.json");
const INVALID_RUNTIME_CANCEL_TARGET_ZERO: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-target-zero.json"
);
const INVALID_RUNTIME_CANCEL_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-unknown-field.json"
);
const INVALID_RUNTIME_STATUS_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-runtime-status-unknown-field.json"
);
const INVALID_RUNTIME_SHUTDOWN_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-runtime-shutdown-unknown-field.json"
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
        let event = recv_event(&rx)?;
        let event_json =
            serde_json::to_value(&event).map_err(|err| format!("event serialize failed: {err}"))?;
        match &event {
            Event::Result {
                request_id, data, ..
            } if *request_id == 101 && data["pong"] == true && event_json["data"].is_object() => {
                Ok(())
            }
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

    record(&mut report, "runtime-ping-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_RUNTIME_PING_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 202, ErrorCode::InvalidParams)
    });

    record(&mut report, "core-info-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_CORE_INFO_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 212, ErrorCode::InvalidParams)
    });

    record(&mut report, "valid-command-source-import", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_SOURCE_IMPORT)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 401
                && data["sourceId"] == "conformance-source"
                && data["imported"] == true =>
            {
                Ok(())
            }
            other => Err(format!("unexpected source.import result {other:?}")),
        }
    });

    record(&mut report, "source-import-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_SOURCE_IMPORT_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 402, ErrorCode::InvalidParams)
    });

    record(&mut report, "source-import-rejects-whitespace-name", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_SOURCE_IMPORT_NAME_WHITESPACE)?;
        expect_event_error(&rx, 417, ErrorCode::InvalidParams)
    });

    record(
        &mut report,
        "source-import-rejects-non-object-rules",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(INVALID_SOURCE_IMPORT_RULES_NOT_OBJECT)?;
            expect_event_error(&rx, 418, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "valid-command-book-search", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOK_SEARCH)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 403
                && data["books"]
                    .as_array()
                    .and_then(|books| books.first())
                    .is_some_and(|book| book["title"] == "Dune") =>
            {
                Ok(())
            }
            other => Err(format!("unexpected book.search result {other:?}")),
        }
    });

    record(&mut report, "book-search-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 404, ErrorCode::InvalidParams)
    });

    record(&mut report, "book-search-rejects-empty-http-method", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_REQUEST_METHOD_EMPTY)?;
        expect_event_error(&rx, 414, ErrorCode::InvalidParams)
    });

    record(
        &mut report,
        "book-search-rejects-non-object-http-headers",
        || {
            let (_runtime, rx) =
                send_to_fresh_runtime(INVALID_BOOK_SEARCH_REQUEST_HEADERS_NOT_OBJECT)?;
            expect_event_error(&rx, 415, ErrorCode::InvalidParams)
        },
    );

    record(
        &mut report,
        "book-search-rejects-whitespace-http-url",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_REQUEST_URL_WHITESPACE)?;
            expect_event_error(&rx, 416, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "book-search-rejects-non-object-source", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_SOURCE_NOT_OBJECT)?;
        expect_event_error(&rx, 419, ErrorCode::InvalidParams)
    });

    record(&mut report, "valid-command-book-detail", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOK_DETAIL)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 405
                && data["book"]["title"] == "Dune"
                && data["book"]["author"] == "Frank Herbert" =>
            {
                Ok(())
            }
            other => Err(format!("unexpected book.detail result {other:?}")),
        }
    });

    record(&mut report, "book-detail-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_DETAIL_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 406, ErrorCode::InvalidParams)
    });

    record(&mut report, "book-detail-rejects-non-object-book", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_DETAIL_BOOK_NOT_OBJECT)?;
        expect_event_error(&rx, 421, ErrorCode::InvalidParams)
    });

    record(&mut report, "valid-command-book-toc", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOK_TOC)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 407
                && data["toc"]
                    .as_array()
                    .and_then(|toc| toc.first())
                    .is_some_and(|entry| entry["title"] == "C1") =>
            {
                Ok(())
            }
            other => Err(format!("unexpected book.toc result {other:?}")),
        }
    });

    record(&mut report, "book-toc-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_TOC_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 408, ErrorCode::InvalidParams)
    });

    record(&mut report, "valid-command-chapter-content", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_CHAPTER_CONTENT)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 409
                && data["chapterTitle"] == "C1"
                && data["content"] == "Hello\nWorld"
                && data["via"] == "rule" =>
            {
                Ok(())
            }
            other => Err(format!("unexpected chapter.content result {other:?}")),
        }
    });

    record(
        &mut report,
        "chapter-content-rejects-unknown-params",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(INVALID_CHAPTER_CONTENT_UNKNOWN_FIELD)?;
            expect_event_error(&rx, 410, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "valid-command-reading-progress-update", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_READING_PROGRESS_UPDATE)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 411
                && data["bookId"] == "1"
                && data["chapterIndex"] == 2
                && data["chapterOffset"] == 128
                && data["chapterProgress"] == 0.5
                && data["stored"] == true =>
            {
                Ok(())
            }
            other => Err(format!(
                "unexpected reading.progress.update result {other:?}"
            )),
        }
    });

    record(
        &mut report,
        "reading-progress-update-rejects-unknown-params",
        || {
            let (_runtime, rx) =
                send_to_fresh_runtime(INVALID_READING_PROGRESS_UPDATE_UNKNOWN_FIELD)?;
            expect_event_error(&rx, 412, ErrorCode::InvalidParams)
        },
    );

    record(
        &mut report,
        "reading-progress-update-rejects-progress-out-of-range",
        || {
            let (_runtime, rx) =
                send_to_fresh_runtime(INVALID_READING_PROGRESS_UPDATE_PROGRESS_OUT_OF_RANGE)?;
            expect_event_error(&rx, 413, ErrorCode::InvalidParams)
        },
    );

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
        let event = recv_event(&rx)?;
        let event_json =
            serde_json::to_value(&event).map_err(|err| format!("event serialize failed: {err}"))?;
        match &event {
            Event::HostRequest {
                request_id,
                operation_id,
                capability,
                params,
                ..
            } if *request_id == 301
                && *operation_id == 1
                && event_json["operationId"]
                    .as_u64()
                    .is_some_and(|operation_id| operation_id > 0)
                && capability == "host.smoke.echo"
                && event_json["capability"]
                    .as_str()
                    .is_some_and(is_valid_token_path)
                && params["message"] == "conformance host request"
                && event_json["params"].is_object() =>
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

    record(&mut report, "host-request-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_REQUEST_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 309, ErrorCode::InvalidParams)
    });

    record(
        &mut report,
        "host-request-rejects-non-object-params",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_REQUEST_PARAMS_NOT_OBJECT)?;
            expect_event_error(&rx, 420, ErrorCode::InvalidParams)
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

    record(&mut report, "host-complete-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_COMPLETE_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 314, ErrorCode::InvalidParams)
    });

    record(
        &mut report,
        "host-complete-rejects-non-object-result",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_COMPLETE_RESULT_NOT_OBJECT)?;
            expect_event_error(&rx, 422, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "host-complete-zero-operation-id", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_COMPLETE_OPERATION_ZERO)?;
        expect_event_error(&rx, 305, ErrorCode::InvalidParams)
    });

    record(&mut report, "host-error-zero-operation-id", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_OPERATION_ZERO)?;
        expect_event_error(&rx, 306, ErrorCode::InvalidParams)
    });

    record(&mut report, "host-error-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 315, ErrorCode::InvalidParams)
    });

    record(&mut report, "host-error-rejects-non-object-details", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_DETAILS_NOT_OBJECT)?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 423
                && error.code == ErrorCode::InvalidParams
                && error.details["source"]
                    .as_str()
                    .is_some_and(|source| source.contains("details")) =>
            {
                Ok(())
            }
            other => Err(format!("unexpected host.error details rejection {other:?}")),
        }
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
                .map_err(|err| format!("remote http command send failed: {err:?}"))?;
            expect_http_host_request(&rx, 501)?;
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
                .map_err(|err| format!("remote http command send failed: {err:?}"))?;
            expect_http_host_request(&rx, 504)?;
            runtime
                .send_json(HOST_HTTP_COMPLETE_INVALID_STATUS.as_bytes())
                .map_err(|err| format!("http host.complete send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 504
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("status") =>
                {
                    Ok(())
                }
                other => Err(format!("expected invalid http status error, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "remote-http-complete-rejects-invalid-headers",
        || {
            let (runtime, rx) = fresh_runtime();
            runtime
                .send(remote_http_search_command(506))
                .map_err(|err| format!("remote http command send failed: {err:?}"))?;
            expect_http_host_request(&rx, 506)?;
            runtime
                .send_json(HOST_HTTP_COMPLETE_INVALID_HEADERS.as_bytes())
                .map_err(|err| format!("http host.complete send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 506
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("headers") =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "expected invalid http headers error, got {other:?}"
                )),
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

    record(
        &mut report,
        "runtime-cancel-cancels-pending-host-request",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
            expect_host_request(&rx)?;
            runtime
                .send_json(VALID_RUNTIME_CANCEL.as_bytes())
                .map_err(|err| format!("runtime.cancel send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 301 && error.code == ErrorCode::Cancelled => {}
                other => return Err(format!("expected cancelled error for 301, got {other:?}")),
            }
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 310 && data["cancelled"] == true => Ok(()),
                other => Err(format!("expected cancel result true, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "runtime-cancel-unknown-id-returns-false",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(VALID_RUNTIME_CANCEL)?;
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

    record(&mut report, "runtime-cancel-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_RUNTIME_CANCEL_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 312, ErrorCode::InvalidParams)
    });

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

    record(&mut report, "runtime-status-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_RUNTIME_STATUS_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 321, ErrorCode::InvalidParams)
    });

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

    record(
        &mut report,
        "runtime-shutdown-stops-future-commands",
        || {
            let (runtime, rx) = send_to_fresh_runtime(VALID_RUNTIME_SHUTDOWN)?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 330
                    && data["shuttingDown"] == true
                    && data["cancelledRequestIds"]
                        .as_array()
                        .is_some_and(Vec::is_empty) =>
                {
                    let err = runtime
                        .send(Command::new(332, methods::RUNTIME_PING, json!({})))
                        .err()
                        .ok_or_else(|| "expected send after shutdown to fail".to_string())?;
                    if err.code != ErrorCode::Internal {
                        return Err(format!("expected INTERNAL after shutdown, got {err:?}"));
                    }
                    expect_no_event(&rx)
                }
                other => Err(format!("expected runtime.shutdown result, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "runtime-shutdown-cancels-pending-host-request",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
            expect_host_request(&rx)?;
            runtime
                .send_json(VALID_RUNTIME_SHUTDOWN.as_bytes())
                .map_err(|err| format!("runtime.shutdown send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 301 && error.code == ErrorCode::Cancelled => {}
                other => return Err(format!("expected cancelled error for 301, got {other:?}")),
            }
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 330
                    && data["shuttingDown"] == true
                    && data["cancelledRequestIds"] == json!([301]) =>
                {
                    Ok(())
                }
                other => Err(format!("expected runtime.shutdown result, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "runtime-shutdown-invalid-params-does-not-stop-runtime",
        || {
            let (runtime, rx) = send_to_fresh_runtime(INVALID_RUNTIME_SHUTDOWN_UNKNOWN_FIELD)?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 331 && error.code == ErrorCode::InvalidParams => {}
                other => {
                    return Err(format!(
                        "expected runtime.shutdown params error, got {other:?}"
                    ));
                }
            }

            runtime
                .send(Command::new(332, methods::RUNTIME_PING, json!({})))
                .map_err(|err| format!("ping after invalid shutdown failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 332 && data["pong"] == true => Ok(()),
                other => Err(format!(
                    "expected ping after invalid shutdown, got {other:?}"
                )),
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
        Event::HostRequest {
            operation_id,
            capability,
            params,
            ..
        } if operation_id > 0 && is_valid_token_path(&capability) && params.is_object() => Ok(()),
        other => Err(format!("expected host.request, got {other:?}")),
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

fn expect_http_host_request(rx: &Receiver<Event>, expected_request_id: u64) -> Result<u64, String> {
    match recv_event(rx)? {
        Event::HostRequest {
            request_id,
            operation_id,
            capability,
            params,
            ..
        } if request_id == expected_request_id
            && operation_id == 1
            && capability == "http.execute"
            && is_valid_token_path(&capability)
            && params.is_object()
            && params["url"] == "https://books.example.test/search?q=empty"
            && params["method"] == "GET"
            && params["headers"]["Accept"] == "application/json" =>
        {
            Ok(operation_id)
        }
        other => Err(format!("unexpected http host.request event {other:?}")),
    }
}

fn is_valid_token_path(value: &str) -> bool {
    value.contains('.')
        && !value.chars().any(char::is_whitespace)
        && value.split('.').all(|segment| !segment.is_empty())
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

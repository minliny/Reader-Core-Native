use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use reader_contract::{
    methods, BookDetailData, BookSearchData, BookTocData, BookshelfGetData, BookshelfListData,
    ChapterContentData, ChapterContentVia, Command, CoreError, CoreInfoData, ErrorCode, Event,
    HostCapability, HostWebViewEvaluateJavaScriptRequest, HostWebViewEvaluateJavaScriptResponse,
    LocalBookCatalogData, LocalBookParseData, PendingHostOperationStatus,
    ReadingProgressUpdateData, RssParseData, RssRefreshData, RuntimeCancelData, RuntimeConfig,
    RuntimePingData, RuntimeShutdownData, RuntimeStatus, SourceImportData, SyncBackupData,
    SyncMergeData, TtsChapterPlanData, TtsQueueNextData, TtsQueuePauseData, TtsQueuePlayData,
    TtsQueuePrevData, TtsQueueResumeData, TtsQueueState, TtsQueueStatusData, TtsQueueStopData,
    TtsSliceData, TtsSlicingStrategy, PROTOCOL_VERSION, V1_CAPABILITIES,
};
use reader_runtime::{EventSink, Runtime};
use reader_storage::{BookshelfEntry, BookshelfStore};
use serde_json::{json, Value};

const EVENT_TIMEOUT: Duration = Duration::from_secs(2);
const NO_EVENT_TIMEOUT: Duration = Duration::from_millis(50);

const VALID_RUNTIME_PING: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-runtime-ping.json");
const VALID_CORE_INFO: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-core-info.json");
const VALID_SOURCE_IMPORT: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-source-import.json");
const VALID_SOURCE_IMPORT_LEGADO_BOOKSOURCE: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/valid-source-import-legado-booksource.json"
);
const VALID_SOURCE_IMPORT_LEGADO_BOOKSOURCE_NAME_ONLY: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/valid-source-import-legado-booksource-name-only.json"
);
const VALID_BOOK_SEARCH: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-book-search.json");
const VALID_BOOK_SEARCH_AUTO_BUILD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/valid-book-search-auto-build.json"
);
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
const INVALID_SOURCE_IMPORT_BOOKSOURCE_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-source-import-booksource-not-object.json"
);
const INVALID_SOURCE_IMPORT_MISSING_NAME_AND_BOOKSOURCE_NAME: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-source-import-missing-name-and-booksource-name.json"
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
const INVALID_BOOK_SEARCH_REQUEST_RETRY_ZERO: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-retry-zero.json"
);
const INVALID_BOOK_SEARCH_REQUEST_SESSION_BLANK: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-session-blank.json"
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

const VALID_RSS_PARSE: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-rss-parse.json");
const VALID_RSS_REFRESH: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-rss-refresh.json");
const VALID_SYNC_MERGE: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-sync-merge.json");
const VALID_SYNC_BACKUP: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-sync-backup.json");
const VALID_LOCAL_BOOK_PARSE: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-local-book-parse.json");
const VALID_LOCAL_BOOK_CATALOG: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-local-book-catalog.json");
const INVALID_RSS_PARSE_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-rss-parse-unknown-field.json"
);
const INVALID_RSS_PARSE_XML_BLANK: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-rss-parse-xml-blank.json"
);
const INVALID_RSS_REFRESH_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-rss-refresh-unknown-field.json"
);
const INVALID_RSS_REFRESH_SUBSCRIPTION_ID_BLANK: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-rss-refresh-subscription-id-blank.json"
);
const INVALID_SYNC_MERGE_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-sync-merge-unknown-field.json"
);
const INVALID_SYNC_MERGE_LOCAL_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-sync-merge-local-not-object.json"
);
const INVALID_SYNC_BACKUP_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-sync-backup-unknown-field.json"
);
const INVALID_SYNC_BACKUP_PACKAGE_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-sync-backup-package-not-object.json"
);
const INVALID_LOCAL_BOOK_PARSE_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-local-book-parse-unknown-field.json"
);
const INVALID_LOCAL_BOOK_PARSE_TEXT_BLANK: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-local-book-parse-text-blank.json"
);
const INVALID_LOCAL_BOOK_CATALOG_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-local-book-catalog-unknown-field.json"
);
const INVALID_LOCAL_BOOK_CATALOG_ENTRY_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-local-book-catalog-entry-not-object.json"
);
const VALID_BOOKSHELF_LIST: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-bookshelf-list.json");
const VALID_BOOKSHELF_GET: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-bookshelf-get.json");
const INVALID_BOOKSHELF_LIST_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-bookshelf-list-unknown-field.json"
);
const INVALID_BOOKSHELF_LIST_BLANK_SOURCE_ID: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-bookshelf-list-blank-source-id.json"
);
const INVALID_BOOKSHELF_LIST_INVALID_SORT_BY: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-bookshelf-list-invalid-sort-by.json"
);
const INVALID_BOOKSHELF_GET_MISSING_BOOK_ID: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-bookshelf-get-missing-book-id.json"
);
const INVALID_BOOKSHELF_GET_BLANK_SOURCE_ID: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-bookshelf-get-blank-source-id.json"
);

const VALID_TTS_SLICE: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-slice.json");
const VALID_TTS_QUEUE_STATUS: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-status.json");
const VALID_TTS_CHAPTER_PLAN: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-chapter-plan.json");
const VALID_TTS_QUEUE_PLAY: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-play.json");
const VALID_TTS_QUEUE_PAUSE: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-pause.json");
const VALID_TTS_QUEUE_RESUME: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-resume.json");
const VALID_TTS_QUEUE_STOP: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-stop.json");
const VALID_TTS_QUEUE_NEXT: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-next.json");
const VALID_TTS_QUEUE_PREV: &str =
    include_str!("../../../protocol/fixtures/conformance/commands/valid-tts-queue-prev.json");
const INVALID_TTS_SLICE_CONTENT_EMPTY: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-slice-content-empty.json"
);
const INVALID_TTS_SLICE_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-slice-unknown-field.json"
);
const INVALID_TTS_QUEUE_STATUS_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-status-unknown-field.json"
);
const INVALID_TTS_CHAPTER_PLAN_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-chapter-plan-unknown-field.json"
);
const INVALID_TTS_QUEUE_PLAY_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-play-unknown-field.json"
);
const INVALID_TTS_QUEUE_PAUSE_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-pause-unknown-field.json"
);
const INVALID_TTS_QUEUE_RESUME_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-resume-unknown-field.json"
);
const INVALID_TTS_QUEUE_STOP_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-stop-unknown-field.json"
);
const INVALID_TTS_QUEUE_NEXT_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-next-unknown-field.json"
);
const INVALID_TTS_QUEUE_PREV_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/commands/invalid-tts-queue-prev-unknown-field.json"
);

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
const HOST_REQUEST_UNSUPPORTED_CAPABILITY: &str =
    include_str!("../../../protocol/fixtures/conformance/host/request-unsupported-capability.json");
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
const HOST_ERROR_CORE_ERROR_UNKNOWN_FIELD: &str =
    include_str!("../../../protocol/fixtures/conformance/host/error-core-error-unknown-field.json");
const HOST_ERROR_DIAGNOSTICS: &str =
    include_str!("../../../protocol/fixtures/conformance/host/error-diagnostics.json");
const HOST_ERROR_DIAGNOSTICS_DETAILS_NOT_OBJECT: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/error-diagnostics-details-not-object.json"
);
const HOST_ERROR_DIAGNOSTICS_UNKNOWN_FIELD: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/error-diagnostics-unknown-field.json"
);
const HOST_HTTP_COMPLETE_SESSION_METADATA: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-session-metadata.json");
const HOST_HTTP_COMPLETE_INVALID_STATUS: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-status.json");
const HOST_HTTP_COMPLETE_INVALID_HEADERS: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-headers.json");
const HOST_HTTP_COMPLETE_INVALID_REDIRECT: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-redirect.json");
const HOST_HTTP_COMPLETE_INVALID_COOKIE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/http-complete-invalid-cookie.json");
const HOST_WEBVIEW_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/webview-request.json");
const HOST_WEBVIEW_REQUEST_BLANK_JAVASCRIPT: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/webview-request-blank-javascript.json"
);
const HOST_WEBVIEW_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/webview-complete.json");
const HOST_WEBVIEW_COMPLETE_BLANK_FINAL_URL: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/webview-complete-blank-final-url.json"
);
const HOST_FILE_READ_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/file-read-request.json");
const HOST_FILE_WRITE_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/file-write-request.json");
const HOST_CACHE_GET_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cache-get-request.json");
const HOST_CACHE_PUT_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cache-put-request.json");
const HOST_FILE_READ_REQUEST_BLANK_PATH: &str =
    include_str!("../../../protocol/fixtures/conformance/host/file-read-request-blank-path.json");
const HOST_CACHE_PUT_REQUEST_MISSING_VALUE: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/cache-put-request-missing-value.json"
);
const HOST_FILE_READ_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/file-read-complete.json");
const HOST_FILE_READ_COMPLETE_MISSING_CONTENT: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/file-read-complete-missing-content.json"
);
const HOST_FILE_WRITE_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/file-write-complete.json");
const HOST_FILE_WRITE_COMPLETE_NOT_WRITTEN: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/file-write-complete-not-written.json"
);
const HOST_CACHE_GET_COMPLETE_HIT: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cache-get-complete-hit.json");
const HOST_CACHE_GET_COMPLETE_INVALID_HIT: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cache-get-complete-invalid-hit.json");
const HOST_CACHE_PUT_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cache-put-complete.json");
const HOST_COOKIE_GET_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cookie-get-request.json");
const HOST_COOKIE_GET_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cookie-get-complete.json");
const HOST_COOKIE_SET_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cookie-set-request.json");
const HOST_COOKIE_SET_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/cookie-set-complete.json");
const HOST_LOG_EMIT_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/log-emit-request.json");
const HOST_LOG_EMIT_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/log-emit-complete.json");
const HOST_TIME_NOW_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/time-now-request.json");
const HOST_TIME_NOW_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/time-now-complete.json");
const HOST_SYSTEM_INFO_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/system-info-request.json");
const HOST_SYSTEM_INFO_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/system-info-complete.json");
const HOST_PERSISTENCE_GET_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/persistence-get-request.json");
const HOST_PERSISTENCE_GET_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/persistence-get-complete.json");
const HOST_PERSISTENCE_PUT_REQUEST: &str =
    include_str!("../../../protocol/fixtures/conformance/host/persistence-put-request.json");
const HOST_PERSISTENCE_PUT_COMPLETE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/persistence-put-complete.json");
const HOST_COOKIE_GET_REQUEST_MISSING_SCOPE: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/cookie-get-request-missing-scope.json"
);
const HOST_LOG_EMIT_REQUEST_BLANK_MESSAGE: &str =
    include_str!("../../../protocol/fixtures/conformance/host/log-emit-request-blank-message.json");
const HOST_PERSISTENCE_GET_COMPLETE_INVALID_FOUND: &str = include_str!(
    "../../../protocol/fixtures/conformance/host/persistence-get-complete-invalid-found.json"
);

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
const DUPLICATE_ACTIVE_REQUEST_ID: &str =
    include_str!("../../../protocol/fixtures/conformance/runtime/duplicate-active-request-id.json");

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
            } if request_id == 101 => {
                let data = serde_json::from_value::<RuntimePingData>(data)
                    .map_err(|err| format!("runtime.ping data contract parse failed: {err}"))?;
                if data.pong && data.method == methods::RUNTIME_PING {
                    Ok(())
                } else {
                    Err(format!("unexpected runtime.ping data {data:?}"))
                }
            }
            other => Err(format!("unexpected event {other:?}")),
        }
    });

    record(
        &mut report,
        "runtime-ping-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "pong",
                    json!({
                        "pong": false,
                        "method": "runtime.ping"
                    }),
                    "pong",
                ),
                (
                    "method",
                    json!({
                        "pong": true,
                        "method": "core.ping"
                    }),
                    "method",
                ),
                (
                    "unknown field",
                    json!({
                        "pong": true,
                        "method": "runtime.ping",
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<RuntimePingData>(data)
                    .err()
                    .ok_or_else(|| format!("expected runtime.ping data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected runtime.ping data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(&mut report, "valid-command-core-info", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_CORE_INFO)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 102 => {
                let data = serde_json::from_value::<CoreInfoData>(data)
                    .map_err(|err| format!("core.info data contract parse failed: {err}"))?;
                if data.abi_version > 0
                    && data.protocol_version == PROTOCOL_VERSION
                    && !data.build_version.is_empty()
                    && data
                        .capabilities
                        .iter()
                        .map(String::as_str)
                        .eq(V1_CAPABILITIES.iter().copied())
                {
                    Ok(())
                } else {
                    Err(format!("unexpected core.info data {data:?}"))
                }
            }
            other => Err(format!("unexpected event {other:?}")),
        }
    });

    record(
        &mut report,
        "core-info-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "abiVersion",
                    json!({
                        "abiVersion": 0,
                        "protocolVersion": PROTOCOL_VERSION,
                        "buildVersion": "reader-core-native test",
                        "capabilities": V1_CAPABILITIES
                    }),
                    "abiVersion",
                ),
                (
                    "protocolVersion",
                    json!({
                        "abiVersion": 1,
                        "protocolVersion": PROTOCOL_VERSION + 1,
                        "buildVersion": "reader-core-native test",
                        "capabilities": V1_CAPABILITIES
                    }),
                    "protocolVersion",
                ),
                (
                    "capabilities",
                    json!({
                        "abiVersion": 1,
                        "protocolVersion": PROTOCOL_VERSION,
                        "buildVersion": "reader-core-native test",
                        "capabilities": ["runtime.ping"]
                    }),
                    "capabilities",
                ),
                (
                    "unknown field",
                    json!({
                        "abiVersion": 1,
                        "protocolVersion": PROTOCOL_VERSION,
                        "buildVersion": "reader-core-native test",
                        "capabilities": V1_CAPABILITIES,
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<CoreInfoData>(data)
                    .err()
                    .ok_or_else(|| format!("expected core.info data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected core.info data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

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
            } if request_id == 401 => {
                let data = serde_json::from_value::<SourceImportData>(data)
                    .map_err(|err| format!("source.import data contract parse failed: {err}"))?;
                if data.source_id == "conformance-source"
                    && data.name == "Conformance Source"
                    && data.imported
                {
                    Ok(())
                } else {
                    Err(format!("unexpected source.import data {data:?}"))
                }
            }
            other => Err(format!("unexpected source.import result {other:?}")),
        }
    });

    record(
        &mut report,
        "valid-command-source-import-legado-booksource",
        || {
            let (runtime, rx) = send_to_fresh_runtime(VALID_SOURCE_IMPORT_LEGADO_BOOKSOURCE)?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 411 => {
                    let data = serde_json::from_value::<SourceImportData>(data).map_err(|err| {
                        format!("source.import Legado data contract parse failed: {err}")
                    })?;
                    if data.source_id == "legado-compat-source"
                        && data.name == "Legado Compat Source"
                        && data.imported
                    {
                        Ok(())
                    } else {
                        Err(format!("unexpected Legado source.import data {data:?}"))
                    }
                }
                other => Err(format!("unexpected Legado source.import result {other:?}")),
            }?;

            let stored = runtime
                .remote_state()
                .storage()
                .get_source("legado-compat-source")
                .map_err(|err| format!("source.import storage read failed: {err}"))?
                .ok_or_else(|| "Legado source.import did not store source".to_string())?;
            if stored.book_source["ruleSearch"] != json!("div.list&&div.item;div.name&&a@text") {
                return Err(format!(
                    "Legado source.import rewrote raw rule: {}",
                    stored.book_source["ruleSearch"]
                ));
            }
            if stored.book_source["futureLegadoField"]
                != json!({
                    "nested": true,
                    "rawRule": "span.future@text"
                })
            {
                return Err(format!(
                    "Legado source.import dropped unknown field: {}",
                    stored.book_source["futureLegadoField"]
                ));
            }
            Ok(())
        },
    );

    // Legado native form: no top-level `name`, only `bookSource.bookSourceName`.
    // Core must derive the source name from `bookSource.bookSourceName`,
    // mirroring Legado `BookSource.bookSourceName` (BookSource.kt). This is the
    // form Android/iOS import when forwarding a raw Legado BookSource JSON.
    record(
        &mut report,
        "valid-command-source-import-legado-booksource-name-only",
        || {
            let (runtime, rx) =
                send_to_fresh_runtime(VALID_SOURCE_IMPORT_LEGADO_BOOKSOURCE_NAME_ONLY)?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 419 => {
                    let data = serde_json::from_value::<SourceImportData>(data).map_err(|err| {
                        format!("source.import Legado name-only data parse failed: {err}")
                    })?;
                    if data.source_id == "legado-native-name-only"
                        && data.name == "Legado Native Name Only"
                        && data.imported
                    {
                        Ok(())
                    } else {
                        Err(format!(
                            "unexpected Legado name-only source.import data {data:?}"
                        ))
                    }
                }
                other => Err(format!(
                    "unexpected Legado name-only source.import result {other:?}"
                )),
            }?;

            let stored = runtime
                .remote_state()
                .storage()
                .get_source("legado-native-name-only")
                .map_err(|err| format!("source.import storage read failed: {err}"))?
                .ok_or_else(|| "Legado name-only source.import did not store source".to_string())?;
            if stored.name != "Legado Native Name Only" {
                return Err(format!(
                    "Legado name-only source.import did not derive name: {}",
                    stored.name
                ));
            }
            if stored.book_source["bookSourceName"] != json!("Legado Native Name Only") {
                return Err(format!(
                    "Legado name-only source.import dropped bookSourceName: {}",
                    stored.book_source["bookSourceName"]
                ));
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "source-import-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "sourceId",
                    json!({
                        "sourceId": " ",
                        "name": "Conformance Source",
                        "imported": true
                    }),
                    "sourceId",
                ),
                (
                    "name",
                    json!({
                        "sourceId": "conformance-source",
                        "name": " ",
                        "imported": true
                    }),
                    "name",
                ),
                (
                    "imported",
                    json!({
                        "sourceId": "conformance-source",
                        "name": "Conformance Source",
                        "imported": false
                    }),
                    "imported",
                ),
                (
                    "unknown field",
                    json!({
                        "sourceId": "conformance-source",
                        "name": "Conformance Source",
                        "imported": true,
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<SourceImportData>(data)
                    .err()
                    .ok_or_else(|| format!("expected source.import data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected source.import data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

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

    record(
        &mut report,
        "source-import-rejects-non-object-booksource",
        || {
            let (_runtime, rx) =
                send_to_fresh_runtime(INVALID_SOURCE_IMPORT_BOOKSOURCE_NOT_OBJECT)?;
            expect_event_error(&rx, 321, ErrorCode::InvalidParams)
        },
    );

    record(
        &mut report,
        "source-import-rejects-missing-name-and-booksource-name",
        || {
            let (_runtime, rx) =
                send_to_fresh_runtime(INVALID_SOURCE_IMPORT_MISSING_NAME_AND_BOOKSOURCE_NAME)?;
            expect_event_error(&rx, 420, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "valid-command-book-search", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOK_SEARCH)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 403 => {
                let data = serde_json::from_value::<BookSearchData>(data)
                    .map_err(|err| format!("book.search data contract parse failed: {err}"))?;
                if data.source_id == "conformance-source"
                    && data.books.len() == 1
                    && data.books[0].book_id == "1"
                    && data.books[0].title == "Dune"
                    && data.books[0].extra["author"] == "Herbert"
                    && data.http.is_none()
                {
                    Ok(())
                } else {
                    Err(format!("unexpected book.search data {data:?}"))
                }
            }
            other => Err(format!("unexpected book.search result {other:?}")),
        }
    });

    // S3/S4 closure: when `searchResponse` and `searchRequest` are both
    // absent but `keyword` is present, Core auto-builds the HTTP request
    // from the source's Legado `searchUrl` template (AnalyzeUrl parity).
    record(
        &mut report,
        "valid-command-book-search-auto-build-emits-http-host-request",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOK_SEARCH_AUTO_BUILD)?;
            match recv_event(&rx)? {
                Event::HostRequest {
                    request_id,
                    operation_id,
                    capability,
                    params,
                    ..
                } if request_id == 440
                    && operation_id == 1
                    && capability == HostCapability::HttpExecute =>
                {
                    let url = params
                        .get("url")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "host.request missing url".to_string())?;
                    let method = params
                        .get("method")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "host.request missing method".to_string())?;
                    if method != "GET" {
                        return Err(format!("expected GET, got {method}"));
                    }
                    // {{key}} expands to percent-encoded "dune"; {{page}} to "2".
                    if !url.starts_with("https://auto-build.example.test/search?q=") {
                        return Err(format!("unexpected url: {url}"));
                    }
                    if !url.contains("q=dune") {
                        return Err(format!(
                            "url should contain raw keyword 'dune' (ASCII safe), got: {url}"
                        ));
                    }
                    if !url.contains("p=2") {
                        return Err(format!("url should contain p=2, got: {url}"));
                    }
                    Ok(())
                }
                other => Err(format!(
                    "expected HostRequest(HttpExecute) for auto-build, got {other:?}"
                )),
            }
        },
    );

    record(
        &mut report,
        "book-search-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "sourceId",
                    json!({
                        "sourceId": " ",
                        "books": []
                    }),
                    "sourceId",
                ),
                (
                    "books",
                    json!({
                        "sourceId": "conformance-source",
                        "books": {}
                    }),
                    "books",
                ),
                (
                    "bookId",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [
                            { "title": "Dune" }
                        ]
                    }),
                    "bookId",
                ),
                (
                    "title",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [
                            { "bookId": "1", "title": " " }
                        ]
                    }),
                    "title",
                ),
                (
                    "http.status",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [],
                        "http": { "status": 99 }
                    }),
                    "status",
                ),
                (
                    "http.headers",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [],
                        "http": { "headers": ["content-type", "application/json"] }
                    }),
                    "headers",
                ),
                (
                    "http.session",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [],
                        "http": { "session": { "id": " " } }
                    }),
                    "session.id",
                ),
                (
                    "http.redirects",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [],
                        "http": {
                            "redirects": [
                                {
                                    "status": 200,
                                    "fromUrl": "https://books.example.test/search",
                                    "toUrl": "https://books.example.test/search?q=empty"
                                }
                            ]
                        }
                    }),
                    "redirect.status",
                ),
                (
                    "http.cookies",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [],
                        "http": {
                            "cookies": [
                                {
                                    "name": " ",
                                    "value": "new"
                                }
                            ]
                        }
                    }),
                    "cookie.name",
                ),
                (
                    "unknown field",
                    json!({
                        "sourceId": "conformance-source",
                        "books": [],
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<BookSearchData>(data)
                    .err()
                    .ok_or_else(|| format!("expected book.search data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected book.search data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

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

    record(
        &mut report,
        "book-search-rejects-zero-retry-attempts",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_REQUEST_RETRY_ZERO)?;
            expect_event_error(&rx, 507, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "book-search-rejects-blank-session-id", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_REQUEST_SESSION_BLANK)?;
        expect_event_error(&rx, 508, ErrorCode::InvalidParams)
    });

    record(&mut report, "book-search-rejects-non-object-source", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_SEARCH_SOURCE_NOT_OBJECT)?;
        expect_event_error(&rx, 419, ErrorCode::InvalidParams)
    });

    record(&mut report, "valid-command-book-detail", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOK_DETAIL)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 405 => {
                let data = serde_json::from_value::<BookDetailData>(data)
                    .map_err(|err| format!("book.detail data contract parse failed: {err}"))?;
                if data.source_id == "conformance-source"
                    && data.book.book_id == "1"
                    && data.book.title == "Dune"
                    && data.book.author == "Frank Herbert"
                    && data.book.intro.as_deref() == Some("desert")
                    && data.http.is_none()
                {
                    Ok(())
                } else {
                    Err(format!("unexpected book.detail data {data:?}"))
                }
            }
            other => Err(format!("unexpected book.detail result {other:?}")),
        }
    });

    record(
        &mut report,
        "book-detail-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "sourceId",
                    json!({
                        "sourceId": " ",
                        "book": {
                            "bookId": "1",
                            "title": "Dune",
                            "author": "Frank Herbert"
                        }
                    }),
                    "sourceId",
                ),
                (
                    "book",
                    json!({
                        "sourceId": "conformance-source",
                        "book": []
                    }),
                    "book",
                ),
                (
                    "bookId",
                    json!({
                        "sourceId": "conformance-source",
                        "book": {
                            "bookId": " ",
                            "title": "Dune",
                            "author": "Frank Herbert"
                        }
                    }),
                    "bookId",
                ),
                (
                    "title",
                    json!({
                        "sourceId": "conformance-source",
                        "book": {
                            "bookId": "1",
                            "author": "Frank Herbert"
                        }
                    }),
                    "title",
                ),
                (
                    "unknown book field",
                    json!({
                        "sourceId": "conformance-source",
                        "book": {
                            "bookId": "1",
                            "title": "Dune",
                            "author": "Frank Herbert",
                            "extra": true
                        }
                    }),
                    "unknown field",
                ),
                (
                    "http.status",
                    json!({
                        "sourceId": "conformance-source",
                        "book": {
                            "bookId": "1",
                            "title": "Dune",
                            "author": "Frank Herbert"
                        },
                        "http": { "status": 99 }
                    }),
                    "status",
                ),
                (
                    "unknown top-level field",
                    json!({
                        "sourceId": "conformance-source",
                        "book": {
                            "bookId": "1",
                            "title": "Dune",
                            "author": "Frank Herbert"
                        },
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<BookDetailData>(data)
                    .err()
                    .ok_or_else(|| format!("expected book.detail data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected book.detail data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

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
            } if request_id == 407 => {
                let data = serde_json::from_value::<BookTocData>(data)
                    .map_err(|err| format!("book.toc data contract parse failed: {err}"))?;
                if data.source_id == "conformance-source"
                    && data.book_id == "1"
                    && data.toc.len() == 2
                    && data.toc[0].index == 0
                    && data.toc[0].title == "C1"
                    && data.toc[0].url == "u1"
                    && data.http.is_none()
                {
                    Ok(())
                } else {
                    Err(format!("unexpected book.toc data {data:?}"))
                }
            }
            other => Err(format!("unexpected book.toc result {other:?}")),
        }
    });

    record(
        &mut report,
        "book-toc-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "sourceId",
                    json!({
                        "sourceId": " ",
                        "bookId": "1",
                        "toc": []
                    }),
                    "sourceId",
                ),
                (
                    "bookId",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": " ",
                        "toc": []
                    }),
                    "bookId",
                ),
                (
                    "toc",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "toc": {}
                    }),
                    "toc",
                ),
                (
                    "index",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "toc": [
                            { "title": "C1", "url": "u1" }
                        ]
                    }),
                    "index",
                ),
                (
                    "title",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "toc": [
                            { "index": 0, "url": "u1" }
                        ]
                    }),
                    "title",
                ),
                (
                    "unknown toc field",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "toc": [
                            { "index": 0, "title": "C1", "url": "u1", "extra": true }
                        ]
                    }),
                    "unknown field",
                ),
                (
                    "http.status",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "toc": [],
                        "http": { "status": 99 }
                    }),
                    "status",
                ),
                (
                    "unknown top-level field",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "toc": [],
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<BookTocData>(data)
                    .err()
                    .ok_or_else(|| format!("expected book.toc data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!("unexpected book.toc data error for {label}: {err}"));
                }
            }
            Ok(())
        },
    );

    record(&mut report, "book-toc-rejects-unknown-params", || {
        let (_runtime, rx) = send_to_fresh_runtime(INVALID_BOOK_TOC_UNKNOWN_FIELD)?;
        expect_event_error(&rx, 408, ErrorCode::InvalidParams)
    });

    record(&mut report, "valid-command-chapter-content", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_CHAPTER_CONTENT)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 409 => {
                let data = serde_json::from_value::<ChapterContentData>(data)
                    .map_err(|err| format!("chapter.content data contract parse failed: {err}"))?;
                if data.source_id == "conformance-source"
                    && data.book_id == "1"
                    && data.chapter_title == "C1"
                    && data.content == json!("Hello\nWorld")
                    && data.via == ChapterContentVia::Rule
                    && data.http.is_none()
                {
                    Ok(())
                } else {
                    Err(format!("unexpected chapter.content data {data:?}"))
                }
            }
            other => Err(format!("unexpected chapter.content result {other:?}")),
        }
    });

    record(
        &mut report,
        "chapter-content-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "sourceId",
                    json!({
                        "sourceId": " ",
                        "bookId": "1",
                        "chapterTitle": "C1",
                        "content": "Hello",
                        "via": "rule"
                    }),
                    "sourceId",
                ),
                (
                    "bookId",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": " ",
                        "chapterTitle": "C1",
                        "content": "Hello",
                        "via": "rule"
                    }),
                    "bookId",
                ),
                (
                    "chapterTitle",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "content": "Hello",
                        "via": "rule"
                    }),
                    "chapterTitle",
                ),
                (
                    "content",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "chapterTitle": "C1",
                        "via": "rule"
                    }),
                    "content",
                ),
                (
                    "via",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "chapterTitle": "C1",
                        "content": "Hello",
                        "via": "native"
                    }),
                    "unknown variant",
                ),
                (
                    "http.status",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "chapterTitle": "C1",
                        "content": "Hello",
                        "via": "rule",
                        "http": { "status": 99 }
                    }),
                    "status",
                ),
                (
                    "unknown top-level field",
                    json!({
                        "sourceId": "conformance-source",
                        "bookId": "1",
                        "chapterTitle": "C1",
                        "content": "Hello",
                        "via": "rule",
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<ChapterContentData>(data)
                    .err()
                    .ok_or_else(|| {
                        format!("expected chapter.content data rejection for {label}")
                    })?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected chapter.content data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

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
            } if request_id == 411 => {
                let data =
                    serde_json::from_value::<ReadingProgressUpdateData>(data).map_err(|err| {
                        format!("reading.progress.update data contract parse failed: {err}")
                    })?;
                if data.book_id == "1"
                    && data.chapter_index == 2
                    && data.chapter_offset == 128
                    && data.chapter_progress == 0.5
                    && data.stored
                {
                    Ok(())
                } else {
                    Err(format!("unexpected reading.progress.update data {data:?}"))
                }
            }
            other => Err(format!(
                "unexpected reading.progress.update result {other:?}"
            )),
        }
    });

    record(
        &mut report,
        "reading-progress-update-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "chapterProgress",
                    json!({
                        "bookId": "1",
                        "chapterIndex": 2,
                        "chapterOffset": 128,
                        "chapterProgress": 1.5,
                        "stored": true
                    }),
                    "chapterProgress",
                ),
                (
                    "stored",
                    json!({
                        "bookId": "1",
                        "chapterIndex": 2,
                        "chapterOffset": 128,
                        "chapterProgress": 0.5,
                        "stored": false
                    }),
                    "stored",
                ),
                (
                    "unknown field",
                    json!({
                        "bookId": "1",
                        "chapterIndex": 2,
                        "chapterOffset": 128,
                        "chapterProgress": 0.5,
                        "stored": true,
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<ReadingProgressUpdateData>(data)
                    .err()
                    .ok_or_else(|| {
                        format!("expected reading.progress.update data rejection for {label}")
                    })?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected reading.progress.update data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

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

    // =======================================================================
    // RSS / sync / local-book verticals (V1 minimal)
    // =======================================================================

    record(&mut report, "valid-command-rss-parse", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_RSS_PARSE)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 601 => {
                let data = serde_json::from_value::<RssParseData>(data)
                    .map_err(|err| format!("rss.parse data contract parse failed: {err}"))?;
                if data.title == "Conformance Feed"
                    && data.entries.len() == 1
                    && data.entries[0].id == "entry-1"
                    && data.entries[0].title == "Entry One"
                {
                    Ok(())
                } else {
                    Err(format!("unexpected rss.parse data {data:?}"))
                }
            }
            other => Err(format!("unexpected rss.parse result {other:?}")),
        }
    });

    record(&mut report, "valid-command-rss-refresh", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_RSS_REFRESH)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 602 => {
                let data = serde_json::from_value::<RssRefreshData>(data)
                    .map_err(|err| format!("rss.refresh data contract parse failed: {err}"))?;
                if data.subscription_id == "conformance-sub"
                    && data.should_fetch
                    && data.reason == "forced"
                {
                    Ok(())
                } else {
                    Err(format!("unexpected rss.refresh data {data:?}"))
                }
            }
            other => Err(format!("unexpected rss.refresh result {other:?}")),
        }
    });

    record(&mut report, "valid-command-sync-merge", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_SYNC_MERGE)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 603 => {
                let data = serde_json::from_value::<SyncMergeData>(data)
                    .map_err(|err| format!("sync.merge data contract parse failed: {err}"))?;
                let snapshot_id = data.snapshot.get("snapshotId").and_then(|v| v.as_str());
                if snapshot_id == Some("merged-1") && data.conflicts.is_empty() {
                    Ok(())
                } else {
                    Err(format!("unexpected sync.merge data {data:?}"))
                }
            }
            other => Err(format!("unexpected sync.merge result {other:?}")),
        }
    });

    record(&mut report, "valid-command-sync-backup", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_SYNC_BACKUP)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 604 => {
                let data = serde_json::from_value::<SyncBackupData>(data)
                    .map_err(|err| format!("sync.backup data contract parse failed: {err}"))?;
                if data.plan.is_object() {
                    Ok(())
                } else {
                    Err(format!("unexpected sync.backup data {data:?}"))
                }
            }
            other => Err(format!("unexpected sync.backup result {other:?}")),
        }
    });

    record(&mut report, "valid-command-local-book-parse", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_LOCAL_BOOK_PARSE)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 605 => {
                let data = serde_json::from_value::<LocalBookParseData>(data)
                    .map_err(|err| format!("local_book.parse data contract parse failed: {err}"))?;
                if data.format == "txt"
                    && data.encoding == "utf8"
                    && data.char_len > 0
                    && data.chapter_count >= 1
                {
                    Ok(())
                } else {
                    Err(format!("unexpected local_book.parse data {data:?}"))
                }
            }
            other => Err(format!("unexpected local_book.parse result {other:?}")),
        }
    });

    record(&mut report, "valid-command-local-book-catalog", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_LOCAL_BOOK_CATALOG)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 606 => {
                let data = serde_json::from_value::<LocalBookCatalogData>(data).map_err(|err| {
                    format!("local_book.catalog data contract parse failed: {err}")
                })?;
                let books = data.catalog.get("books").and_then(|v| v.as_array());
                if let Some(books) = books {
                    if books.iter().any(|entry| {
                        entry.get("stableBookId").and_then(|v| v.as_str())
                            == Some("conformance-book-1")
                    }) {
                        Ok(())
                    } else {
                        Err(format!(
                            "local_book.catalog did not upsert conformance-book-1: {data:?}"
                        ))
                    }
                } else {
                    Err(format!("unexpected local_book.catalog data {data:?}"))
                }
            }
            other => Err(format!("unexpected local_book.catalog result {other:?}")),
        }
    });

    record(&mut report, "valid-command-bookshelf-list-empty", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOKSHELF_LIST)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 700 => {
                let data = serde_json::from_value::<BookshelfListData>(data)
                    .map_err(|err| format!("bookshelf.list data contract parse failed: {err}"))?;
                if data.books.is_empty() && data.total == 0 {
                    Ok(())
                } else {
                    Err(format!("expected empty shelf, got {data:?}"))
                }
            }
            other => Err(format!("unexpected bookshelf.list result {other:?}")),
        }
    });

    record(
        &mut report,
        "valid-command-bookshelf-list-with-preset",
        || {
            let (runtime, rx) = fresh_runtime();
            let entry: BookshelfEntry = serde_json::from_value(json!({
                "sourceId": "src-1",
                "bookId": "book-1",
                "title": "Conformance Book",
                "author": "Conformance Author",
                "addedAt": 1000,
                "sortIndex": 0
            }))
            .map_err(|err| format!("shelf entry construction failed: {err}"))?;
            runtime
                .remote_state()
                .storage()
                .add_to_shelf(entry)
                .map_err(|err| format!("add_to_shelf failed: {err}"))?;
            runtime
                .send_json(VALID_BOOKSHELF_LIST.as_bytes())
                .map_err(|err| format!("send_json failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 700 => {
                    let data =
                        serde_json::from_value::<BookshelfListData>(data).map_err(|err| {
                            format!("bookshelf.list data contract parse failed: {err}")
                        })?;
                    if data.books.len() == 1
                        && data.total == 1
                        && data.books[0].source_id == "src-1"
                        && data.books[0].book_id == "book-1"
                    {
                        Ok(())
                    } else {
                        Err(format!("unexpected bookshelf.list data {data:?}"))
                    }
                }
                other => Err(format!("unexpected bookshelf.list result {other:?}")),
            }
        },
    );

    record(&mut report, "valid-command-bookshelf-get-not-found", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_BOOKSHELF_GET)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 701 => {
                let data = serde_json::from_value::<BookshelfGetData>(data)
                    .map_err(|err| format!("bookshelf.get data contract parse failed: {err}"))?;
                if data.book.is_none() {
                    Ok(())
                } else {
                    Err(format!("expected null book, got {data:?}"))
                }
            }
            other => Err(format!("unexpected bookshelf.get result {other:?}")),
        }
    });

    record(&mut report, "valid-command-bookshelf-get-found", || {
        let (runtime, rx) = fresh_runtime();
        let entry: BookshelfEntry = serde_json::from_value(json!({
            "sourceId": "src-1",
            "bookId": "book-1",
            "title": "Conformance Book",
            "author": "Conformance Author",
            "addedAt": 1000,
            "sortIndex": 0
        }))
        .map_err(|err| format!("shelf entry construction failed: {err}"))?;
        runtime
            .remote_state()
            .storage()
            .add_to_shelf(entry)
            .map_err(|err| format!("add_to_shelf failed: {err}"))?;
        runtime
            .send_json(VALID_BOOKSHELF_GET.as_bytes())
            .map_err(|err| format!("send_json failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 701 => {
                let data = serde_json::from_value::<BookshelfGetData>(data)
                    .map_err(|err| format!("bookshelf.get data contract parse failed: {err}"))?;
                let book = data
                    .book
                    .ok_or_else(|| "expected non-null book".to_string())?;
                if book.source_id == "src-1" && book.book_id == "book-1" {
                    Ok(())
                } else {
                    Err(format!("unexpected bookshelf.get data {book:?}"))
                }
            }
            other => Err(format!("unexpected bookshelf.get result {other:?}")),
        }
    });

    record(&mut report, "valid-command-tts-slice", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_TTS_SLICE)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 420 => {
                let data = serde_json::from_value::<TtsSliceData>(data)
                    .map_err(|err| format!("tts.slice data contract parse failed: {err}"))?;
                if data.plan.strategy == TtsSlicingStrategy::Paragraph
                    && data.plan.slices.len() == 2
                    && data.plan.slices[0].text == "第一段内容。"
                    && data.plan.slices[1].text == "第二段内容。"
                    && data.plan.source_char_count == 14
                    && data.plan.chapter.chapter_index == 0
                    && data.plan.chapter.chapter_title == "第一章"
                {
                    Ok(())
                } else {
                    Err(format!("unexpected tts.slice data {data:?}"))
                }
            }
            other => Err(format!("unexpected tts.slice result {other:?}")),
        }
    });

    record(&mut report, "valid-command-tts-queue-status", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_TTS_QUEUE_STATUS)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 421 => {
                let data = serde_json::from_value::<TtsQueueStatusData>(data)
                    .map_err(|err| format!("tts.queue.status data contract parse failed: {err}"))?;
                if data.snapshot.state == TtsQueueState::Idle
                    && data.snapshot.total_slices == 0
                    && data.snapshot.completed_slices == 0
                    && data.snapshot.current_slice_index.is_none()
                    && data.snapshot.chapter.chapter_index == 2
                {
                    Ok(())
                } else {
                    Err(format!("unexpected tts.queue.status data {data:?}"))
                }
            }
            other => Err(format!("unexpected tts.queue.status result {other:?}")),
        }
    });

    record(&mut report, "valid-command-tts-chapter-plan", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_TTS_CHAPTER_PLAN)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 422 => {
                let data = serde_json::from_value::<TtsChapterPlanData>(data)
                    .map_err(|err| format!("tts.chapter.plan data contract parse failed: {err}"))?;
                if data.transition.next.is_none()
                    && data.transition.current.chapter_index == 2
                    && data.transition.drain_behavior
                        == reader_contract::TtsQueueDrainBehavior::AdvanceToNext
                {
                    Ok(())
                } else {
                    Err(format!("unexpected tts.chapter.plan data {data:?}"))
                }
            }
            other => Err(format!("unexpected tts.chapter.plan result {other:?}")),
        }
    });

    // --- TTS queue control commands (Gap F closure) ----------------------
    //
    // Exercises the Core-owned queue state machine:
    //   Idle --play--> Playing --pause--> Paused --resume--> Playing --stop--> Stopped
    //   Playing --next--> (cursor advance) --prev--> (cursor retreat)
    // Mirrors Legado BaseReadAloudService control actions. Pure logic; no
    // audio. The host observes snapshots via the protocol and vocalizes.

    record(&mut report, "valid-command-tts-queue-play", || {
        let (_runtime, rx) = send_to_fresh_runtime(VALID_TTS_QUEUE_PLAY)?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 500 => {
                let data = serde_json::from_value::<TtsQueuePlayData>(data)
                    .map_err(|err| format!("tts.queue.play data contract parse failed: {err}"))?;
                if data.snapshot.state == TtsQueueState::Playing
                    && data.snapshot.current_slice_index == Some(0)
                    && data.snapshot.total_slices == 2
                    && data.snapshot.completed_slices == 0
                    && data.snapshot.chapter.chapter_index == 0
                {
                    Ok(())
                } else {
                    Err(format!("unexpected tts.queue.play data {data:?}"))
                }
            }
            other => Err(format!("unexpected tts.queue.play result {other:?}")),
        }
    });

    record(
        &mut report,
        "valid-command-tts-queue-lifecycle-play-pause-resume-stop",
        || {
            // Multi-step on a single runtime: play → pause → resume → stop.
            let (runtime, rx) = send_to_fresh_runtime(VALID_TTS_QUEUE_PLAY)?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 500 => {
                    let data = serde_json::from_value::<TtsQueuePlayData>(data)
                        .map_err(|err| format!("play data parse failed: {err}"))?;
                    if data.snapshot.state != TtsQueueState::Playing {
                        return Err(format!("expected Playing, got {:?}", data.snapshot.state));
                    }
                }
                other => return Err(format!("unexpected play result {other:?}")),
            }

            runtime
                .send_json(VALID_TTS_QUEUE_PAUSE.as_bytes())
                .map_err(|err| format!("pause send_json failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 501 => {
                    let data = serde_json::from_value::<TtsQueuePauseData>(data)
                        .map_err(|err| format!("pause data parse failed: {err}"))?;
                    if data.snapshot.state != TtsQueueState::Paused {
                        return Err(format!("expected Paused, got {:?}", data.snapshot.state));
                    }
                }
                other => return Err(format!("unexpected pause result {other:?}")),
            }

            runtime
                .send_json(VALID_TTS_QUEUE_RESUME.as_bytes())
                .map_err(|err| format!("resume send_json failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 502 => {
                    let data = serde_json::from_value::<TtsQueueResumeData>(data)
                        .map_err(|err| format!("resume data parse failed: {err}"))?;
                    if data.snapshot.state != TtsQueueState::Playing {
                        return Err(format!("expected Playing, got {:?}", data.snapshot.state));
                    }
                }
                other => return Err(format!("unexpected resume result {other:?}")),
            }

            runtime
                .send_json(VALID_TTS_QUEUE_STOP.as_bytes())
                .map_err(|err| format!("stop send_json failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 503 => {
                    let data = serde_json::from_value::<TtsQueueStopData>(data)
                        .map_err(|err| format!("stop data parse failed: {err}"))?;
                    if data.snapshot.state != TtsQueueState::Stopped {
                        return Err(format!("expected Stopped, got {:?}", data.snapshot.state));
                    }
                }
                other => return Err(format!("unexpected stop result {other:?}")),
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "valid-command-tts-queue-next-prev-cursor",
        || {
            // Multi-step on a single runtime: play → next → prev.
            let (runtime, rx) = send_to_fresh_runtime(VALID_TTS_QUEUE_PLAY)?;
            let _ = recv_event(&rx)?; // play result

            runtime
                .send_json(VALID_TTS_QUEUE_NEXT.as_bytes())
                .map_err(|err| format!("next send_json failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 504 => {
                    let data = serde_json::from_value::<TtsQueueNextData>(data)
                        .map_err(|err| format!("next data parse failed: {err}"))?;
                    if data.snapshot.current_slice_index != Some(1)
                        || data.snapshot.completed_slices != 1
                    {
                        return Err(format!("next did not advance cursor: {data:?}"));
                    }
                }
                other => return Err(format!("unexpected next result {other:?}")),
            }

            runtime
                .send_json(VALID_TTS_QUEUE_PREV.as_bytes())
                .map_err(|err| format!("prev send_json failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 505 => {
                    let data = serde_json::from_value::<TtsQueuePrevData>(data)
                        .map_err(|err| format!("prev data parse failed: {err}"))?;
                    if data.snapshot.current_slice_index != Some(0) {
                        return Err(format!("prev did not retreat cursor: {data:?}"));
                    }
                }
                other => return Err(format!("unexpected prev result {other:?}")),
            }
            Ok(())
        },
    );

    record(&mut report, "invalid-tts-queue-pause-no-queue", || {
        // pause without a loaded queue → InvalidParams (state machine error).
        let (_runtime, rx) = send_to_fresh_runtime(VALID_TTS_QUEUE_PAUSE)?;
        expect_event_error(&rx, 501, ErrorCode::InvalidParams)
    });

    for (name, json, request_id) in [
        (
            "rss-parse-rejects-unknown-params",
            INVALID_RSS_PARSE_UNKNOWN_FIELD,
            607,
        ),
        (
            "rss-parse-rejects-xml-blank",
            INVALID_RSS_PARSE_XML_BLANK,
            608,
        ),
        (
            "rss-refresh-rejects-unknown-params",
            INVALID_RSS_REFRESH_UNKNOWN_FIELD,
            609,
        ),
        (
            "rss-refresh-rejects-subscription-id-blank",
            INVALID_RSS_REFRESH_SUBSCRIPTION_ID_BLANK,
            610,
        ),
        (
            "sync-merge-rejects-unknown-params",
            INVALID_SYNC_MERGE_UNKNOWN_FIELD,
            611,
        ),
        (
            "sync-merge-rejects-local-not-object",
            INVALID_SYNC_MERGE_LOCAL_NOT_OBJECT,
            612,
        ),
        (
            "sync-backup-rejects-unknown-params",
            INVALID_SYNC_BACKUP_UNKNOWN_FIELD,
            613,
        ),
        (
            "sync-backup-rejects-package-not-object",
            INVALID_SYNC_BACKUP_PACKAGE_NOT_OBJECT,
            614,
        ),
        (
            "local-book-parse-rejects-unknown-params",
            INVALID_LOCAL_BOOK_PARSE_UNKNOWN_FIELD,
            615,
        ),
        (
            "local-book-parse-rejects-text-blank",
            INVALID_LOCAL_BOOK_PARSE_TEXT_BLANK,
            616,
        ),
        (
            "local-book-catalog-rejects-unknown-params",
            INVALID_LOCAL_BOOK_CATALOG_UNKNOWN_FIELD,
            617,
        ),
        (
            "local-book-catalog-rejects-entry-not-object",
            INVALID_LOCAL_BOOK_CATALOG_ENTRY_NOT_OBJECT,
            618,
        ),
        (
            "bookshelf-list-rejects-unknown-params",
            INVALID_BOOKSHELF_LIST_UNKNOWN_FIELD,
            702,
        ),
        (
            "bookshelf-list-rejects-blank-source-id",
            INVALID_BOOKSHELF_LIST_BLANK_SOURCE_ID,
            703,
        ),
        (
            "bookshelf-list-rejects-invalid-sort-by",
            INVALID_BOOKSHELF_LIST_INVALID_SORT_BY,
            704,
        ),
        (
            "bookshelf-get-rejects-missing-book-id",
            INVALID_BOOKSHELF_GET_MISSING_BOOK_ID,
            705,
        ),
        (
            "bookshelf-get-rejects-blank-source-id",
            INVALID_BOOKSHELF_GET_BLANK_SOURCE_ID,
            706,
        ),
        (
            "tts-slice-rejects-content-empty",
            INVALID_TTS_SLICE_CONTENT_EMPTY,
            423,
        ),
        (
            "tts-slice-rejects-unknown-params",
            INVALID_TTS_SLICE_UNKNOWN_FIELD,
            424,
        ),
        (
            "tts-queue-status-rejects-unknown-params",
            INVALID_TTS_QUEUE_STATUS_UNKNOWN_FIELD,
            425,
        ),
        (
            "tts-chapter-plan-rejects-unknown-params",
            INVALID_TTS_CHAPTER_PLAN_UNKNOWN_FIELD,
            426,
        ),
        (
            "tts-queue-play-rejects-unknown-params",
            INVALID_TTS_QUEUE_PLAY_UNKNOWN_FIELD,
            510,
        ),
        (
            "tts-queue-pause-rejects-unknown-params",
            INVALID_TTS_QUEUE_PAUSE_UNKNOWN_FIELD,
            511,
        ),
        (
            "tts-queue-resume-rejects-unknown-params",
            INVALID_TTS_QUEUE_RESUME_UNKNOWN_FIELD,
            512,
        ),
        (
            "tts-queue-stop-rejects-unknown-params",
            INVALID_TTS_QUEUE_STOP_UNKNOWN_FIELD,
            513,
        ),
        (
            "tts-queue-next-rejects-unknown-params",
            INVALID_TTS_QUEUE_NEXT_UNKNOWN_FIELD,
            514,
        ),
        (
            "tts-queue-prev-rejects-unknown-params",
            INVALID_TTS_QUEUE_PREV_UNKNOWN_FIELD,
            515,
        ),
    ] {
        record(&mut report, name, || {
            let (_runtime, rx) = send_to_fresh_runtime(json)?;
            expect_event_error(&rx, request_id, ErrorCode::InvalidParams)
        });
    }

    record(
        &mut report,
        "rss-parse-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                ("missing title", json!({ "entries": [] }), "title"),
                ("missing entries", json!({ "title": "x" }), "entries"),
                (
                    "unknown field",
                    json!({ "title": "x", "entries": [], "extra": true }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<RssParseData>(data)
                    .err()
                    .ok_or_else(|| format!("expected rss.parse data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected rss.parse data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "rss-refresh-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "missing shouldFetch",
                    json!({
                        "subscriptionId": "s",
                        "reason": "forced",
                        "evaluatedAt": 1
                    }),
                    "shouldFetch",
                ),
                (
                    "unknown field",
                    json!({
                        "subscriptionId": "s",
                        "shouldFetch": true,
                        "reason": "forced",
                        "evaluatedAt": 1,
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<RssRefreshData>(data)
                    .err()
                    .ok_or_else(|| format!("expected rss.refresh data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected rss.refresh data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "sync-merge-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                ("missing snapshot", json!({ "conflicts": [] }), "snapshot"),
                (
                    "unknown field",
                    json!({ "snapshot": {}, "conflicts": [], "extra": true }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<SyncMergeData>(data)
                    .err()
                    .ok_or_else(|| format!("expected sync.merge data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected sync.merge data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "sync-backup-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                ("missing plan", json!({}), "plan"),
                (
                    "unknown field",
                    json!({ "plan": {}, "extra": true }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<SyncBackupData>(data)
                    .err()
                    .ok_or_else(|| format!("expected sync.backup data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected sync.backup data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "local-book-parse-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "missing format",
                    json!({
                        "book": {},
                        "encoding": "utf8",
                        "byteLen": 0,
                        "charLen": 0,
                        "chapterCount": 0
                    }),
                    "format",
                ),
                (
                    "unknown field",
                    json!({
                        "book": {},
                        "format": "txt",
                        "encoding": "utf8",
                        "byteLen": 0,
                        "charLen": 0,
                        "chapterCount": 0,
                        "extra": true
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<LocalBookParseData>(data)
                    .err()
                    .ok_or_else(|| {
                        format!("expected local_book.parse data rejection for {label}")
                    })?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected local_book.parse data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "local-book-catalog-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                ("missing catalog", json!({}), "catalog"),
                (
                    "unknown field",
                    json!({ "catalog": {}, "extra": true }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<LocalBookCatalogData>(data)
                    .err()
                    .ok_or_else(|| {
                        format!("expected local_book.catalog data rejection for {label}")
                    })?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected local_book.catalog data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
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

    record(
        &mut report,
        "duplicate-active-request-id-rejected-synchronously",
        || {
            let fixture = serde_json::from_str::<Value>(DUPLICATE_ACTIVE_REQUEST_ID)
                .map_err(|err| format!("duplicate requestId fixture parse failed: {err}"))?;
            let pending_json = fixture["pendingCommand"].to_string();
            let duplicate = &fixture["duplicateCommand"];
            let duplicate_json = duplicate.to_string();
            let request_id = duplicate["requestId"]
                .as_u64()
                .ok_or_else(|| "duplicate fixture missing requestId".to_string())?;

            let (runtime, rx) = fresh_runtime();
            runtime
                .send_json(pending_json.as_bytes())
                .map_err(|err| format!("pending command send failed: {err:?}"))?;
            expect_host_request(&rx)?;

            let err = runtime
                .send_json(duplicate_json.as_bytes())
                .err()
                .ok_or_else(|| "expected duplicate active requestId error".to_string())?;
            if err.code != ErrorCode::InvalidMessage {
                return Err(format!(
                    "expected duplicate active requestId INVALID_MESSAGE, got {err:?}"
                ));
            }
            if !err.message.contains("duplicate active requestId") {
                return Err(format!(
                    "unexpected duplicate active requestId message: {}",
                    err.message
                ));
            }
            if err.details["requestId"] != json!(request_id) {
                return Err(format!(
                    "unexpected duplicate active requestId details: {:?}",
                    err.details
                ));
            }
            expect_no_event(&rx)?;

            runtime.cancel(request_id);
            expect_event_error(&rx, request_id, ErrorCode::Cancelled)
        },
    );

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
                && *capability == HostCapability::HostSmokeEcho
                && event_json["capability"] == HostCapability::HostSmokeEcho.as_str()
                && params["message"] == "conformance host request"
                && event_json["params"].is_object() =>
            {
                Ok(())
            }
            other => Err(format!("unexpected host.request event {other:?}")),
        }
    });

    record(&mut report, "event-json-rejects-unknown-fields", || {
        for event in [
            json!({
                "protocolVersion": 1,
                "requestId": 301,
                "type": "result",
                "data": {},
                "extra": true
            }),
            json!({
                "protocolVersion": 1,
                "requestId": 301,
                "type": "error",
                "error": {
                    "code": "INTERNAL",
                    "message": "failed",
                    "retryable": true
                },
                "extra": true
            }),
            json!({
                "protocolVersion": 1,
                "requestId": 301,
                "type": "host.request",
                "operationId": 1,
                "capability": "host.smoke.echo",
                "params": {},
                "extra": true
            }),
        ] {
            let err = serde_json::from_value::<Event>(event)
                .err()
                .ok_or_else(|| "expected event unknown-field rejection".to_string())?;
            if !err.to_string().contains("unknown field") {
                return Err(format!("unexpected event unknown-field error: {err}"));
            }
        }
        Ok(())
    });

    record(
        &mut report,
        "event-json-rejects-unsupported-protocol-version",
        || {
            for event in [
                json!({
                    "protocolVersion": 2,
                    "requestId": 301,
                    "type": "result",
                    "data": {}
                }),
                json!({
                    "protocolVersion": 2,
                    "requestId": 301,
                    "type": "error",
                    "error": {
                        "code": "INTERNAL",
                        "message": "failed",
                        "retryable": true
                    }
                }),
                json!({
                    "protocolVersion": 2,
                    "requestId": 301,
                    "type": "host.request",
                    "operationId": 1,
                    "capability": "host.smoke.echo",
                    "params": {}
                }),
            ] {
                let err = serde_json::from_value::<Event>(event)
                    .err()
                    .ok_or_else(|| "expected event protocolVersion rejection".to_string())?;
                if !err.to_string().contains("protocolVersion") {
                    return Err(format!("unexpected event protocolVersion error: {err}"));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "event-json-rejects-zero-non-error-request-id",
        || {
            for event in [
                json!({
                    "protocolVersion": 1,
                    "requestId": 0,
                    "type": "result",
                    "data": {}
                }),
                json!({
                    "protocolVersion": 1,
                    "requestId": 0,
                    "type": "host.request",
                    "operationId": 1,
                    "capability": "host.smoke.echo",
                    "params": {}
                }),
            ] {
                let err = serde_json::from_value::<Event>(event)
                    .err()
                    .ok_or_else(|| "expected event requestId rejection".to_string())?;
                if !err.to_string().contains("requestId") {
                    return Err(format!("unexpected event requestId error: {err}"));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "event-json-allows-process-level-error-request-id-zero",
        || {
            let event = serde_json::from_value::<Event>(json!({
                "protocolVersion": 1,
                "requestId": 0,
                "type": "error",
                "error": {
                    "code": "INVALID_MESSAGE",
                    "message": "failed before command correlation",
                    "retryable": false
                }
            }))
            .map_err(|err| format!("process-level error event parse failed: {err}"))?;

            match event {
                Event::Error { request_id, .. } if request_id == 0 => Ok(()),
                other => Err(format!("expected process-level error event, got {other:?}")),
            }
        },
    );

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

    record(&mut report, "host-request-unsupported-capability", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_REQUEST_UNSUPPORTED_CAPABILITY)?;
        expect_event_error(&rx, 421, ErrorCode::InvalidParams)
    });

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

    record(&mut report, "host-webview-request-event-shape", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_WEBVIEW_REQUEST)?;
        match recv_event(&rx)? {
            Event::HostRequest {
                request_id,
                operation_id,
                capability,
                params,
                ..
            } if request_id == 431
                && operation_id == 1
                && capability == HostCapability::WebViewEvaluateJavaScript =>
            {
                let request =
                    serde_json::from_value::<HostWebViewEvaluateJavaScriptRequest>(params)
                        .map_err(|err| format!("webview request DTO parse failed: {err}"))?;
                request
                    .validate()
                    .map_err(|err| format!("webview request validation failed: {err:?}"))?;
                if request
                    .document
                    .body
                    .as_deref()
                    .is_some_and(|body| body.contains("Dune"))
                    && request.java_script.contains("querySelector")
                    && request.timeout_millis == Some(3000)
                {
                    Ok(())
                } else {
                    Err(format!("unexpected webview request {request:?}"))
                }
            }
            other => Err(format!("unexpected webview host.request event {other:?}")),
        }
    });

    record(
        &mut report,
        "host-webview-request-rejects-blank-javascript",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_WEBVIEW_REQUEST_BLANK_JAVASCRIPT)?;
            expect_event_error(&rx, 432, ErrorCode::InvalidParams)
        },
    );

    record(&mut report, "host-webview-complete-routes-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_WEBVIEW_REQUEST)?;
        match recv_event(&rx)? {
            Event::HostRequest {
                capability, params, ..
            } if capability == HostCapability::WebViewEvaluateJavaScript
                && params["document"]["kind"] == "html" => {}
            other => return Err(format!("expected webview host.request, got {other:?}")),
        }
        runtime
            .send_json(HOST_WEBVIEW_COMPLETE.as_bytes())
            .map_err(|err| format!("webview host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 431 => {
                let response =
                    serde_json::from_value::<HostWebViewEvaluateJavaScriptResponse>(data.clone())
                        .map_err(|err| format!("webview response DTO parse failed: {err}"))?;
                response
                    .validate()
                    .map_err(|err| format!("webview response validation failed: {err:?}"))?;
                if response.value == json!("Dune")
                    && response.final_url.as_deref() == Some("https://books.example.test/detail")
                {
                    Ok(())
                } else {
                    Err(format!("unexpected webview response {response:?}"))
                }
            }
            other => Err(format!("unexpected webview completion result {other:?}")),
        }
    });

    record(
        &mut report,
        "host-webview-complete-rejects-invalid-result",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_WEBVIEW_REQUEST)?;
            match recv_event(&rx)? {
                Event::HostRequest {
                    capability, params, ..
                } if capability == HostCapability::WebViewEvaluateJavaScript
                    && params["document"]["kind"] == "html" => {}
                other => return Err(format!("expected webview host.request, got {other:?}")),
            }
            runtime
                .send_json(HOST_WEBVIEW_COMPLETE_BLANK_FINAL_URL.as_bytes())
                .map_err(|err| format!("webview invalid host.complete send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 431
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("finalUrl") =>
                {
                    Ok(())
                }
                other => Err(format!("unexpected webview completion error {other:?}")),
            }
        },
    );

    record(&mut report, "host-file-read-routes-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_FILE_READ_REQUEST)?;
        expect_capability_host_request(
            &rx,
            435,
            HostCapability::FileRead,
            json!({
                "path": "core-cache/books/basic.json",
                "encoding": "utf-8",
                "byteOffset": 0,
                "maxBytes": 4096
            }),
        )?;
        runtime
            .send_json(HOST_FILE_READ_COMPLETE.as_bytes())
            .map_err(|err| format!("file.read host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 435
                && data["content"] == "cached body"
                && data["encoding"] == "utf-8"
                && data["byteLength"] == 11 =>
            {
                Ok(())
            }
            other => Err(format!("unexpected file.read completion result {other:?}")),
        }
    });

    record(&mut report, "host-file-write-routes-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_FILE_WRITE_REQUEST)?;
        expect_capability_host_request(
            &rx,
            436,
            HostCapability::FileWrite,
            json!({
                "path": "core-cache/books/basic.json",
                "content": "{\"books\":[]}",
                "encoding": "utf-8",
                "createDirectories": true,
                "append": false
            }),
        )?;
        runtime
            .send_json(HOST_FILE_WRITE_COMPLETE.as_bytes())
            .map_err(|err| format!("file.write host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 436 && data["written"] == true && data["byteLength"] == 11 => Ok(()),
            other => Err(format!("unexpected file.write completion result {other:?}")),
        }
    });

    record(&mut report, "host-cache-get-routes-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_CACHE_GET_REQUEST)?;
        expect_capability_host_request(
            &rx,
            437,
            HostCapability::CacheGet,
            json!({
                "namespace": "remote.response",
                "key": "search/basic"
            }),
        )?;
        runtime
            .send_json(HOST_CACHE_GET_COMPLETE_HIT.as_bytes())
            .map_err(|err| format!("cache.get host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 437
                && data["hit"] == true
                && data["value"] == "{\"books\":[]}"
                && data["expiresAt"] == "2026-06-26T00:00:00Z" =>
            {
                Ok(())
            }
            other => Err(format!("unexpected cache.get completion result {other:?}")),
        }
    });

    record(&mut report, "host-cache-put-routes-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_CACHE_PUT_REQUEST)?;
        expect_capability_host_request(
            &rx,
            438,
            HostCapability::CachePut,
            json!({
                "namespace": "remote.response",
                "key": "search/basic",
                "value": "{\"books\":[]}",
                "ttlMillis": 60000
            }),
        )?;
        runtime
            .send_json(HOST_CACHE_PUT_COMPLETE.as_bytes())
            .map_err(|err| format!("cache.put host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Result {
                request_id, data, ..
            } if request_id == 438
                && data["stored"] == true
                && data["expiresAt"] == "2026-06-26T00:00:00Z" =>
            {
                Ok(())
            }
            other => Err(format!("unexpected cache.put completion result {other:?}")),
        }
    });

    record(&mut report, "host-file-read-rejects-blank-path", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_FILE_READ_REQUEST_BLANK_PATH)?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 439
                && error.code == ErrorCode::InvalidParams
                && error.message.contains("file.read path") =>
            {
                Ok(())
            }
            other => Err(format!("expected file.read path rejection, got {other:?}")),
        }
    });

    record(&mut report, "host-cache-put-rejects-missing-value", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_CACHE_PUT_REQUEST_MISSING_VALUE)?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 440
                && error.code == ErrorCode::InvalidParams
                && error.message.contains("value") =>
            {
                Ok(())
            }
            other => Err(format!(
                "expected cache.put missing value rejection, got {other:?}"
            )),
        }
    });

    record(&mut report, "host-file-read-rejects-invalid-result", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_FILE_READ_REQUEST)?;
        expect_capability_host_request(
            &rx,
            435,
            HostCapability::FileRead,
            json!({
                "path": "core-cache/books/basic.json",
                "encoding": "utf-8",
                "byteOffset": 0,
                "maxBytes": 4096
            }),
        )?;
        runtime
            .send_json(HOST_FILE_READ_COMPLETE_MISSING_CONTENT.as_bytes())
            .map_err(|err| format!("file.read invalid host.complete send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 435
                && error.code == ErrorCode::InvalidParams
                && error.message.contains("content") =>
            {
                Ok(())
            }
            other => Err(format!(
                "expected file.read result rejection, got {other:?}"
            )),
        }
    });

    record(
        &mut report,
        "host-file-write-rejects-invalid-result",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_FILE_WRITE_REQUEST)?;
            expect_capability_host_request(
                &rx,
                436,
                HostCapability::FileWrite,
                json!({
                    "path": "core-cache/books/basic.json",
                    "content": "{\"books\":[]}",
                    "encoding": "utf-8",
                    "createDirectories": true,
                    "append": false
                }),
            )?;
            runtime
                .send_json(HOST_FILE_WRITE_COMPLETE_NOT_WRITTEN.as_bytes())
                .map_err(|err| format!("file.write invalid host.complete send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 436
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("written") =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "expected file.write result rejection, got {other:?}"
                )),
            }
        },
    );

    record(
        &mut report,
        "host-cache-get-rejects-invalid-hit-result",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_CACHE_GET_REQUEST)?;
            expect_capability_host_request(
                &rx,
                437,
                HostCapability::CacheGet,
                json!({
                    "namespace": "remote.response",
                    "key": "search/basic"
                }),
            )?;
            runtime
                .send_json(HOST_CACHE_GET_COMPLETE_INVALID_HIT.as_bytes())
                .map_err(|err| format!("cache.get invalid host.complete send failed: {err:?}"))?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 437
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("value") =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "expected cache.get hit result rejection, got {other:?}"
                )),
            }
        },
    );

    record(&mut report, "host-cookie-get-routes-result", || {
        expect_capability_roundtrip(
            HOST_COOKIE_GET_REQUEST,
            HOST_COOKIE_GET_COMPLETE,
            448,
            HostCapability::CookieGet,
            json!({
                "url": "https://books.example.test/search",
                "name": "sid",
                "sessionId": "core-session-main"
            }),
            json!({
                "cookies": [
                    {
                        "name": "sid",
                        "value": "new",
                        "domain": "books.example.test",
                        "path": "/",
                        "httpOnly": true,
                        "secure": true,
                        "sameSite": "Lax"
                    }
                ]
            }),
        )
    });

    record(&mut report, "host-cookie-set-routes-result", || {
        expect_capability_roundtrip(
            HOST_COOKIE_SET_REQUEST,
            HOST_COOKIE_SET_COMPLETE,
            450,
            HostCapability::CookieSet,
            json!({
                "url": "https://books.example.test/search",
                "sessionId": "core-session-main",
                "cookie": {
                    "name": "sid",
                    "value": "new",
                    "domain": "books.example.test",
                    "path": "/",
                    "httpOnly": true,
                    "secure": true,
                    "sameSite": "Lax"
                }
            }),
            json!({ "stored": true }),
        )
    });

    record(&mut report, "host-log-emit-routes-result", || {
        expect_capability_roundtrip(
            HOST_LOG_EMIT_REQUEST,
            HOST_LOG_EMIT_COMPLETE,
            452,
            HostCapability::LogEmit,
            json!({
                "level": "info",
                "message": "host capability smoke",
                "target": "reader-core",
                "fields": { "operation": "log.emit" }
            }),
            json!({ "emitted": true }),
        )
    });

    record(&mut report, "host-time-now-routes-result", || {
        expect_capability_roundtrip(
            HOST_TIME_NOW_REQUEST,
            HOST_TIME_NOW_COMPLETE,
            454,
            HostCapability::TimeNow,
            json!({
                "clock": "system",
                "timezone": "UTC"
            }),
            json!({
                "unixMillis": 1782432000000_u64,
                "iso8601": "2026-06-26T00:00:00Z",
                "timezone": "UTC"
            }),
        )
    });

    record(&mut report, "host-system-info-routes-result", || {
        expect_capability_roundtrip(
            HOST_SYSTEM_INFO_REQUEST,
            HOST_SYSTEM_INFO_COMPLETE,
            456,
            HostCapability::SystemInfo,
            json!({
                "keys": ["os", "locale", "network"]
            }),
            json!({
                "info": {
                    "os": "test",
                    "locale": "en-US",
                    "network": "fixture"
                }
            }),
        )
    });

    record(&mut report, "host-persistence-get-routes-result", || {
        expect_capability_roundtrip(
            HOST_PERSISTENCE_GET_REQUEST,
            HOST_PERSISTENCE_GET_COMPLETE,
            458,
            HostCapability::PersistenceGet,
            json!({
                "namespace": "reader.session",
                "key": "last-source"
            }),
            json!({
                "found": true,
                "value": "basic-src",
                "revision": "rev-1"
            }),
        )
    });

    record(&mut report, "host-persistence-put-routes-result", || {
        expect_capability_roundtrip(
            HOST_PERSISTENCE_PUT_REQUEST,
            HOST_PERSISTENCE_PUT_COMPLETE,
            460,
            HostCapability::PersistencePut,
            json!({
                "namespace": "reader.session",
                "key": "last-source",
                "value": "basic-src",
                "expectedRevision": "rev-1"
            }),
            json!({
                "stored": true,
                "revision": "rev-2"
            }),
        )
    });

    record(&mut report, "host-cookie-get-rejects-missing-scope", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_COOKIE_GET_REQUEST_MISSING_SCOPE)?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 462
                && error.code == ErrorCode::InvalidParams
                && error.message.contains("cookie.get requires") =>
            {
                Ok(())
            }
            other => Err(format!(
                "expected cookie.get missing scope rejection, got {other:?}"
            )),
        }
    });

    record(&mut report, "host-log-emit-rejects-blank-message", || {
        let (_runtime, rx) = send_to_fresh_runtime(HOST_LOG_EMIT_REQUEST_BLANK_MESSAGE)?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 463
                && error.code == ErrorCode::InvalidParams
                && error.message.contains("log.emit message") =>
            {
                Ok(())
            }
            other => Err(format!(
                "expected log.emit blank message rejection, got {other:?}"
            )),
        }
    });

    record(
        &mut report,
        "host-persistence-get-rejects-invalid-found-result",
        || {
            let (runtime, rx) = send_to_fresh_runtime(HOST_PERSISTENCE_GET_REQUEST)?;
            expect_capability_host_request(
                &rx,
                458,
                HostCapability::PersistenceGet,
                json!({
                    "namespace": "reader.session",
                    "key": "last-source"
                }),
            )?;
            runtime
                .send_json(HOST_PERSISTENCE_GET_COMPLETE_INVALID_FOUND.as_bytes())
                .map_err(|err| {
                    format!("persistence.get invalid host.complete send failed: {err:?}")
                })?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 458
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("value") =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "expected persistence.get found result rejection, got {other:?}"
                )),
            }
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
            } if request_id == 301
                && error.code == ErrorCode::Internal
                && error.retryable
                && error.details["host"]["operationId"] == 1
                && error.details["host"]["requestId"] == 301
                && error.details["host"]["capability"] == "host.smoke.echo" =>
            {
                Ok(())
            }
            other => Err(format!("unexpected host.error event {other:?}")),
        }
    });

    record(&mut report, "host-error-routes-diagnostics", || {
        let (runtime, rx) = send_to_fresh_runtime(HOST_REQUEST)?;
        expect_host_request(&rx)?;
        runtime
            .send_json(HOST_ERROR_DIAGNOSTICS.as_bytes())
            .map_err(|err| format!("host.error diagnostics send failed: {err:?}"))?;
        match recv_event(&rx)? {
            Event::Error {
                request_id, error, ..
            } if request_id == 301
                && error.code == ErrorCode::Internal
                && error.details["host"]["operationId"] == 1
                && error.details["host"]["capability"] == "host.smoke.echo"
                && error.details["host"]["diagnostics"]["code"] == "TIMEOUT"
                && error.details["host"]["diagnostics"]["phase"] == "transport"
                && error.details["host"]["diagnostics"]["details"]["timeoutMillis"] == 30000 =>
            {
                Ok(())
            }
            other => Err(format!("unexpected host.error diagnostics event {other:?}")),
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
        "host-error-rejects-unknown-error-fields",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_CORE_ERROR_UNKNOWN_FIELD)?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 424
                    && error.code == ErrorCode::InvalidParams
                    && error.details["source"]
                        .as_str()
                        .is_some_and(|source| source.contains("unknown field")) =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "unexpected host.error unknown CoreError field rejection {other:?}"
                )),
            }
        },
    );

    record(
        &mut report,
        "host-error-rejects-non-object-diagnostic-details",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_DIAGNOSTICS_DETAILS_NOT_OBJECT)?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 426
                    && error.code == ErrorCode::InvalidParams
                    && error.details["source"]
                        .as_str()
                        .is_some_and(|source| source.contains("diagnostics.details")) =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "unexpected host.error diagnostics details rejection {other:?}"
                )),
            }
        },
    );

    record(
        &mut report,
        "host-error-rejects-unknown-diagnostics-fields",
        || {
            let (_runtime, rx) = send_to_fresh_runtime(HOST_ERROR_DIAGNOSTICS_UNKNOWN_FIELD)?;
            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 427
                    && error.code == ErrorCode::InvalidParams
                    && error.details["source"]
                        .as_str()
                        .is_some_and(|source| source.contains("unknown field")) =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "unexpected host.error diagnostics unknown field rejection {other:?}"
                )),
            }
        },
    );

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
                .send_json(HOST_HTTP_COMPLETE_SESSION_METADATA.as_bytes())
                .map_err(|err| format!("http host.complete send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Result {
                    request_id, data, ..
                } if request_id == 501 => {
                    let data = serde_json::from_value::<BookSearchData>(data)
                        .map_err(|err| format!("book.search http data parse failed: {err}"))?;
                    let http = data
                        .http
                        .ok_or_else(|| "missing book.search http diagnostics".to_string())?;
                    if data.books.is_empty()
                        && http.status == Some(200)
                        && http.headers.as_ref().is_some_and(|headers| {
                            headers["content-type"] == "application/json; charset=gbk"
                                && headers["set-cookie"][0] == "sid=new; Path=/; HttpOnly"
                        })
                        && http.final_url.as_deref()
                            == Some("https://books.example.test/search?q=empty")
                        && http.charset_hint.as_deref() == Some("gbk")
                        && http
                            .session
                            .as_ref()
                            .is_some_and(|session| session.id == "core-session-main")
                        && http.redirects.as_ref().is_some_and(|redirects| {
                            redirects[0].status == 302
                                && redirects[0].from_url == "https://books.example.test/search"
                                && redirects[0].to_url
                                    == "https://books.example.test/search?q=empty"
                        })
                        && http.cookies.as_ref().is_some_and(|cookies| {
                            cookies[0].name == "sid"
                                && cookies[0].domain.as_deref() == Some("books.example.test")
                                && cookies[0].path.as_deref() == Some("/")
                                && cookies[0].http_only == Some(true)
                                && cookies[0].secure == Some(true)
                                && cookies[0].same_site.as_deref() == Some("Lax")
                        })
                    {
                        Ok(())
                    } else {
                        Err(format!(
                            "unexpected book.search http completion data: books={:?} http={http:?}",
                            data.books
                        ))
                    }
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
        "remote-http-complete-rejects-invalid-redirect",
        || {
            let (runtime, rx) = fresh_runtime();
            runtime
                .send(remote_http_search_command(509))
                .map_err(|err| format!("remote http command send failed: {err:?}"))?;
            expect_http_host_request(&rx, 509)?;
            runtime
                .send_json(HOST_HTTP_COMPLETE_INVALID_REDIRECT.as_bytes())
                .map_err(|err| format!("http host.complete send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 509
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("redirect.status") =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "expected invalid http redirect error, got {other:?}"
                )),
            }
        },
    );

    record(
        &mut report,
        "remote-http-complete-rejects-invalid-cookie",
        || {
            let (runtime, rx) = fresh_runtime();
            runtime
                .send(remote_http_search_command(510))
                .map_err(|err| format!("remote http command send failed: {err:?}"))?;
            expect_http_host_request(&rx, 510)?;
            runtime
                .send_json(HOST_HTTP_COMPLETE_INVALID_COOKIE.as_bytes())
                .map_err(|err| format!("http host.complete send failed: {err:?}"))?;

            match recv_event(&rx)? {
                Event::Error {
                    request_id, error, ..
                } if request_id == 510
                    && error.code == ErrorCode::InvalidParams
                    && error.message.contains("cookie.name") =>
                {
                    Ok(())
                }
                other => Err(format!("expected invalid http cookie error, got {other:?}")),
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
                } if request_id == 310 => {
                    let data =
                        serde_json::from_value::<RuntimeCancelData>(data).map_err(|err| {
                            format!("runtime.cancel data contract parse failed: {err}")
                        })?;
                    if data.cancelled {
                        Ok(())
                    } else {
                        Err(format!("expected cancel result true, got {data:?}"))
                    }
                }
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
                } if request_id == 310 => {
                    let data =
                        serde_json::from_value::<RuntimeCancelData>(data).map_err(|err| {
                            format!("runtime.cancel data contract parse failed: {err}")
                        })?;
                    if !data.cancelled {
                        Ok(())
                    } else {
                        Err(format!("expected cancelled:false result, got {data:?}"))
                    }
                }
                other => Err(format!("expected cancelled:false result, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "runtime-cancel-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                ("cancelled", json!({ "cancelled": "yes" }), "boolean"),
                (
                    "unknown field",
                    json!({
                        "cancelled": true,
                        "requestId": 301
                    }),
                    "unknown field",
                ),
            ] {
                let err = serde_json::from_value::<RuntimeCancelData>(data)
                    .err()
                    .ok_or_else(|| format!("expected runtime.cancel data rejection for {label}"))?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected runtime.cancel data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
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
                } if request_id == 320 => {
                    let status = serde_json::from_value::<RuntimeStatus>(data)
                        .map_err(|err| format!("runtime.status contract parse failed: {err}"))?;
                    if status.active_request_count == 0
                        && status.active_request_ids.is_empty()
                        && status.pending_host_operation_count == 0
                        && status.pending_host_operations.is_empty()
                        && !status.shutting_down
                    {
                        Ok(())
                    } else {
                        Err(format!("unexpected empty runtime.status {status:?}"))
                    }
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
                } if request_id == 320 => {
                    let status = serde_json::from_value::<RuntimeStatus>(data)
                        .map_err(|err| format!("runtime.status contract parse failed: {err}"))?;
                    if status.active_request_count != 1
                        || status.active_request_ids != vec![301]
                        || status.pending_host_operation_count != 1
                    {
                        return Err(format!("unexpected pending runtime.status {status:?}"));
                    }
                    let Some(operation) = status.pending_host_operations.first() else {
                        return Err("pendingHostOperations empty".to_string());
                    };
                    if operation.operation_id != 1
                        || operation.request_id != 301
                        || operation.capability != HostCapability::HostSmokeEcho
                        || operation.state != "pending"
                    {
                        return Err(format!("unexpected pending operation {operation:?}"));
                    }
                    Ok(())
                }
                other => Err(format!("expected pending runtime.status, got {other:?}")),
            }
        },
    );

    record(
        &mut report,
        "runtime-status-data-rejects-zero-active-request-id",
        || {
            let err = serde_json::from_value::<RuntimeStatus>(json!({
                "activeRequestCount": 1,
                "activeRequestIds": [0],
                "pendingHostOperationCount": 0,
                "pendingHostOperations": [],
                "shuttingDown": false
            }))
            .err()
            .ok_or_else(|| "expected runtime.status activeRequestIds rejection".to_string())?;

            if err.to_string().contains("activeRequestIds") {
                Ok(())
            } else {
                Err(format!(
                    "unexpected runtime.status activeRequestIds error: {err}"
                ))
            }
        },
    );

    record(
        &mut report,
        "pending-host-operation-rejects-invalid-state",
        || {
            let err = serde_json::from_value::<PendingHostOperationStatus>(json!({
                "operationId": 1,
                "requestId": 301,
                "capability": "host.smoke.echo",
                "state": "completed"
            }))
            .err()
            .ok_or_else(|| "expected pending host operation state rejection".to_string())?;

            if err.to_string().contains("state") {
                Ok(())
            } else {
                Err(format!("unexpected pending operation state error: {err}"))
            }
        },
    );

    record(
        &mut report,
        "pending-host-operation-rejects-zero-ids",
        || {
            for operation in [
                json!({
                    "operationId": 0,
                    "requestId": 301,
                    "capability": "host.smoke.echo",
                    "state": "pending"
                }),
                json!({
                    "operationId": 1,
                    "requestId": 0,
                    "capability": "host.smoke.echo",
                    "state": "pending"
                }),
            ] {
                let err = serde_json::from_value::<PendingHostOperationStatus>(operation)
                    .err()
                    .ok_or_else(|| "expected pending host operation id rejection".to_string())?;
                if !err.to_string().contains("ids") {
                    return Err(format!("unexpected pending operation id error: {err}"));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "pending-host-operation-rejects-invalid-capability",
        || {
            for capability in ["", "host. smoke.echo", "host..echo", "host", "custom.valid"] {
                let err = serde_json::from_value::<PendingHostOperationStatus>(json!({
                    "operationId": 1,
                    "requestId": 301,
                    "capability": capability,
                    "state": "pending"
                }))
                .err()
                .ok_or_else(|| {
                    format!(
                        "expected pending host operation capability rejection for {capability:?}"
                    )
                })?;
                if !err.to_string().contains("capability") {
                    return Err(format!(
                        "unexpected pending operation capability error: {err}"
                    ));
                }
            }
            Ok(())
        },
    );

    record(
        &mut report,
        "runtime-shutdown-data-rejects-invalid-result-shape",
        || {
            for (label, data, expected) in [
                (
                    "shuttingDown",
                    json!({
                        "shuttingDown": false,
                        "cancelledRequestIds": []
                    }),
                    "shuttingDown",
                ),
                (
                    "cancelledRequestIds",
                    json!({
                        "shuttingDown": true,
                        "cancelledRequestIds": [0]
                    }),
                    "cancelledRequestIds",
                ),
            ] {
                let err = serde_json::from_value::<RuntimeShutdownData>(data)
                    .err()
                    .ok_or_else(|| {
                        format!("expected runtime.shutdown data rejection for {label}")
                    })?;
                if !err.to_string().contains(expected) {
                    return Err(format!(
                        "unexpected runtime.shutdown data error for {label}: {err}"
                    ));
                }
            }
            Ok(())
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
                } if request_id == 330 => {
                    let data =
                        serde_json::from_value::<RuntimeShutdownData>(data).map_err(|err| {
                            format!("runtime.shutdown data contract parse failed: {err}")
                        })?;
                    if !data.shutting_down || !data.cancelled_request_ids.is_empty() {
                        return Err(format!("unexpected runtime.shutdown data {data:?}"));
                    }
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
                } if request_id == 330 => {
                    let data =
                        serde_json::from_value::<RuntimeShutdownData>(data).map_err(|err| {
                            format!("runtime.shutdown data contract parse failed: {err}")
                        })?;
                    if data.shutting_down && data.cancelled_request_ids == vec![301] {
                        Ok(())
                    } else {
                        Err(format!("unexpected runtime.shutdown data {data:?}"))
                    }
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
        } if operation_id > 0
            && params.is_object()
            && matches!(capability, HostCapability::HostSmokeEcho) =>
        {
            Ok(())
        }
        other => Err(format!("expected host.request, got {other:?}")),
    }
}

fn expect_capability_host_request(
    rx: &Receiver<Event>,
    expected_request_id: u64,
    expected_capability: HostCapability,
    expected_params: Value,
) -> Result<(), String> {
    match recv_event(rx)? {
        Event::HostRequest {
            request_id,
            operation_id,
            capability,
            params,
            ..
        } if request_id == expected_request_id
            && operation_id == 1
            && capability == expected_capability
            && params == expected_params =>
        {
            Ok(())
        }
        other => Err(format!(
            "expected {expected_capability} host.request requestId={expected_request_id}, got {other:?}"
        )),
    }
}

fn expect_capability_roundtrip(
    request_json: &str,
    complete_json: &str,
    expected_request_id: u64,
    expected_capability: HostCapability,
    expected_params: Value,
    expected_result: Value,
) -> Result<(), String> {
    let (runtime, rx) = send_to_fresh_runtime(request_json)?;
    expect_capability_host_request(
        &rx,
        expected_request_id,
        expected_capability,
        expected_params,
    )?;
    runtime
        .send_json(complete_json.as_bytes())
        .map_err(|err| format!("{expected_capability} host.complete send failed: {err:?}"))?;
    match recv_event(&rx)? {
        Event::Result {
            request_id, data, ..
        } if request_id == expected_request_id && data == expected_result => Ok(()),
        other => Err(format!(
            "unexpected {expected_capability} completion result {other:?}"
        )),
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
                "headers": {
                    "Accept": "application/json",
                    "Cookie": "sid=old"
                },
                "charset": "gbk",
                "followRedirects": false,
                "maxRedirects": 0,
                "retry": {
                    "maxAttempts": 2,
                    "backoffMillis": 50
                },
                "usePlatformCookieJar": false,
                "session": {
                    "id": "core-session-main"
                }
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
            && capability == HostCapability::HttpExecute
            && params.is_object()
            && params["url"] == "https://books.example.test/search?q=empty"
            && params["method"] == "GET"
            && params["headers"]["Accept"] == "application/json"
            && params["headers"]["Cookie"] == "sid=old"
            && params["charset"] == "gbk"
            && params["followRedirects"] == false
            && params["maxRedirects"] == 0
            && params["retry"]["maxAttempts"] == 2
            && params["retry"]["backoffMillis"] == 50
            && params["usePlatformCookieJar"] == false
            && params["session"]["id"] == "core-session-main" =>
        {
            Ok(operation_id)
        }
        other => Err(format!("unexpected http host.request event {other:?}")),
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

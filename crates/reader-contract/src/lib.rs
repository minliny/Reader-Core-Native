//! Reader-Core JSON command/event protocol types.
//!
//! Mirrors `protocol/reader-command.schema.json` and
//! `protocol/reader-event.schema.json`. This is the single source of truth for
//! the protocol's Rust representation; ABI version lives in `reader-ffi`.
//!
//! - C ABI version: 1 (see `include/reader_core.h`)
//! - JSON protocol version: 1 ([`PROTOCOL_VERSION`])

pub mod command;
pub mod config;
pub mod core_info;
pub mod error;
pub mod event;
pub mod host;
pub mod remote;
pub mod tts;

pub use command::{Command, EmptyParams};
pub use config::RuntimeConfig;
pub use core_info::{core_info, CoreInfoData};
pub use error::{CoreError, ErrorCode};
pub use event::Event;
pub use host::{
    HostCacheGetRequest, HostCacheGetResponse, HostCachePutRequest, HostCachePutResponse,
    HostCapability, HostCompleteParams, HostCookieGetRequest, HostCookieGetResponse,
    HostCookieRecord, HostCookieSetRequest, HostCookieSetResponse, HostErrorCode,
    HostErrorDiagnostics, HostErrorParams, HostErrorPhase, HostFileReadRequest,
    HostFileReadResponse, HostFileWriteRequest, HostFileWriteResponse, HostLogEmitRequest,
    HostLogEmitResponse, HostLogLevel, HostPersistenceGetRequest, HostPersistenceGetResponse,
    HostPersistencePutRequest, HostPersistencePutResponse, HostSmokeParams, HostSystemInfoRequest,
    HostSystemInfoResponse, HostTimeNowRequest, HostTimeNowResponse, HostWebViewDocument,
    HostWebViewDocumentKind, HostWebViewEvaluateJavaScriptRequest,
    HostWebViewEvaluateJavaScriptResponse, PendingHostOperationStatus, RuntimeCancelData,
    RuntimeCancelParams, RuntimePingData, RuntimeShutdownData, RuntimeShutdownParams,
    RuntimeStatus, RuntimeStatusParams,
};
pub use remote::{
    BookDetailBookData, BookDetailData, BookDetailParams, BookGroupCreateData, BookGroupCreateParams,
    BookGroupData, BookGroupDeleteData, BookGroupDeleteParams, BookGroupListData,
    BookGroupListParams, BookGroupUpdateData, BookGroupUpdateParams, BookmarkCreateData,
    BookmarkCreateParams, BookmarkData, BookmarkDeleteData, BookmarkDeleteParams,
    BookmarkListData, BookmarkListParams, BookmarkUpdateData, BookmarkUpdateParams, BookSearchBookData,
    BookSearchData, BookSearchParams, BookTocData, BookTocEntryData, BookTocParams,
    BookshelfEntryData, BookshelfGetData, BookshelfGetParams, BookshelfListData,
    BookshelfListParams, ChapterContentData, ChapterContentParams, ChapterContentVia,
    HostHttpCookie, HostHttpRedirect, HostHttpRequest, HostHttpResponse, LocalBookCatalogData,
    LocalBookCatalogParams, LocalBookParseData, LocalBookParseParams, ReadRecordCreateData,
    ReadRecordCreateParams, ReadRecordData, ReadRecordDeleteData, ReadRecordDeleteParams,
    ReadRecordListData, ReadRecordListParams, ReadRecordUpdateData, ReadRecordUpdateParams,
    ReadingProgressUpdateData, ReadingProgressUpdateParams, RemoteHttpDiagnosticsData,
    ReplaceRuleCreateData, ReplaceRuleCreateParams, ReplaceRuleData, ReplaceRuleDeleteData,
    ReplaceRuleDeleteParams, ReplaceRuleListData, ReplaceRuleListParams, ReplaceRuleUpdateData,
    ReplaceRuleUpdateParams, RssParseData, RssParseEntryData, RssParseParams, RssRefreshData,
    RssRefreshParams, SourceExploreData, SourceExploreKindEntry, SourceExploreKindsData,
    SourceExploreKindsParams, SourceExploreParams, SourceImportData, SourceImportParams,
    SyncBackupData, SyncBackupParams, SyncMergeData, SyncMergeParams, TxtTocRuleCreateData,
    TxtTocRuleCreateParams, TxtTocRuleData, TxtTocRuleDeleteData, TxtTocRuleDeleteParams,
    TxtTocRuleListData, TxtTocRuleListParams, TxtTocRuleUpdateData, TxtTocRuleUpdateParams,
};
pub use tts::{
    TtsChapterPlanData, TtsChapterPlanParams, TtsChapterRef, TtsChapterTransition,
    TtsQueueDrainBehavior, TtsQueueNextData, TtsQueueNextParams, TtsQueuePauseData,
    TtsQueuePauseParams, TtsQueuePlayData, TtsQueuePlayParams, TtsQueuePrevData,
    TtsQueuePrevParams, TtsQueueResumeData, TtsQueueResumeParams, TtsQueueSnapshot, TtsQueueState,
    TtsQueueStatusData, TtsQueueStatusParams, TtsQueueStopData, TtsQueueStopParams, TtsSlice,
    TtsSliceData, TtsSliceParams, TtsSlicePlan, TtsSliceStatus, TtsSlicingStrategy,
};

/// JSON protocol version. Bumped on non-backward-compatible schema changes.
/// See `protocol/compatibility.md`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Method names known to Core in v1.
pub mod methods {
    pub const CORE_INFO: &str = "core.info";
    pub const RUNTIME_PING: &str = "runtime.ping";
    pub const RUNTIME_HOST_SMOKE: &str = "runtime.hostSmoke";
    pub const RUNTIME_CANCEL: &str = "runtime.cancel";
    pub const RUNTIME_STATUS: &str = "runtime.status";
    pub const RUNTIME_SHUTDOWN: &str = "runtime.shutdown";
    pub const HOST_COMPLETE: &str = "host.complete";
    pub const HOST_ERROR: &str = "host.error";

    /// Bootstrap alias kept so the current FFI/Harmony smoke binaries can keep
    /// proving ABI loadability while hosts migrate to `runtime.ping`.
    pub const LEGACY_CORE_PING: &str = "core.ping";

    // --- Remote-reading vertical (V1 minimal) -------------------------------
    /// Import a remote book source definition.
    pub const SOURCE_IMPORT: &str = "source.import";
    /// Search books at a source using a prefetched or host-fetched response.
    pub const BOOK_SEARCH: &str = "book.search";
    /// Fetch/merge book detail metadata from a prefetched or host-fetched response.
    pub const BOOK_DETAIL: &str = "book.detail";
    /// Fetch a book's table of contents from a prefetched or host-fetched response.
    pub const BOOK_TOC: &str = "book.toc";
    /// Extract chapter body text from a prefetched or host-fetched response.
    pub const CHAPTER_CONTENT: &str = "chapter.content";
    /// Update reading progress/state for a book.
    pub const READING_PROGRESS_UPDATE: &str = "reading.progress.update";

    // --- TTS vertical (V1 contract model) ----------------------------------
    /// Slice chapter text into speakable utterances. Core owns slicing;
    /// host owns vocalization.
    pub const TTS_SLICE: &str = "tts.slice";
    /// Query the current TTS playback queue snapshot.
    pub const TTS_QUEUE_STATUS: &str = "tts.queue.status";
    /// Compute the chapter boundary transition plan (current + next + drain
    /// behavior) for a given chapter.
    pub const TTS_CHAPTER_PLAN: &str = "tts.chapter.plan";
    /// Load a slice plan and start playback. Core-owned queue state machine
    /// (Gap F closure): `Idle/Stopped/Completed → Playing`.
    pub const TTS_QUEUE_PLAY: &str = "tts.queue.play";
    /// Pause the active queue. `Playing → Paused`.
    pub const TTS_QUEUE_PAUSE: &str = "tts.queue.pause";
    /// Resume a paused queue. `Paused → Playing`.
    pub const TTS_QUEUE_RESUME: &str = "tts.queue.resume";
    /// Stop the queue. `Playing/Paused/Completed → Stopped` (terminal).
    pub const TTS_QUEUE_STOP: &str = "tts.queue.stop";
    /// Advance the cursor to the next slice. At the last slice, enters
    /// `Completed` (chapter-internal boundary; cross-chapter is Gap G).
    pub const TTS_QUEUE_NEXT: &str = "tts.queue.next";
    /// Move the cursor to the previous slice. Errors at the first slice
    /// (cross-chapter retreat is Gap G).
    pub const TTS_QUEUE_PREV: &str = "tts.queue.prev";
    // --- RSS vertical (V1 minimal) -----------------------------------------
    /// Parse an RSS/Atom XML feed into entries + diagnostics (pure, no network).
    pub const RSS_PARSE: &str = "rss.parse";
    /// Decide whether an RSS subscription should be refreshed (pure policy).
    pub const RSS_REFRESH: &str = "rss.refresh";

    // --- Sync vertical (V1 minimal) ----------------------------------------
    /// Merge two sync snapshots with deterministic last-write-wins (pure).
    pub const SYNC_MERGE: &str = "sync.merge";
    /// Plan backup restore operations from a manifest (pure planner).
    pub const SYNC_BACKUP: &str = "sync.backup";

    // --- Local-book vertical (V1 minimal) ----------------------------------
    /// Parse a local TXT book into book + toc + chapters (pure, no network).
    pub const LOCAL_BOOK_PARSE: &str = "local_book.parse";
    /// Upsert a local-book catalog entry (fingerprint + chapters, pure).
    pub const LOCAL_BOOK_CATALOG: &str = "local_book.catalog";

    // --- Bookshelf vertical (V1 minimal) ----------------------------------
    /// List shelf entries with optional filter/sort/pagination (pure read,
    /// no host callback). Mirrors Legado's books table read path.
    pub const BOOKSHELF_LIST: &str = "bookshelf.list";
    /// Look up a single shelf entry by composite `(sourceId, bookId)` key
    /// (pure read, no host callback). Mirrors Legado's `Book` lookup by
    /// `(origin, bookUrl)`.
    pub const BOOKSHELF_GET: &str = "bookshelf.get";

    // --- Explore vertical (V1 minimal) ------------------------------------
    /// List discovery categories for a source (parses `exploreUrl`).
    /// Mirrors Legado `BookSourceExtensions.getExploreUrl` +
    /// `WebBook.getBookInfo` explore-kind split.
    pub const SOURCE_EXPLORE_KINDS: &str = "source.exploreKinds";
    /// Fetch a discovery category book list from a source. Core emits
    /// `http.execute` for the category URL; host returns the response; Core
    /// parses via `ruleExplore` (falls back to `ruleSearch` per Legado).
    pub const SOURCE_EXPLORE: &str = "source.explore";

    // --- TxtTocRule vertical (V1 minimal) ---------------------------------
    /// Create a TXT-to-contents regex rule. Mirrors Legado `TxtTocRule.kt`
    /// + `TextFile.kt:440-461` chapter-split algorithm.
    pub const TXT_TOC_RULE_CREATE: &str = "txt-toc-rule.create";
    /// List TXT-to-contents rules (optionally enabled-only).
    pub const TXT_TOC_RULE_LIST: &str = "txt-toc-rule.list";
    /// Update a TXT-to-contents rule (partial update by `id`).
    pub const TXT_TOC_RULE_UPDATE: &str = "txt-toc-rule.update";
    /// Delete a TXT-to-contents rule by `id`.
    pub const TXT_TOC_RULE_DELETE: &str = "txt-toc-rule.delete";

    // --- ReplaceRule vertical (V1 minimal) --------------------------------
    /// Create a content replace rule. Mirrors Legado `ReplaceRule.kt` +
    /// `ContentProcessor.kt:91` getContent() replace pipeline.
    pub const REPLACE_RULE_CREATE: &str = "replace-rule.create";
    /// List replace rules (optionally enabled-only).
    pub const REPLACE_RULE_LIST: &str = "replace-rule.list";
    /// Update a replace rule (partial update by `id`).
    pub const REPLACE_RULE_UPDATE: &str = "replace-rule.update";
    /// Delete a replace rule by `id` (idempotent).
    pub const REPLACE_RULE_DELETE: &str = "replace-rule.delete";

    // --- Bookmark vertical (V1 minimal) -----------------------------------
    /// Create a bookmark. Mirrors Legado `Bookmark.kt` (entity) +
    /// `BookmarkDao.kt` (CRUD). Core owns the `bookmarks` table.
    pub const BOOKMARK_CREATE: &str = "bookmark.create";
    /// List bookmarks (optionally filtered by `(bookName, bookAuthor)`).
    pub const BOOKMARK_LIST: &str = "bookmark.list";
    /// Update a bookmark (partial update by `time`).
    pub const BOOKMARK_UPDATE: &str = "bookmark.update";
    /// Delete a bookmark by `time` (idempotent).
    pub const BOOKMARK_DELETE: &str = "bookmark.delete";

    // --- BookGroup vertical (V1 minimal) ----------------------------------
    /// Create a bookshelf group. Mirrors Legado `BookGroup.kt` (entity) +
    /// `BookGroupDao.kt` (CRUD). Core owns the `book_groups` table.
    pub const BOOK_GROUP_CREATE: &str = "book-group.create";
    /// List bookshelf groups (optionally show-only).
    pub const BOOK_GROUP_LIST: &str = "book-group.list";
    /// Update a bookshelf group (partial update by `groupId`).
    pub const BOOK_GROUP_UPDATE: &str = "book-group.update";
    /// Delete a bookshelf group by `groupId` (idempotent).
    pub const BOOK_GROUP_DELETE: &str = "book-group.delete";

    // --- ReadRecord vertical (V1 minimal) ---------------------------------
    /// Create/upsert a reading-time record. Mirrors Legado `ReadRecord.kt`
    /// (entity) + `ReadRecordDao.kt` (CRUD). Core owns the `read_records`
    /// table; composite key `(deviceId, bookName)`.
    pub const READ_RECORD_CREATE: &str = "read-record.create";
    /// List reading-time records (optionally filtered by `deviceId`).
    pub const READ_RECORD_LIST: &str = "read-record.list";
    /// Update a reading-time record (partial update by composite key).
    pub const READ_RECORD_UPDATE: &str = "read-record.update";
    /// Delete a reading-time record by composite key (idempotent).
    pub const READ_RECORD_DELETE: &str = "read-record.delete";
}

/// Non-method capability names advertised by `core.info` in v1.
pub mod capabilities {
    pub const HOST_BUS_V1: &str = "host.bus.v1";
    /// Host-provided HTTP transport capability used by remote-reading commands.
    pub const HTTP_EXECUTE: &str = "http.execute";
    pub const RUNTIME_CONFIG_V1: &str = "runtime.config.v1";
    /// Remote-reading vertical (V1 minimal).
    pub const REMOTE_READING_V1: &str = "remote.reading.v1";
    /// RSS vertical (V1 minimal): feed parsing + refresh decisions.
    pub const RSS_V1: &str = "rss.v1";
    /// Sync vertical (V1 minimal): snapshot merge + backup planning.
    pub const SYNC_V1: &str = "sync.v1";
    /// Local-book vertical (V1 minimal): TXT parsing + catalog bookkeeping.
    pub const LOCAL_BOOK_V1: &str = "local_book.v1";
    /// Bookshelf vertical (V1 minimal): shelf list/get reads over the
    /// in-memory `BookshelfStore`. Mirrors Legado's books table reads.
    pub const BOOKSHELF_V1: &str = "bookshelf.v1";
}

/// Capabilities advertised by `core.info` in v1.
pub const V1_CAPABILITIES: &[&str] = &[
    methods::CORE_INFO,
    methods::RUNTIME_PING,
    methods::RUNTIME_HOST_SMOKE,
    methods::RUNTIME_CANCEL,
    methods::RUNTIME_STATUS,
    methods::RUNTIME_SHUTDOWN,
    methods::HOST_COMPLETE,
    methods::HOST_ERROR,
    capabilities::HOST_BUS_V1,
    capabilities::HTTP_EXECUTE,
    capabilities::RUNTIME_CONFIG_V1,
    capabilities::REMOTE_READING_V1,
    capabilities::RSS_V1,
    capabilities::SYNC_V1,
    capabilities::LOCAL_BOOK_V1,
    capabilities::BOOKSHELF_V1,
];

/// Host-owned capabilities Core may request in v1.
pub const V1_HOST_CAPABILITIES: &[HostCapability] = HostCapability::ALL;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use serde_json::Value;

    #[test]
    fn command_schema_capability_extension_matches_core_info() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let schema_capabilities = strings_at(&schema, "x-reader-core-v1-capabilities");
        assert_eq!(schema_capabilities, V1_CAPABILITIES);

        let info = core_info(1, "test-build");
        let info_capabilities = info["capabilities"]
            .as_array()
            .expect("core.info capabilities must be an array")
            .iter()
            .map(|value| value.as_str().expect("capability must be a string"))
            .collect::<Vec<_>>();
        assert_eq!(info_capabilities, schema_capabilities);
    }

    #[test]
    fn schema_host_capability_extensions_match_contract_enum() {
        let command_schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let event_schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let expected = host_capability_strings();

        assert_eq!(
            strings_at(&command_schema, "x-reader-core-v1-host-capabilities"),
            expected
        );
        assert_eq!(
            strings_at(&event_schema, "x-reader-core-v1-host-capabilities"),
            expected
        );
        assert_eq!(
            strings_at(&command_schema["$defs"]["HostCapability"], "enum"),
            expected
        );
        assert_eq!(
            strings_at(&event_schema["$defs"]["HostCapability"], "enum"),
            expected
        );
    }

    #[test]
    fn command_schema_method_examples_are_current_v1_methods() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let schema_examples = schema["properties"]["method"]["examples"]
            .as_array()
            .expect("method examples must be an array")
            .iter()
            .map(|value| value.as_str().expect("method example must be a string"))
            .collect::<Vec<_>>();

        assert_eq!(
            schema_examples,
            vec![
                methods::CORE_INFO,
                methods::RUNTIME_PING,
                methods::RUNTIME_HOST_SMOKE,
                methods::RUNTIME_CANCEL,
                methods::RUNTIME_STATUS,
                methods::RUNTIME_SHUTDOWN,
                methods::HOST_COMPLETE,
                methods::HOST_ERROR,
                methods::SOURCE_IMPORT,
                methods::BOOK_SEARCH,
                methods::BOOK_DETAIL,
                methods::BOOK_TOC,
                methods::CHAPTER_CONTENT,
                methods::READING_PROGRESS_UPDATE,
                methods::TTS_SLICE,
                methods::TTS_QUEUE_STATUS,
                methods::TTS_CHAPTER_PLAN,
                methods::TTS_QUEUE_PLAY,
                methods::TTS_QUEUE_PAUSE,
                methods::TTS_QUEUE_RESUME,
                methods::TTS_QUEUE_STOP,
                methods::TTS_QUEUE_NEXT,
                methods::TTS_QUEUE_PREV,
                methods::RSS_PARSE,
                methods::RSS_REFRESH,
                methods::SYNC_MERGE,
                methods::SYNC_BACKUP,
                methods::LOCAL_BOOK_PARSE,
                methods::LOCAL_BOOK_CATALOG,
                methods::BOOKSHELF_LIST,
                methods::BOOKSHELF_GET,
                methods::REPLACE_RULE_CREATE,
                methods::REPLACE_RULE_LIST,
                methods::REPLACE_RULE_UPDATE,
                methods::REPLACE_RULE_DELETE,
                methods::SOURCE_EXPLORE_KINDS,
                methods::SOURCE_EXPLORE,
                methods::TXT_TOC_RULE_CREATE,
                methods::TXT_TOC_RULE_LIST,
                methods::TXT_TOC_RULE_UPDATE,
                methods::TXT_TOC_RULE_DELETE,
                methods::BOOKMARK_CREATE,
                methods::BOOKMARK_LIST,
                methods::BOOKMARK_UPDATE,
                methods::BOOKMARK_DELETE,
                methods::BOOK_GROUP_CREATE,
                methods::BOOK_GROUP_LIST,
                methods::BOOK_GROUP_UPDATE,
                methods::BOOK_GROUP_DELETE,
                methods::READ_RECORD_CREATE,
                methods::READ_RECORD_LIST,
                methods::READ_RECORD_UPDATE,
                methods::READ_RECORD_DELETE,
            ]
        );
        assert!(!schema_examples.contains(&methods::LEGACY_CORE_PING));
    }

    #[test]
    fn command_schema_binds_no_param_control_methods_to_empty_params() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        for method in [methods::CORE_INFO, methods::RUNTIME_PING] {
            assert_eq!(
                params_ref_for_method(&schema, method),
                Some("#/$defs/EmptyParams"),
                "{method} must use EmptyParams in command schema"
            );
        }
    }

    #[test]
    fn command_schema_binds_runtime_lifecycle_methods_to_param_defs() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        for (method, params_ref) in [
            (methods::RUNTIME_CANCEL, "#/$defs/RuntimeCancelParams"),
            (methods::RUNTIME_STATUS, "#/$defs/RuntimeStatusParams"),
            (methods::RUNTIME_SHUTDOWN, "#/$defs/RuntimeShutdownParams"),
        ] {
            assert_eq!(
                params_ref_for_method(&schema, method),
                Some(params_ref),
                "{method} must use {params_ref} in command schema"
            );
        }
    }

    #[test]
    fn command_schema_binds_host_bus_methods_to_param_defs() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        for (method, params_ref) in [
            (methods::RUNTIME_HOST_SMOKE, "#/$defs/HostSmokeParams"),
            (methods::HOST_COMPLETE, "#/$defs/HostCompleteParams"),
            (methods::HOST_ERROR, "#/$defs/HostErrorParams"),
        ] {
            assert_eq!(
                params_ref_for_method(&schema, method),
                Some(params_ref),
                "{method} must use {params_ref} in command schema"
            );
        }
    }

    #[test]
    fn command_schema_requires_host_smoke_params_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let params = &schema["$defs"]["HostSmokeParams"]["properties"]["params"];
        let capability = &schema["$defs"]["HostSmokeParams"]["properties"]["capability"];

        assert_eq!(params["type"], serde_json::json!("object"));
        assert_eq!(params["default"], serde_json::json!({}));
        assert_eq!(
            capability["$ref"],
            serde_json::json!("#/$defs/HostCapability")
        );
        assert_eq!(
            capability["default"],
            serde_json::json!(HostCapability::HostSmokeEcho.as_str())
        );
    }

    #[test]
    fn command_schema_defines_host_error_diagnostics_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let diagnostics = &schema["$defs"]["HostErrorParams"]["properties"]["diagnostics"];
        let diagnostics_def = &schema["$defs"]["HostErrorDiagnostics"];

        assert_eq!(
            diagnostics["$ref"],
            serde_json::json!("#/$defs/HostErrorDiagnostics")
        );
        assert_eq!(
            strings_at(diagnostics_def, "required"),
            vec!["code", "phase"]
        );
        assert_eq!(
            diagnostics_def["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(
            diagnostics_def["properties"]["code"]["$ref"],
            serde_json::json!("#/$defs/HostErrorCode")
        );
        assert_eq!(
            diagnostics_def["properties"]["phase"]["$ref"],
            serde_json::json!("#/$defs/HostErrorPhase")
        );
        assert_eq!(
            diagnostics_def["properties"]["details"]["type"],
            serde_json::json!("object")
        );
        assert_eq!(
            owned_strings_at(&schema["$defs"]["HostErrorCode"], "enum"),
            serialize_enum_strings(&[
                HostErrorCode::CapabilityUnavailable,
                HostErrorCode::PermissionDenied,
                HostErrorCode::Timeout,
                HostErrorCode::NetworkError,
                HostErrorCode::TlsError,
                HostErrorCode::HttpError,
                HostErrorCode::InvalidResponse,
                HostErrorCode::Cancelled,
                HostErrorCode::Internal,
            ])
        );
        assert_eq!(
            owned_strings_at(&schema["$defs"]["HostErrorPhase"], "enum"),
            serialize_enum_strings(&[
                HostErrorPhase::Request,
                HostErrorPhase::Transport,
                HostErrorPhase::Response,
                HostErrorPhase::Decode,
                HostErrorPhase::Storage,
                HostErrorPhase::Runtime,
            ])
        );
    }

    #[test]
    fn schemas_define_webview_evaluate_javascript_contract() {
        let command_schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let event_schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let request = &command_schema["$defs"]["HostWebViewEvaluateJavaScriptRequest"];
        let response = &command_schema["$defs"]["HostWebViewEvaluateJavaScriptResponse"];
        let document = &command_schema["$defs"]["HostWebViewDocument"];

        assert_eq!(
            strings_at(request, "required"),
            vec!["document", "javaScript"]
        );
        assert_eq!(request["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            request["properties"]["document"]["$ref"],
            serde_json::json!("#/$defs/HostWebViewDocument")
        );
        assert_eq!(
            request["properties"]["javaScript"]["pattern"],
            serde_json::json!("\\S")
        );
        assert_eq!(
            request["properties"]["timeoutMillis"]["minimum"],
            serde_json::json!(1)
        );
        assert_eq!(strings_at(response, "required"), vec!["value"]);
        assert_eq!(response["additionalProperties"], serde_json::json!(false));
        assert_eq!(document["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            document["properties"]["kind"]["enum"],
            serde_json::json!(["html", "url"])
        );
        assert_eq!(
            command_schema["$defs"]["HostSmokeParams"]["allOf"][0]["then"]["properties"]["params"]
                ["$ref"],
            serde_json::json!("#/$defs/HostWebViewEvaluateJavaScriptRequest")
        );
        assert_eq!(
            event_schema["$defs"]["HostRequestEvent"]["allOf"][0]["then"]["properties"]["params"]
                ["$ref"],
            serde_json::json!("#/$defs/HostWebViewEvaluateJavaScriptRequest")
        );
        assert_eq!(
            event_schema["$defs"]["HostWebViewEvaluateJavaScriptRequest"]["properties"]
                ["javaScript"]["pattern"],
            serde_json::json!("\\S")
        );
    }

    #[test]
    fn schemas_define_remaining_host_capability_contracts() {
        let command_schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let event_schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");

        for (capability, request_ref) in [
            ("file.read", "#/$defs/HostFileReadRequest"),
            ("file.write", "#/$defs/HostFileWriteRequest"),
            ("cache.get", "#/$defs/HostCacheGetRequest"),
            ("cache.put", "#/$defs/HostCachePutRequest"),
            ("cookie.get", "#/$defs/HostCookieGetRequest"),
            ("cookie.set", "#/$defs/HostCookieSetRequest"),
            ("log.emit", "#/$defs/HostLogEmitRequest"),
            ("time.now", "#/$defs/HostTimeNowRequest"),
            ("system.info", "#/$defs/HostSystemInfoRequest"),
            ("persistence.get", "#/$defs/HostPersistenceGetRequest"),
            ("persistence.put", "#/$defs/HostPersistencePutRequest"),
        ] {
            assert_eq!(
                capability_params_ref(&command_schema["$defs"]["HostSmokeParams"], capability),
                Some(request_ref),
                "HostSmokeParams must bind {capability} request params"
            );
            assert_eq!(
                capability_params_ref(&event_schema["$defs"]["HostRequestEvent"], capability),
                Some(request_ref),
                "HostRequestEvent must bind {capability} request params"
            );
        }

        let file_read_request = &command_schema["$defs"]["HostFileReadRequest"];
        assert_eq!(strings_at(file_read_request, "required"), vec!["path"]);
        assert_eq!(
            file_read_request["properties"]["path"]["pattern"],
            serde_json::json!("\\S")
        );
        assert_eq!(
            file_read_request["properties"]["maxBytes"]["minimum"],
            serde_json::json!(1)
        );

        let file_read_response = &command_schema["$defs"]["HostFileReadResponse"];
        assert_eq!(
            file_read_response["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(
            file_read_response["oneOf"].as_array().map(Vec::len),
            Some(2)
        );

        let file_write_response = &command_schema["$defs"]["HostFileWriteResponse"];
        assert_eq!(
            file_write_response["properties"]["written"]["const"],
            serde_json::json!(true)
        );

        let cache_get_request = &command_schema["$defs"]["HostCacheGetRequest"];
        assert_eq!(
            strings_at(cache_get_request, "required"),
            vec!["namespace", "key"]
        );
        assert_eq!(
            cache_get_request["properties"]["namespace"]["pattern"],
            serde_json::json!("\\S")
        );

        let cache_get_response = &command_schema["$defs"]["HostCacheGetResponse"];
        assert_eq!(strings_at(cache_get_response, "required"), vec!["hit"]);
        assert_eq!(
            cache_get_response["allOf"][0]["then"]["oneOf"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );

        let cache_put_request = &command_schema["$defs"]["HostCachePutRequest"];
        assert_eq!(
            strings_at(cache_put_request, "required"),
            vec!["namespace", "key"]
        );
        assert_eq!(
            cache_put_request["properties"]["ttlMillis"]["minimum"],
            serde_json::json!(1)
        );

        let cache_put_response = &command_schema["$defs"]["HostCachePutResponse"];
        assert_eq!(
            cache_put_response["properties"]["stored"]["const"],
            serde_json::json!(true)
        );

        let cookie_record = &command_schema["$defs"]["HostCookieRecord"];
        assert_eq!(strings_at(cookie_record, "required"), vec!["name"]);
        assert_eq!(
            cookie_record["properties"]["name"]["pattern"],
            serde_json::json!("\\S")
        );
        let cookie_get_request = &command_schema["$defs"]["HostCookieGetRequest"];
        assert_eq!(
            cookie_get_request["anyOf"].as_array().map(Vec::len),
            Some(3)
        );
        let cookie_get_response = &command_schema["$defs"]["HostCookieGetResponse"];
        assert_eq!(strings_at(cookie_get_response, "required"), vec!["cookies"]);
        assert_eq!(
            cookie_get_response["properties"]["cookies"]["items"]["$ref"],
            serde_json::json!("#/$defs/HostCookieRecord")
        );
        let cookie_set_response = &command_schema["$defs"]["HostCookieSetResponse"];
        assert_eq!(
            cookie_set_response["properties"]["stored"]["const"],
            serde_json::json!(true)
        );

        let log_request = &command_schema["$defs"]["HostLogEmitRequest"];
        assert_eq!(
            strings_at(log_request, "required"),
            vec!["level", "message"]
        );
        assert_eq!(
            command_schema["$defs"]["HostLogLevel"]["enum"],
            serde_json::json!(["trace", "debug", "info", "warn", "error"])
        );
        assert_eq!(
            command_schema["$defs"]["HostLogEmitResponse"]["properties"]["emitted"]["const"],
            serde_json::json!(true)
        );

        let time_response = &command_schema["$defs"]["HostTimeNowResponse"];
        assert_eq!(
            strings_at(time_response, "required"),
            vec!["unixMillis", "iso8601"]
        );
        assert_eq!(
            time_response["properties"]["iso8601"]["pattern"],
            serde_json::json!("\\S")
        );

        let system_info_response = &command_schema["$defs"]["HostSystemInfoResponse"];
        assert_eq!(
            system_info_response["properties"]["info"]["type"],
            serde_json::json!("object")
        );

        let persistence_get_request = &command_schema["$defs"]["HostPersistenceGetRequest"];
        assert_eq!(
            strings_at(persistence_get_request, "required"),
            vec!["namespace", "key"]
        );
        let persistence_get_response = &command_schema["$defs"]["HostPersistenceGetResponse"];
        assert_eq!(
            strings_at(persistence_get_response, "required"),
            vec!["found"]
        );
        assert_eq!(
            persistence_get_response["allOf"][0]["then"]["oneOf"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        let persistence_put_response = &command_schema["$defs"]["HostPersistencePutResponse"];
        assert_eq!(
            persistence_put_response["properties"]["stored"]["const"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn command_schema_requires_host_complete_result_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let result = &schema["$defs"]["HostCompleteParams"]["properties"]["result"];

        assert_eq!(result["type"], serde_json::json!("object"));
    }

    #[test]
    fn command_schema_binds_source_import_to_param_def() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        assert_eq!(
            params_ref_for_method(&schema, methods::SOURCE_IMPORT),
            Some("#/$defs/SourceImportParams"),
            "source.import must use SourceImportParams in command schema"
        );
    }

    #[test]
    fn command_schema_binds_book_search_to_param_def() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        assert_eq!(
            params_ref_for_method(&schema, methods::BOOK_SEARCH),
            Some("#/$defs/BookSearchParams"),
            "book.search must use BookSearchParams in command schema"
        );
    }

    #[test]
    fn command_schema_binds_book_detail_to_param_def() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        assert_eq!(
            params_ref_for_method(&schema, methods::BOOK_DETAIL),
            Some("#/$defs/BookDetailParams"),
            "book.detail must use BookDetailParams in command schema"
        );
    }

    #[test]
    fn command_schema_requires_book_detail_book_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let book = &schema["$defs"]["BookDetailParams"]["properties"]["book"];

        assert_eq!(book["type"], serde_json::json!("object"));
    }

    #[test]
    fn command_schema_binds_book_toc_to_param_def() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        assert_eq!(
            params_ref_for_method(&schema, methods::BOOK_TOC),
            Some("#/$defs/BookTocParams"),
            "book.toc must use BookTocParams in command schema"
        );
    }

    #[test]
    fn command_schema_binds_chapter_content_to_param_def() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        assert_eq!(
            params_ref_for_method(&schema, methods::CHAPTER_CONTENT),
            Some("#/$defs/ChapterContentParams"),
            "chapter.content must use ChapterContentParams in command schema"
        );
    }

    #[test]
    fn command_schema_binds_reading_progress_update_to_param_def() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        assert_eq!(
            params_ref_for_method(&schema, methods::READING_PROGRESS_UPDATE),
            Some("#/$defs/ReadingProgressUpdateParams"),
            "reading.progress.update must use ReadingProgressUpdateParams in command schema"
        );
    }

    #[test]
    fn command_schema_bounds_reading_progress_update_progress() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let progress =
            &schema["$defs"]["ReadingProgressUpdateParams"]["properties"]["chapterProgress"];

        assert_eq!(progress["minimum"], serde_json::json!(0));
        assert_eq!(progress["maximum"], serde_json::json!(1));
    }

    #[test]
    fn command_schema_rejects_blank_host_http_request_method() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let method = &schema["$defs"]["HostHttpRequest"]["properties"]["method"];

        assert_eq!(method["minLength"], serde_json::json!(1));
        assert_eq!(method["pattern"], serde_json::json!("\\S"));
    }

    #[test]
    fn command_schema_requires_host_http_request_headers_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let headers = &schema["$defs"]["HostHttpRequest"]["properties"]["headers"];

        assert_eq!(headers["type"], serde_json::json!("object"));
        assert_eq!(headers["default"], serde_json::json!({}));
    }

    #[test]
    fn command_schema_rejects_blank_host_http_request_url() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let url = &schema["$defs"]["HostHttpRequest"]["properties"]["url"];

        assert_eq!(url["minLength"], serde_json::json!(1));
        assert_eq!(url["pattern"], serde_json::json!("\\S"));
    }

    #[test]
    fn command_schema_defines_host_http_response_redirect_cookie_metadata() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let response = &schema["$defs"]["HostHttpResponse"];
        let redirect = &schema["$defs"]["HostHttpRedirect"];
        let cookie = &schema["$defs"]["HostHttpCookie"];

        assert_eq!(
            response["properties"]["redirects"]["items"]["$ref"],
            serde_json::json!("#/$defs/HostHttpRedirect")
        );
        assert_eq!(
            response["properties"]["cookies"]["items"]["$ref"],
            serde_json::json!("#/$defs/HostHttpCookie")
        );
        assert_eq!(redirect["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            strings_at(redirect, "required"),
            vec!["status", "fromUrl", "toUrl"]
        );
        assert_eq!(
            redirect["properties"]["status"]["minimum"],
            serde_json::json!(300)
        );
        assert_eq!(
            redirect["properties"]["status"]["maximum"],
            serde_json::json!(399)
        );
        assert_eq!(
            redirect["properties"]["headers"]["type"],
            serde_json::json!("object")
        );
        assert_eq!(cookie["additionalProperties"], serde_json::json!(false));
        assert_eq!(strings_at(cookie, "required"), vec!["name"]);
        assert_eq!(
            cookie["properties"]["name"]["pattern"],
            serde_json::json!("\\S")
        );
        assert_eq!(
            cookie["properties"]["domain"]["pattern"],
            serde_json::json!("\\S")
        );
        assert_eq!(
            cookie["properties"]["httpOnly"]["type"],
            serde_json::json!("boolean")
        );
        assert_eq!(
            cookie["properties"]["secure"]["type"],
            serde_json::json!("boolean")
        );
    }

    #[test]
    fn command_schema_rejects_blank_source_import_name() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let name = &schema["$defs"]["SourceImportParams"]["properties"]["name"];

        assert_eq!(name["minLength"], serde_json::json!(1));
        assert_eq!(name["pattern"], serde_json::json!("\\S"));
    }

    #[test]
    fn command_schema_requires_source_import_rules_object_or_null() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let rules = &schema["$defs"]["SourceImportParams"]["properties"]["rules"];

        assert_eq!(rules["type"], serde_json::json!(["object", "null"]));
        assert_eq!(rules["default"], serde_json::json!(null));
    }

    #[test]
    fn command_schema_allows_source_import_raw_legado_book_source() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");
        let book_source = &schema["$defs"]["SourceImportParams"]["properties"]["bookSource"];

        assert_eq!(book_source["type"], serde_json::json!(["object", "null"]));
        assert_eq!(book_source["default"], serde_json::json!(null));
    }

    #[test]
    fn command_schema_requires_remote_inline_source_object_or_null() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        for params in [
            "BookSearchParams",
            "BookDetailParams",
            "BookTocParams",
            "ChapterContentParams",
        ] {
            let source = &schema["$defs"][params]["properties"]["source"];
            assert_eq!(source["type"], serde_json::json!(["object", "null"]));
            assert_eq!(source["default"], serde_json::json!(null));
        }
    }

    #[test]
    fn event_schema_error_codes_match_error_code_enum() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let schema_codes = schema["$defs"]["CoreError"]["properties"]["code"]["enum"]
            .as_array()
            .expect("error code enum must be an array")
            .iter()
            .map(|value| value.as_str().expect("error code must be a string"))
            .collect::<Vec<_>>();

        let rust_codes = [
            ErrorCode::UnknownMethod,
            ErrorCode::InvalidParams,
            ErrorCode::InvalidProtocolVersion,
            ErrorCode::Cancelled,
            ErrorCode::InvalidMessage,
            ErrorCode::Internal,
        ]
        .into_iter()
        .map(|code| {
            serde_json::to_value(code)
                .expect("error code must serialize")
                .as_str()
                .expect("error code must serialize as string")
                .to_string()
        })
        .collect::<Vec<_>>();

        assert_eq!(schema_codes, rust_codes);
    }

    #[test]
    fn event_schema_requires_core_error_details_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let details = &schema["$defs"]["CoreError"]["properties"]["details"];

        assert_eq!(details["type"], serde_json::json!("object"));
    }

    #[test]
    fn event_schema_rejects_core_error_unknown_fields() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let core_error = &schema["$defs"]["CoreError"];

        assert_eq!(core_error["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn event_schema_requires_result_data_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["ResultEvent"]["properties"]["data"];

        assert_eq!(data["type"], serde_json::json!("object"));
    }

    #[test]
    fn event_schema_requires_current_protocol_version() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");

        for event_def in ["ResultEvent", "ErrorEvent", "HostRequestEvent"] {
            assert_eq!(
                schema["$defs"][event_def]["properties"]["protocolVersion"]["const"],
                serde_json::json!(PROTOCOL_VERSION),
                "{event_def} protocolVersion must match PROTOCOL_VERSION"
            );
        }
    }

    #[test]
    fn event_schema_requires_non_error_event_request_ids_positive() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");

        for event_def in ["ResultEvent", "HostRequestEvent"] {
            assert_eq!(
                schema["$defs"][event_def]["properties"]["requestId"]["minimum"],
                serde_json::json!(1),
                "{event_def} requestId must be positive"
            );
        }
        assert!(
            schema["$defs"]["ErrorEvent"]["properties"]["requestId"]
                .get("minimum")
                .is_none(),
            "ErrorEvent requestId 0 is reserved for process-level errors"
        );
    }

    #[test]
    fn event_schema_requires_host_request_params_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let params = &schema["$defs"]["HostRequestEvent"]["properties"]["params"];

        assert_eq!(params["type"], serde_json::json!("object"));
    }

    #[test]
    fn event_schema_requires_host_request_operation_id_minimum() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let operation_id = &schema["$defs"]["HostRequestEvent"]["properties"]["operationId"];

        assert_eq!(operation_id["minimum"], serde_json::json!(1));
    }

    #[test]
    fn event_schema_defines_host_capability_enum() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let capability = &schema["$defs"]["HostCapability"];
        let values = strings_at(capability, "enum");
        let expected = host_capability_strings();

        assert_eq!(capability["type"], serde_json::json!("string"));
        assert_eq!(values, expected);
        assert_eq!(
            schema["$defs"]["HostRequestEvent"]["properties"]["capability"]["$ref"],
            serde_json::json!("#/$defs/HostCapability")
        );
    }

    #[test]
    fn event_schema_rejects_unknown_top_level_fields() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");

        for event_def in ["ResultEvent", "ErrorEvent", "HostRequestEvent"] {
            assert_eq!(
                schema["$defs"][event_def]["additionalProperties"],
                serde_json::json!(false),
                "{event_def} must reject unknown top-level fields"
            );
        }
    }

    #[test]
    fn event_schema_requires_pending_host_operation_state_pending() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let state = &schema["$defs"]["PendingHostOperationStatus"]["properties"]["state"];

        assert_eq!(state["const"], serde_json::json!("pending"));
    }

    #[test]
    fn event_schema_requires_pending_host_operation_positive_ids() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let properties = &schema["$defs"]["PendingHostOperationStatus"]["properties"];

        assert_eq!(properties["operationId"]["minimum"], serde_json::json!(1));
        assert_eq!(properties["requestId"]["minimum"], serde_json::json!(1));
    }

    #[test]
    fn event_schema_requires_runtime_status_active_request_ids_positive() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let active_request_ids =
            &schema["$defs"]["RuntimeStatusData"]["properties"]["activeRequestIds"];

        assert_eq!(active_request_ids["items"]["minimum"], serde_json::json!(1));
    }

    #[test]
    fn event_schema_uses_host_capability_for_pending_host_operations() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let capability = &schema["$defs"]["PendingHostOperationStatus"]["properties"]["capability"];

        assert_eq!(
            capability["$ref"],
            serde_json::json!("#/$defs/HostCapability")
        );
    }

    #[test]
    fn event_schema_requires_runtime_shutdown_data_contract_bounds() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let properties = &schema["$defs"]["RuntimeShutdownData"]["properties"];

        assert_eq!(properties["shuttingDown"]["const"], serde_json::json!(true));
        assert_eq!(
            properties["cancelledRequestIds"]["items"]["minimum"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn event_schema_defines_core_info_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["CoreInfoData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];
        let capabilities = properties["capabilities"]["prefixItems"]
            .as_array()
            .expect("core.info capabilities prefixItems must be an array")
            .iter()
            .map(|item| {
                item["const"]
                    .as_str()
                    .expect("core.info capability prefix item must be const string")
            })
            .collect::<Vec<_>>();

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            required,
            vec![
                "abiVersion",
                "protocolVersion",
                "buildVersion",
                "capabilities"
            ]
        );
        assert_eq!(properties["abiVersion"]["minimum"], serde_json::json!(1));
        assert_eq!(
            properties["protocolVersion"]["const"],
            serde_json::json!(PROTOCOL_VERSION)
        );
        assert_eq!(
            properties["buildVersion"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            properties["capabilities"]["minItems"],
            serde_json::json!(V1_CAPABILITIES.len())
        );
        assert_eq!(
            properties["capabilities"]["maxItems"],
            serde_json::json!(V1_CAPABILITIES.len())
        );
        assert_eq!(capabilities, V1_CAPABILITIES);
    }

    #[test]
    fn event_schema_defines_source_import_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["SourceImportData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(required, vec!["sourceId", "name", "imported"]);
        assert_eq!(properties["sourceId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["sourceId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(properties["name"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["name"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(properties["imported"]["const"], serde_json::json!(true));
    }

    #[test]
    fn event_schema_defines_book_search_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["BookSearchData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];
        let book = &schema["$defs"]["BookSearchBookData"];
        let book_required = strings_at(book, "required");
        let http = &schema["$defs"]["RemoteHttpDiagnosticsData"];

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(required, vec!["sourceId", "books"]);
        assert_eq!(properties["sourceId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["sourceId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(properties["books"]["type"], serde_json::json!("array"));
        assert_eq!(
            properties["books"]["items"]["$ref"],
            serde_json::json!("#/$defs/BookSearchBookData")
        );
        assert_eq!(
            properties["http"]["$ref"],
            serde_json::json!("#/$defs/RemoteHttpDiagnosticsData")
        );
        assert_eq!(book_required, vec!["bookId", "title"]);
        assert_eq!(book["additionalProperties"], Value::Null);
        assert_eq!(
            book["properties"]["bookId"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            book["properties"]["title"]["pattern"],
            serde_json::json!("\\S")
        );
        assert_eq!(http["additionalProperties"], serde_json::json!(false));
        assert_eq!(http["minProperties"], serde_json::json!(1));
        assert_eq!(
            http["properties"]["status"]["minimum"],
            serde_json::json!(100)
        );
        assert_eq!(
            http["properties"]["status"]["maximum"],
            serde_json::json!(599)
        );
        assert_eq!(
            http["properties"]["headers"]["type"],
            serde_json::json!("object")
        );
        assert_eq!(
            http["properties"]["redirects"]["items"]["$ref"],
            serde_json::json!("#/$defs/HostHttpRedirect")
        );
        assert_eq!(
            http["properties"]["cookies"]["items"]["$ref"],
            serde_json::json!("#/$defs/HostHttpCookie")
        );
    }

    #[test]
    fn event_schema_defines_book_detail_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["BookDetailData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];
        let book = &schema["$defs"]["BookDetailBookData"];
        let book_required = strings_at(book, "required");

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(required, vec!["sourceId", "book"]);
        assert_eq!(properties["sourceId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["sourceId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(
            properties["book"]["$ref"],
            serde_json::json!("#/$defs/BookDetailBookData")
        );
        assert_eq!(
            properties["http"]["$ref"],
            serde_json::json!("#/$defs/RemoteHttpDiagnosticsData")
        );
        assert_eq!(book["additionalProperties"], serde_json::json!(false));
        assert_eq!(book_required, vec!["bookId", "title", "author"]);
        assert_eq!(
            book["properties"]["bookId"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            book["properties"]["bookId"]["pattern"],
            serde_json::json!("\\S")
        );
        assert_eq!(
            book["properties"]["title"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            book["properties"]["author"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            book["properties"]["intro"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            book["properties"]["lastChapter"]["type"],
            serde_json::json!("string")
        );
    }

    #[test]
    fn event_schema_defines_book_toc_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["BookTocData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];
        let entry = &schema["$defs"]["BookTocEntryData"];
        let entry_required = strings_at(entry, "required");

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(required, vec!["sourceId", "bookId", "toc"]);
        assert_eq!(properties["sourceId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["sourceId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(properties["bookId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["bookId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(properties["toc"]["type"], serde_json::json!("array"));
        assert_eq!(
            properties["toc"]["items"]["$ref"],
            serde_json::json!("#/$defs/BookTocEntryData")
        );
        assert_eq!(
            properties["http"]["$ref"],
            serde_json::json!("#/$defs/RemoteHttpDiagnosticsData")
        );
        assert_eq!(entry["additionalProperties"], serde_json::json!(false));
        assert_eq!(entry_required, vec!["index", "title", "url"]);
        assert_eq!(
            entry["properties"]["index"]["minimum"],
            serde_json::json!(0)
        );
        assert_eq!(
            entry["properties"]["title"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            entry["properties"]["url"]["type"],
            serde_json::json!("string")
        );
    }

    #[test]
    fn event_schema_defines_chapter_content_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["ChapterContentData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];
        let content_types = strings_at(&properties["content"], "type");
        let via_values = strings_at(&properties["via"], "enum");

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            required,
            vec!["sourceId", "bookId", "chapterTitle", "content", "via"]
        );
        assert_eq!(properties["sourceId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["sourceId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(properties["bookId"]["minLength"], serde_json::json!(1));
        assert_eq!(properties["bookId"]["pattern"], serde_json::json!("\\S"));
        assert_eq!(
            properties["chapterTitle"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            content_types,
            vec!["string", "object", "array", "number", "boolean", "null"]
        );
        assert_eq!(via_values, vec!["rule", "js"]);
        assert_eq!(
            properties["http"]["$ref"],
            serde_json::json!("#/$defs/RemoteHttpDiagnosticsData")
        );
    }

    #[test]
    fn event_schema_defines_runtime_ping_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["RuntimePingData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(required, vec!["pong", "method"]);
        assert_eq!(properties["pong"]["const"], serde_json::json!(true));
        assert_eq!(
            properties["method"]["const"],
            serde_json::json!(methods::RUNTIME_PING)
        );
    }

    #[test]
    fn event_schema_defines_runtime_cancel_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["RuntimeCancelData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(required, vec!["cancelled"]);
        assert_eq!(
            properties["cancelled"]["type"],
            serde_json::json!("boolean")
        );
    }

    #[test]
    fn event_schema_defines_reading_progress_update_data_contract() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["ReadingProgressUpdateData"];
        let required = strings_at(data, "required");
        let properties = &data["properties"];

        assert_eq!(data["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            required,
            vec![
                "bookId",
                "chapterIndex",
                "chapterOffset",
                "chapterProgress",
                "stored"
            ]
        );
        assert_eq!(properties["chapterIndex"]["minimum"], serde_json::json!(0));
        assert_eq!(properties["chapterOffset"]["minimum"], serde_json::json!(0));
        assert_eq!(
            properties["chapterProgress"]["minimum"],
            serde_json::json!(0)
        );
        assert_eq!(
            properties["chapterProgress"]["maximum"],
            serde_json::json!(1)
        );
        assert_eq!(properties["stored"]["const"], serde_json::json!(true));
    }

    #[test]
    fn command_schema_binds_tts_methods_to_param_defs() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        for (method, params_ref) in [
            (methods::TTS_SLICE, "#/$defs/TtsSliceParams"),
            (methods::TTS_QUEUE_STATUS, "#/$defs/TtsQueueStatusParams"),
            (methods::TTS_CHAPTER_PLAN, "#/$defs/TtsChapterPlanParams"),
            (methods::TTS_QUEUE_PLAY, "#/$defs/TtsQueuePlayParams"),
            (methods::TTS_QUEUE_PAUSE, "#/$defs/TtsQueuePauseParams"),
            (methods::TTS_QUEUE_RESUME, "#/$defs/TtsQueueResumeParams"),
            (methods::TTS_QUEUE_STOP, "#/$defs/TtsQueueStopParams"),
            (methods::TTS_QUEUE_NEXT, "#/$defs/TtsQueueNextParams"),
            (methods::TTS_QUEUE_PREV, "#/$defs/TtsQueuePrevParams"),
        ] {
            assert_eq!(
                params_ref_for_method(&schema, method),
                Some(params_ref),
                "{method} must use {params_ref} in command schema"
            );
        }
    }

    #[test]
    fn command_schema_defines_tts_param_contracts() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        let slice_params = &schema["$defs"]["TtsSliceParams"];
        assert_eq!(
            slice_params["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(
            strings_at(slice_params, "required"),
            vec!["chapter", "content"]
        );
        assert_eq!(
            slice_params["properties"]["chapter"]["$ref"],
            serde_json::json!("#/$defs/TtsChapterRef")
        );
        assert_eq!(
            slice_params["properties"]["content"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            slice_params["properties"]["strategy"]["default"],
            serde_json::json!("paragraph")
        );

        let queue_params = &schema["$defs"]["TtsQueueStatusParams"];
        assert_eq!(
            queue_params["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(strings_at(queue_params, "required"), vec!["chapter"]);
        assert_eq!(
            queue_params["properties"]["chapter"]["$ref"],
            serde_json::json!("#/$defs/TtsChapterRef")
        );

        let plan_params = &schema["$defs"]["TtsChapterPlanParams"];
        assert_eq!(
            plan_params["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(strings_at(plan_params, "required"), vec!["chapter"]);
        assert_eq!(
            plan_params["properties"]["drainBehavior"]["default"],
            serde_json::json!("stop-on-boundary")
        );
    }

    #[test]
    fn command_schema_defines_tts_queue_control_params() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        // play params: requires plan, optional startSliceIndex (default 0).
        let play_params = &schema["$defs"]["TtsQueuePlayParams"];
        assert_eq!(
            play_params["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(strings_at(play_params, "required"), vec!["plan"]);
        assert_eq!(
            play_params["properties"]["plan"]["$ref"],
            serde_json::json!("#/$defs/TtsSlicePlan")
        );
        assert_eq!(
            play_params["properties"]["startSliceIndex"]["default"],
            serde_json::json!(0)
        );
        assert_eq!(
            play_params["properties"]["startSliceIndex"]["minimum"],
            serde_json::json!(0)
        );

        // pause/resume/stop/next/prev: each requires chapter only.
        for def_name in [
            "TtsQueuePauseParams",
            "TtsQueueResumeParams",
            "TtsQueueStopParams",
            "TtsQueueNextParams",
            "TtsQueuePrevParams",
        ] {
            let params = &schema["$defs"][def_name];
            assert_eq!(params["additionalProperties"], serde_json::json!(false));
            assert_eq!(strings_at(params, "required"), vec!["chapter"]);
            assert_eq!(
                params["properties"]["chapter"]["$ref"],
                serde_json::json!("#/$defs/TtsChapterRef")
            );
        }
    }

    #[test]
    fn command_schema_defines_tts_data_model_defs() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-command.schema.json"))
                .expect("command schema must be valid JSON");

        let chapter_ref = &schema["$defs"]["TtsChapterRef"];
        assert_eq!(
            chapter_ref["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(
            strings_at(chapter_ref, "required"),
            vec!["sourceId", "bookId", "chapterIndex"]
        );
        assert_eq!(
            chapter_ref["properties"]["sourceId"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            chapter_ref["properties"]["bookId"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            chapter_ref["properties"]["chapterIndex"]["minimum"],
            serde_json::json!(0)
        );

        let strategy = &schema["$defs"]["TtsSlicingStrategy"];
        assert_eq!(
            strings_at(strategy, "enum"),
            vec![
                "paragraph",
                "sentence",
                "paragraph-then-sentence",
                "line-break"
            ]
        );
        assert_eq!(strategy["default"], serde_json::json!("paragraph"));

        let slice = &schema["$defs"]["TtsSlice"];
        assert_eq!(slice["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            strings_at(slice, "required"),
            vec!["index", "text", "charStart", "charEnd", "paragraphIndex"]
        );
        assert_eq!(
            slice["properties"]["text"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            slice["properties"]["index"]["minimum"],
            serde_json::json!(0)
        );

        let plan = &schema["$defs"]["TtsSlicePlan"];
        assert_eq!(plan["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            strings_at(plan, "required"),
            vec!["chapter", "slices", "sourceCharCount"]
        );
        assert_eq!(
            plan["properties"]["chapter"]["$ref"],
            serde_json::json!("#/$defs/TtsChapterRef")
        );
        assert_eq!(
            plan["properties"]["slices"]["items"]["$ref"],
            serde_json::json!("#/$defs/TtsSlice")
        );
        assert_eq!(
            plan["properties"]["sourceCharCount"]["minimum"],
            serde_json::json!(0)
        );

        let queue_state = &schema["$defs"]["TtsQueueState"];
        assert_eq!(
            strings_at(queue_state, "enum"),
            vec!["idle", "playing", "paused", "completed", "stopped"]
        );

        let slice_status = &schema["$defs"]["TtsSliceStatus"];
        assert_eq!(
            strings_at(slice_status, "enum"),
            vec!["pending", "speaking", "done", "skipped", "failed"]
        );

        let snapshot = &schema["$defs"]["TtsQueueSnapshot"];
        assert_eq!(snapshot["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            strings_at(snapshot, "required"),
            vec!["state", "totalSlices", "completedSlices", "chapter"]
        );
        assert_eq!(
            snapshot["properties"]["state"]["$ref"],
            serde_json::json!("#/$defs/TtsQueueState")
        );
        assert_eq!(
            snapshot["properties"]["chapter"]["$ref"],
            serde_json::json!("#/$defs/TtsChapterRef")
        );
        assert_eq!(
            snapshot["properties"]["sliceStatuses"]["items"]["$ref"],
            serde_json::json!("#/$defs/TtsSliceStatus")
        );

        let drain = &schema["$defs"]["TtsQueueDrainBehavior"];
        assert_eq!(
            strings_at(drain, "enum"),
            vec!["stop-on-boundary", "advance-to-next"]
        );
        assert_eq!(drain["default"], serde_json::json!("stop-on-boundary"));

        let transition = &schema["$defs"]["TtsChapterTransition"];
        assert_eq!(transition["additionalProperties"], serde_json::json!(false));
        assert_eq!(strings_at(transition, "required"), vec!["current"]);
        assert_eq!(
            transition["properties"]["current"]["$ref"],
            serde_json::json!("#/$defs/TtsChapterRef")
        );
        assert_eq!(
            transition["properties"]["next"]["anyOf"][1]["$ref"],
            serde_json::json!("#/$defs/TtsChapterRef")
        );
        assert_eq!(
            transition["properties"]["drainBehavior"]["default"],
            serde_json::json!("stop-on-boundary")
        );
    }

    #[test]
    fn event_schema_defines_tts_data_contracts() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");

        let slice_data = &schema["$defs"]["TtsSliceData"];
        assert_eq!(slice_data["additionalProperties"], serde_json::json!(false));
        assert_eq!(strings_at(slice_data, "required"), vec!["plan"]);
        assert_eq!(
            slice_data["properties"]["plan"]["$ref"],
            serde_json::json!("#/$defs/TtsSlicePlan")
        );

        let queue_data = &schema["$defs"]["TtsQueueStatusData"];
        assert_eq!(queue_data["additionalProperties"], serde_json::json!(false));
        assert_eq!(strings_at(queue_data, "required"), vec!["snapshot"]);
        assert_eq!(
            queue_data["properties"]["snapshot"]["$ref"],
            serde_json::json!("#/$defs/TtsQueueSnapshot")
        );

        let plan_data = &schema["$defs"]["TtsChapterPlanData"];
        assert_eq!(plan_data["additionalProperties"], serde_json::json!(false));
        assert_eq!(strings_at(plan_data, "required"), vec!["transition"]);
        assert_eq!(
            plan_data["properties"]["transition"]["$ref"],
            serde_json::json!("#/$defs/TtsChapterTransition")
        );

        // Queue control result data (Gap F closure): each wraps a snapshot.
        for def_name in [
            "TtsQueuePlayData",
            "TtsQueuePauseData",
            "TtsQueueResumeData",
            "TtsQueueStopData",
            "TtsQueueNextData",
            "TtsQueuePrevData",
        ] {
            let data = &schema["$defs"][def_name];
            assert_eq!(data["additionalProperties"], serde_json::json!(false));
            assert_eq!(strings_at(data, "required"), vec!["snapshot"]);
            assert_eq!(
                data["properties"]["snapshot"]["$ref"],
                serde_json::json!("#/$defs/TtsQueueSnapshot")
            );
        }
    }

    fn strings_at<'a>(value: &'a Value, key: &str) -> Vec<&'a str> {
        value[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} must be an array"))
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("{key} item must be a string"))
            })
            .collect()
    }

    fn owned_strings_at(value: &Value, key: &str) -> Vec<String> {
        strings_at(value, key)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    fn host_capability_strings() -> Vec<&'static str> {
        V1_HOST_CAPABILITIES
            .iter()
            .copied()
            .map(HostCapability::as_str)
            .collect()
    }

    fn serialize_enum_strings<T: Serialize + Copy>(values: &[T]) -> Vec<String> {
        values
            .iter()
            .copied()
            .map(|value| {
                serde_json::to_value(value)
                    .expect("enum must serialize")
                    .as_str()
                    .expect("enum must serialize as string")
                    .to_string()
            })
            .collect()
    }

    fn capability_params_ref<'a>(schema_def: &'a Value, capability: &str) -> Option<&'a str> {
        schema_def["allOf"].as_array()?.iter().find_map(|rule| {
            let matches_capability =
                rule["if"]["properties"]["capability"]["const"].as_str() == Some(capability);
            if matches_capability {
                rule["then"]["properties"]["params"]["$ref"].as_str()
            } else {
                None
            }
        })
    }

    fn params_ref_for_method<'a>(schema: &'a Value, method: &str) -> Option<&'a str> {
        schema["allOf"].as_array()?.iter().find_map(|rule| {
            let matches_method =
                rule["if"]["properties"]["method"]["const"].as_str() == Some(method);
            if matches_method {
                rule["then"]["properties"]["params"]["$ref"].as_str()
            } else {
                None
            }
        })
    }
}

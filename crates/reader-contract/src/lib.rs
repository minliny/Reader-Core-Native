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

pub use command::{Command, EmptyParams};
pub use config::RuntimeConfig;
pub use core_info::core_info;
pub use error::{CoreError, ErrorCode};
pub use event::Event;
pub use host::{
    HostCompleteParams, HostErrorParams, HostSmokeParams, PendingHostOperationStatus,
    RuntimeCancelParams, RuntimeShutdownParams, RuntimeStatus, RuntimeStatusParams,
};
pub use remote::{
    BookDetailParams, BookSearchParams, BookTocParams, ChapterContentParams, HostHttpRequest,
    HostHttpResponse, ReadingProgressUpdateParams, SourceImportParams,
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
}

/// Non-method capability names advertised by `core.info` in v1.
pub mod capabilities {
    pub const HOST_BUS_V1: &str = "host.bus.v1";
    /// Host-provided HTTP transport capability used by remote-reading commands.
    pub const HTTP_EXECUTE: &str = "http.execute";
    pub const RUNTIME_CONFIG_V1: &str = "runtime.config.v1";
    /// Remote-reading vertical (V1 minimal).
    pub const REMOTE_READING_V1: &str = "remote.reading.v1";
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
];

#[cfg(test)]
mod tests {
    use super::*;
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

        assert_eq!(params["type"], serde_json::json!("object"));
        assert_eq!(params["default"], serde_json::json!({}));
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
    fn event_schema_requires_result_data_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let data = &schema["$defs"]["ResultEvent"]["properties"]["data"];

        assert_eq!(data["type"], serde_json::json!("object"));
    }

    #[test]
    fn event_schema_requires_host_request_params_object() {
        let schema: Value =
            serde_json::from_str(include_str!("../../../protocol/reader-event.schema.json"))
                .expect("event schema must be valid JSON");
        let params = &schema["$defs"]["HostRequestEvent"]["properties"]["params"];

        assert_eq!(params["type"], serde_json::json!("object"));
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

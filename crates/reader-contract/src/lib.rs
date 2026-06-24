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

pub use command::Command;
pub use config::RuntimeConfig;
pub use core_info::core_info;
pub use error::{CoreError, ErrorCode};
pub use event::Event;
pub use host::{
    HostCompleteParams, HostErrorParams, HostSmokeParams, PendingHostOperationStatus,
    RuntimeCancelParams, RuntimeStatus, RuntimeStatusParams,
};
pub use remote::{
    BookDetailParams, BookSearchParams, BookTocParams, ChapterContentParams, HostHttpRequest,
    ReadingProgressUpdateParams, SourceImportParams,
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
}

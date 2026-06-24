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
pub use host::{HostCompleteParams, HostErrorParams, HostSmokeParams};
pub use remote::{
    BookDetailParams, BookSearchParams, BookTocParams, ChapterContentParams,
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
    pub const HOST_COMPLETE: &str = "host.complete";
    pub const HOST_ERROR: &str = "host.error";

    /// Bootstrap alias kept so the current FFI/Harmony smoke binaries can keep
    /// proving ABI loadability while hosts migrate to `runtime.ping`.
    pub const LEGACY_CORE_PING: &str = "core.ping";

    // --- Remote-reading vertical (V1 minimal) -------------------------------
    /// Import a remote book source definition.
    pub const SOURCE_IMPORT: &str = "source.import";
    /// Search books at a source using a pre-fetched search response.
    pub const BOOK_SEARCH: &str = "book.search";
    /// Fetch/merge book detail metadata from a pre-fetched detail response.
    pub const BOOK_DETAIL: &str = "book.detail";
    /// Fetch a book's table of contents from a pre-fetched toc response.
    pub const BOOK_TOC: &str = "book.toc";
    /// Extract chapter body text from a pre-fetched chapter response.
    pub const CHAPTER_CONTENT: &str = "chapter.content";
    /// Update reading progress/state for a book.
    pub const READING_PROGRESS_UPDATE: &str = "reading.progress.update";
}

/// Non-method capability names advertised by `core.info` in v1.
pub mod capabilities {
    pub const HOST_BUS_V1: &str = "host.bus.v1";
    pub const RUNTIME_CONFIG_V1: &str = "runtime.config.v1";
    /// Remote-reading vertical (V1 minimal, fixture/inline content only).
    pub const REMOTE_READING_V1: &str = "remote.reading.v1";
}

/// Capabilities advertised by `core.info` in v1.
pub const V1_CAPABILITIES: &[&str] = &[
    methods::CORE_INFO,
    methods::RUNTIME_PING,
    methods::RUNTIME_HOST_SMOKE,
    methods::HOST_COMPLETE,
    methods::HOST_ERROR,
    capabilities::HOST_BUS_V1,
    capabilities::RUNTIME_CONFIG_V1,
    capabilities::REMOTE_READING_V1,
];

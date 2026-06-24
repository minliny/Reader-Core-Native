//! Reader-Core JSON command/event protocol types.
//!
//! Mirrors `protocol/reader-command.schema.json` and
//! `protocol/reader-event.schema.json`. This is the single source of truth for
//! the protocol's Rust representation; ABI version lives in `reader-ffi`.
//!
//! - C ABI version: 1 (see `include/reader_core.h`)
//! - JSON protocol version: 1 ([`PROTOCOL_VERSION`])

pub mod command;
pub mod core_info;
pub mod error;
pub mod event;

pub use command::Command;
pub use core_info::core_info;
pub use error::{CoreError, ErrorCode};
pub use event::Event;

/// JSON protocol version. Bumped on non-backward-compatible schema changes.
/// See `protocol/compatibility.md`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Method names known to Core in v1.
pub mod methods {
    pub const CORE_INFO: &str = "core.info";
    pub const CORE_PING: &str = "core.ping";
    pub const HOST_COMPLETE: &str = "host.complete";
    pub const HOST_ERROR: &str = "host.error";
}

/// Capabilities advertised by `core.info` in v1.
pub const V1_CAPABILITIES: &[&str] = &[methods::CORE_INFO, methods::CORE_PING];

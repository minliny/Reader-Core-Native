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

pub use command::Command;
pub use config::RuntimeConfig;
pub use core_info::core_info;
pub use error::{CoreError, ErrorCode};
pub use event::Event;
pub use host::{HostCompleteParams, HostErrorParams, HostSmokeParams};

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
}

/// Non-method capability names advertised by `core.info` in v1.
pub mod capabilities {
    pub const HOST_BUS_V1: &str = "host.bus.v1";
    pub const RUNTIME_CONFIG_V1: &str = "runtime.config.v1";
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
];

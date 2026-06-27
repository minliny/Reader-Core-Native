//! Reader-Core runtime — command dispatcher, worker thread, cancellation.
//!
//! First-version concurrency model (per ARCHITECTURE.md §5.1):
//! - a single Core worker thread driven by an mpsc channel;
//! - per-request cancellation via a shared cancelled-id set;
//! - events delivered through the [`EventSink`] trait, implemented by the
//!   FFI layer (and by `reader-cli` / tests).
//!
//! Tokio is intentionally NOT a dependency in v1 to keep the OHOS toolchain
//! risk surface minimal.

pub mod remote;
pub mod runtime;
pub mod sink;
pub mod webdav_bridge;

pub use reader_contract::CoreError;
pub use remote::RemoteState;
pub use runtime::Runtime;
pub use sink::EventSink;

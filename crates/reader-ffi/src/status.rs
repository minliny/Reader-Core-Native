//! Stable coarse status codes returned by the C ABI entry points.
//!
//! These numeric values mirror `include/reader_core.h` and are frozen for
//! ABI v1. Keep the namespaces per entry point: the same integer can carry a
//! different meaning for `create` and `send`.

pub const PANIC: i32 = -1;
pub const OK: i32 = 0;

pub mod create {
    pub const OK: i32 = super::OK;
    pub const NULL_OUT_RUNTIME: i32 = 2;
    pub const NULL_CALLBACK: i32 = 3;
    pub const INVALID_CONFIG: i32 = 4;
}

pub mod send {
    pub const OK: i32 = super::OK;
    pub const NULL_RUNTIME: i32 = 1;
    pub const NULL_COMMAND: i32 = 2;
    pub const INVALID_COMMAND: i32 = 3;
    pub const PROTOCOL_ERROR: i32 = 4;
}

pub mod cancel {
    pub const OK: i32 = super::OK;
    pub const NULL_RUNTIME: i32 = 1;
}

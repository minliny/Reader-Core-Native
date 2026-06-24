use serde_json::{json, Value};

use crate::{PROTOCOL_VERSION, V1_CAPABILITIES};

/// Build the `data` object returned by the `core.info` method.
///
/// `abi_version` is supplied by the caller (it lives with the C ABI in
/// `reader-ffi`) so this crate stays free of FFI concerns.
pub fn core_info(abi_version: u32, build_version: &str) -> Value {
    json!({
        "abiVersion": abi_version,
        "protocolVersion": PROTOCOL_VERSION,
        "buildVersion": build_version,
        "capabilities": V1_CAPABILITIES,
    })
}

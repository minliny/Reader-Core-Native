use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PROTOCOL_VERSION;

/// A platform → Core command. Mirrors `reader-command.schema.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,

    #[serde(rename = "requestId")]
    pub request_id: u64,

    pub method: String,

    /// Method-specific parameters. Free-form in v1; per-method schemas land
    /// before ARCHITECTURE phase 4.
    #[serde(default)]
    pub params: Value,
}

impl Command {
    /// Build a command with the current protocol version prefilled.
    pub fn new(request_id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            method: method.into(),
            params,
        }
    }
}

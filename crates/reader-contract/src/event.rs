use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CoreError;
use crate::PROTOCOL_VERSION;

/// Core → platform event. Mirrors `reader-event.schema.json`.
///
/// Discriminated by the `type` field (`"result"` / `"error"` / `"host.request"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "result")]
    Result {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "requestId")]
        request_id: u64,
        data: Value,
    },

    #[serde(rename = "error")]
    Error {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "requestId")]
        request_id: u64,
        error: CoreError,
    },

    #[serde(rename = "host.request")]
    HostRequest {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "operationId")]
        operation_id: u64,
        capability: String,
        params: Value,
    },
}

impl Event {
    /// Build a `result` event for the given request.
    pub fn result(request_id: u64, data: Value) -> Self {
        Event::Result {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            data,
        }
    }

    /// Build an `error` event for the given request.
    pub fn error(request_id: u64, error: CoreError) -> Self {
        Event::Error {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            error,
        }
    }
}

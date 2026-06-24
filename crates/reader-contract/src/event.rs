use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CoreError;
use crate::PROTOCOL_VERSION;

/// Core → platform event. Mirrors `reader-event.schema.json`.
///
/// Discriminated by the `type` field (`"result"` / `"error"` / `"host.request"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        #[serde(rename = "requestId")]
        request_id: u64,
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

    /// Build a `host.request` event linked to the originating command.
    pub fn host_request(
        request_id: u64,
        operation_id: u64,
        capability: impl Into<String>,
        params: Value,
    ) -> Self {
        Event::HostRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            operation_id,
            capability: capability.into(),
            params,
        }
    }
}

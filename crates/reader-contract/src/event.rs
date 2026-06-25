use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CoreError;
use crate::PROTOCOL_VERSION;

fn assert_object_payload(field: &str, value: &Value) {
    assert!(value.is_object(), "{field} must be a JSON object");
}

fn assert_positive_id(field: &str, value: u64) {
    assert!(value > 0, "{field} must be greater than 0");
}

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
        assert_object_payload("result.data", &data);
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
        assert_positive_id("host.request operationId", operation_id);
        assert_object_payload("host.request params", &params);
        Event::HostRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            operation_id,
            capability: capability.into(),
            params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_event_accepts_object_data() {
        let event = Event::result(1, serde_json::json!({ "ok": true }));
        let json = serde_json::to_value(event).expect("event must serialize");

        assert_eq!(json["type"], "result");
        assert!(json["data"].is_object());
        assert_eq!(json["data"]["ok"], true);
    }

    #[test]
    #[should_panic(expected = "result.data must be a JSON object")]
    fn result_event_rejects_non_object_data() {
        let _event = Event::result(1, serde_json::json!(["not", "an", "object"]));
    }

    #[test]
    fn host_request_event_accepts_object_params() {
        let event = Event::host_request(1, 2, "host.smoke.echo", serde_json::json!({ "ok": true }));
        let json = serde_json::to_value(event).expect("event must serialize");

        assert_eq!(json["type"], "host.request");
        assert_eq!(json["operationId"], 2);
        assert!(json["params"].is_object());
        assert_eq!(json["params"]["ok"], true);
    }

    #[test]
    #[should_panic(expected = "host.request operationId must be greater than 0")]
    fn host_request_event_rejects_zero_operation_id() {
        let _event = Event::host_request(1, 0, "host.smoke.echo", serde_json::json!({}));
    }

    #[test]
    #[should_panic(expected = "host.request params must be a JSON object")]
    fn host_request_event_rejects_non_object_params() {
        let _event = Event::host_request(
            1,
            2,
            "host.smoke.echo",
            serde_json::json!(["not", "an", "object"]),
        );
    }
}

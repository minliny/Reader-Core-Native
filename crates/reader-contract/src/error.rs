use serde::{de, Deserialize, Deserializer, Serialize};

fn deserialize_error_details<'de, D>(deserializer: D) -> Result<serde_json::Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(de::Error::custom("CoreError details must be a JSON object"))
    }
}

/// Machine-readable error codes. Serialized as `SCREAMING_SNAKE_CASE` strings
/// to match the `error.code` field in `reader-event.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// `method` is not recognized by this Core build.
    UnknownMethod,
    /// `params` failed method-specific validation.
    InvalidParams,
    /// `protocolVersion` is not supported by this Core.
    InvalidProtocolVersion,
    /// Request was cancelled via `rc_runtime_cancel` before completion.
    Cancelled,
    /// Malformed JSON or message structure.
    InvalidMessage,
    /// Catch-all for unexpected internal failures.
    Internal,
}

/// A structured Core error. Mirrors the `error` object in
/// `reader-event.schema.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        deserialize_with = "deserialize_error_details"
    )]
    pub details: serde_json::Value,
}

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            details: serde_json::Value::Null,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    pub fn unknown_method(method: &str) -> Self {
        Self::new(
            ErrorCode::UnknownMethod,
            format!("unknown method: {method}"),
            false,
        )
    }

    pub fn invalid_message(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidMessage, message, false)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, message, false)
    }

    pub fn invalid_protocol_version(version: u32) -> Self {
        Self::new(
            ErrorCode::InvalidProtocolVersion,
            format!("unsupported protocolVersion: {version}"),
            false,
        )
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCode::Cancelled, "request cancelled", false)
    }

    pub fn host_operation_not_found(operation_id: u64) -> Self {
        Self::new(
            ErrorCode::InvalidParams,
            format!("unknown host operationId: {operation_id}"),
            false,
        )
        .with_details(serde_json::json!({ "operationId": operation_id }))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_accepts_missing_or_object_details() {
        let without_details: CoreError = serde_json::from_value(serde_json::json!({
            "code": "INTERNAL",
            "message": "host failed",
            "retryable": true
        }))
        .unwrap();
        assert!(without_details.details.is_null());

        let with_details: CoreError = serde_json::from_value(serde_json::json!({
            "code": "INTERNAL",
            "message": "host failed",
            "retryable": true,
            "details": { "phase": "host" }
        }))
        .unwrap();
        assert_eq!(with_details.details["phase"], "host");
    }

    #[test]
    fn core_error_rejects_non_object_details() {
        for details in [
            serde_json::json!(null),
            serde_json::json!(["not", "an", "object"]),
            serde_json::json!("not an object"),
        ] {
            let err = serde_json::from_value::<CoreError>(serde_json::json!({
                "code": "INTERNAL",
                "message": "host failed",
                "retryable": true,
                "details": details
            }))
            .unwrap_err();
            assert!(
                err.to_string().contains("details"),
                "unexpected details error: {err}"
            );
        }
    }

    #[test]
    fn core_error_rejects_unknown_fields() {
        let err = serde_json::from_value::<CoreError>(serde_json::json!({
            "code": "INTERNAL",
            "message": "host failed",
            "retryable": true,
            "extra": true
        }))
        .unwrap_err();

        assert!(
            err.to_string().contains("unknown field"),
            "unexpected CoreError unknown-field error: {err}"
        );
    }
}

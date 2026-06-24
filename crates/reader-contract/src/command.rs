use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{CoreError, PROTOCOL_VERSION};

fn default_params() -> Value {
    Value::Object(Default::default())
}

/// A platform → Core command. Mirrors `reader-command.schema.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,

    #[serde(rename = "requestId")]
    pub request_id: u64,

    pub method: String,

    /// Method-specific parameters. Free-form in v1; per-method schemas land
    /// before ARCHITECTURE phase 4.
    #[serde(default = "default_params")]
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

    /// Parse and validate a command JSON payload.
    ///
    /// This is intentionally stricter than `serde_json::from_slice`: unknown
    /// top-level fields and non-object `params` values become structured Core
    /// errors instead of being silently ignored.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        let command: Self = serde_json::from_slice(bytes).map_err(|err| {
            CoreError::invalid_message("invalid command JSON").with_details(json!({
                "source": err.to_string(),
            }))
        })?;
        command.validate()?;
        Ok(command)
    }

    /// Validate protocol-version and top-level shape checks that are common to
    /// every method.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(CoreError::invalid_protocol_version(self.protocol_version));
        }
        if !self.params.is_object() {
            return Err(
                CoreError::invalid_params("command params must be a JSON object")
                    .with_details(json!({ "method": self.method })),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods;

    #[test]
    fn rejects_unknown_top_level_field() {
        let err = Command::from_json_bytes(
            br#"{"protocolVersion":1,"requestId":1,"method":"runtime.ping","params":{},"extra":true}"#,
        )
        .unwrap_err();
        assert_eq!(err.code, crate::ErrorCode::InvalidMessage);
    }

    #[test]
    fn defaults_missing_params_to_object() {
        let command = Command::from_json_bytes(
            br#"{"protocolVersion":1,"requestId":1,"method":"runtime.ping"}"#,
        )
        .unwrap();
        assert_eq!(command.method, methods::RUNTIME_PING);
        assert_eq!(command.params, serde_json::json!({}));
    }
}

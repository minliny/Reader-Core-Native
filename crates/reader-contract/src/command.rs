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
        if self.request_id == 0 {
            return Err(CoreError::invalid_message(
                "command requestId must be greater than 0",
            ));
        }
        if self.method.trim().is_empty() {
            return Err(CoreError::invalid_message(
                "command method must be a non-empty string",
            ));
        }
        if !is_valid_token_path(&self.method) {
            return Err(CoreError::invalid_message(
                "command method must be dot-separated non-empty tokens without whitespace",
            )
            .with_details(json!({ "method": self.method })));
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

pub(crate) fn is_valid_token_path(value: &str) -> bool {
    if value.chars().any(char::is_whitespace) || !value.contains('.') {
        return false;
    }
    value.split('.').all(|segment| !segment.is_empty())
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

    #[test]
    fn conformance_valid_command_fixtures_parse() {
        for (name, json) in [
            (
                "valid-runtime-ping",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-runtime-ping.json"
                ),
            ),
            (
                "valid-runtime-cancel",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
                ),
            ),
            (
                "valid-runtime-status",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-runtime-status.json"
                ),
            ),
            (
                "valid-core-info",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-core-info.json"
                ),
            ),
            (
                "valid-runtime-cancel",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
                ),
            ),
            (
                "valid-runtime-status",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-runtime-status.json"
                ),
            ),
            (
                "valid-runtime-shutdown",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/valid-runtime-shutdown.json"
                ),
            ),
        ] {
            Command::from_json_bytes(json.as_bytes())
                .unwrap_or_else(|err| panic!("{name} should parse, got {err:?}"));
        }
    }

    #[test]
    fn conformance_invalid_command_fixtures_return_expected_codes() {
        for (name, json, expected) in [
            (
                "invalid-malformed-json",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-malformed-json.json"
                ),
                crate::ErrorCode::InvalidMessage,
            ),
            (
                "invalid-unsupported-protocol",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-unsupported-protocol.json"
                ),
                crate::ErrorCode::InvalidProtocolVersion,
            ),
            (
                "invalid-missing-request-id",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-missing-request-id.json"
                ),
                crate::ErrorCode::InvalidMessage,
            ),
            (
                "invalid-request-id-zero",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-request-id-zero.json"
                ),
                crate::ErrorCode::InvalidMessage,
            ),
            (
                "invalid-empty-method",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-empty-method.json"
                ),
                crate::ErrorCode::InvalidMessage,
            ),
            (
                "invalid-method-whitespace",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-method-whitespace.json"
                ),
                crate::ErrorCode::InvalidMessage,
            ),
            (
                "invalid-method-empty-segment",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-method-empty-segment.json"
                ),
                crate::ErrorCode::InvalidMessage,
            ),
            (
                "invalid-params-not-object",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-params-not-object.json"
                ),
                crate::ErrorCode::InvalidParams,
            ),
        ] {
            let err = match Command::from_json_bytes(json.as_bytes()) {
                Ok(_) => panic!("{name} should be rejected"),
                Err(err) => err,
            };
            assert_eq!(err.code, expected, "{name} returned {err:?}");
        }
    }
}

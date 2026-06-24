use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::CoreError;

/// Runtime creation config supplied by hosts.
///
/// The C ABI receives this as JSON, but the schema is owned by the protocol
/// contract so every host validates the same field names and error behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_directory: Option<String>,
}

impl RuntimeConfig {
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let config: Self = serde_json::from_slice(bytes).map_err(|err| {
            CoreError::invalid_message("invalid runtime config JSON").with_details(json!({
                "source": err.to_string(),
            }))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        validate_optional_directory("dataDirectory", &self.data_directory)?;
        validate_optional_directory("cacheDirectory", &self.cache_directory)?;
        Ok(())
    }
}

fn validate_optional_directory(field: &str, value: &Option<String>) -> Result<(), CoreError> {
    if let Some(path) = value {
        if path.trim().is_empty() {
            return Err(
                CoreError::invalid_params(format!("{field} must not be empty"))
                    .with_details(json!({ "field": field })),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_runtime_directories() {
        let config = RuntimeConfig::from_json_bytes(
            br#"{"dataDirectory":"/tmp/reader-data","cacheDirectory":"/tmp/reader-cache"}"#,
        )
        .unwrap();
        assert_eq!(config.data_directory.as_deref(), Some("/tmp/reader-data"));
        assert_eq!(config.cache_directory.as_deref(), Some("/tmp/reader-cache"));
    }

    #[test]
    fn rejects_unknown_runtime_config_field() {
        let err = RuntimeConfig::from_json_bytes(br#"{"dataDirectory":"/tmp","extra":true}"#)
            .unwrap_err();
        assert_eq!(err.code, crate::ErrorCode::InvalidMessage);
    }
}

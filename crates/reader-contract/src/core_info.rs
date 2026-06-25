use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::{PROTOCOL_VERSION, V1_CAPABILITIES};

fn deserialize_core_info_protocol_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value == PROTOCOL_VERSION {
        Ok(value)
    } else {
        Err(de::Error::custom(format!(
            "core.info protocolVersion must be {PROTOCOL_VERSION}"
        )))
    }
}

fn deserialize_core_info_abi_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value > 0 {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "core.info abiVersion must be greater than 0",
        ))
    }
}

fn deserialize_core_info_capabilities<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    if values
        .iter()
        .map(String::as_str)
        .eq(V1_CAPABILITIES.iter().copied())
    {
        Ok(values)
    } else {
        Err(de::Error::custom(
            "core.info capabilities must exactly match the V1 capability list",
        ))
    }
}

/// Result data for `core.info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreInfoData {
    #[serde(deserialize_with = "deserialize_core_info_abi_version")]
    pub abi_version: u32,
    #[serde(deserialize_with = "deserialize_core_info_protocol_version")]
    pub protocol_version: u32,
    pub build_version: String,
    #[serde(deserialize_with = "deserialize_core_info_capabilities")]
    pub capabilities: Vec<String>,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_info_data_parses_builder_output_and_rejects_invalid_shape() {
        let data: CoreInfoData = serde_json::from_value(core_info(1, "test-build")).unwrap();
        assert_eq!(data.abi_version, 1);
        assert_eq!(data.protocol_version, PROTOCOL_VERSION);
        assert_eq!(data.build_version, "test-build");
        assert_eq!(
            data.capabilities,
            V1_CAPABILITIES
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        );

        for (label, value, expected) in [
            (
                "abiVersion",
                json!({
                    "abiVersion": 0,
                    "protocolVersion": PROTOCOL_VERSION,
                    "buildVersion": "test-build",
                    "capabilities": V1_CAPABILITIES
                }),
                "abiVersion",
            ),
            (
                "protocolVersion",
                json!({
                    "abiVersion": 1,
                    "protocolVersion": PROTOCOL_VERSION + 1,
                    "buildVersion": "test-build",
                    "capabilities": V1_CAPABILITIES
                }),
                "protocolVersion",
            ),
            (
                "capabilities",
                json!({
                    "abiVersion": 1,
                    "protocolVersion": PROTOCOL_VERSION,
                    "buildVersion": "test-build",
                    "capabilities": ["runtime.ping"]
                }),
                "capabilities",
            ),
            (
                "unknown field",
                json!({
                    "abiVersion": 1,
                    "protocolVersion": PROTOCOL_VERSION,
                    "buildVersion": "test-build",
                    "capabilities": V1_CAPABILITIES,
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<CoreInfoData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected core.info data error for {label}: {err}"
            );
        }
    }
}

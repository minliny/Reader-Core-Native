use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CoreError;

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn default_smoke_capability() -> String {
    "host.smoke.echo".to_string()
}

/// Parameters for `runtime.hostSmoke`, a local driver method that exercises the
/// host bus without involving reader business modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSmokeParams {
    #[serde(default = "default_smoke_capability")]
    pub capability: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

/// Parameters sent by the host to complete a pending `host.request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCompleteParams {
    pub operation_id: u64,
    pub result: Value,
}

/// Parameters sent by the host to fail a pending `host.request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostErrorParams {
    pub operation_id: u64,
    pub error: CoreError,
}

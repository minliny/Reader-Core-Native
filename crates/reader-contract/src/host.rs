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

/// Parameters for `runtime.cancel`, the JSON-protocol counterpart of the C ABI
/// `rc_runtime_cancel`. Lets a host driving Core purely over the JSON protocol
/// cancel an in-flight request by its `requestId`.
///
/// Result data: `{ "cancelled": <bool> }` — `true` if the target was active
/// and got cancelled (either immediately, when blocked on a host operation, or
/// marked for cancellation while still queued), `false` if the target was
/// unknown or already completed. The cancelled original request receives a
/// separate `CANCELLED` error event on its own `requestId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCancelParams {
    /// The requestId to cancel. Must differ from the `runtime.cancel` command's
    /// own requestId (self-cancellation is rejected with `INVALID_PARAMS`).
    pub request_id: u64,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[test]
    fn conformance_host_param_fixtures_parse() {
        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/complete.json").as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        assert_eq!(complete.operation_id, 1);
        assert_eq!(complete.result["status"], "ok");

        let error_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/error.json").as_bytes(),
        )
        .unwrap();
        let error: HostErrorParams = serde_json::from_value(error_command.params).unwrap();
        assert_eq!(error.operation_id, 1);
        assert_eq!(error.error.code, ErrorCode::Internal);
    }

    #[test]
    fn runtime_cancel_params_parse_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: RuntimeCancelParams = serde_json::from_value(command.params).unwrap();
        assert_eq!(params.request_id, 301);

        let err = serde_json::from_value::<RuntimeCancelParams>(serde_json::json!({
            "requestId": 1,
            "extra": true
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }
}

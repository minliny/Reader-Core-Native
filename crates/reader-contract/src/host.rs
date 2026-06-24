use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::is_valid_token_path;
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

impl HostSmokeParams {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !is_valid_token_path(&self.capability) {
            return Err(CoreError::invalid_params(
                "runtime.hostSmoke capability must be dot-separated non-empty tokens without whitespace",
            )
            .with_details(serde_json::json!({ "capability": self.capability })));
        }
        Ok(())
    }
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

impl RuntimeCancelParams {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.request_id == 0 {
            return Err(CoreError::invalid_params(
                "runtime.cancel requestId must be greater than 0",
            )
            .with_details(serde_json::json!({ "requestId": self.request_id })));
        }
        Ok(())
    }
}

/// Parameters for `runtime.status`.
///
/// The command is read-only and accepts no method-specific fields. It gives
/// hosts a protocol-level snapshot of request/host-operation liveness without
/// exposing host payloads or platform state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStatusParams {}

/// One pending host operation reported by `runtime.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingHostOperationStatus {
    pub operation_id: u64,
    pub request_id: u64,
    pub capability: String,
    pub state: String,
}

/// Result data for `runtime.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStatus {
    pub active_request_count: u64,
    pub active_request_ids: Vec<u64>,
    pub pending_host_operation_count: u64,
    pub pending_host_operations: Vec<PendingHostOperationStatus>,
    pub shutting_down: bool,
}

/// Parameters sent by the host to complete a pending `host.request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCompleteParams {
    pub operation_id: u64,
    pub result: Value,
}

impl HostCompleteParams {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.operation_id == 0 {
            return Err(CoreError::invalid_params(
                "host.complete operationId must be greater than 0",
            )
            .with_details(serde_json::json!({ "operationId": self.operation_id })));
        }
        Ok(())
    }
}

/// Parameters sent by the host to fail a pending `host.request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostErrorParams {
    pub operation_id: u64,
    pub error: CoreError,
}

impl HostErrorParams {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.operation_id == 0 {
            return Err(
                CoreError::invalid_params("host.error operationId must be greater than 0")
                    .with_details(serde_json::json!({ "operationId": self.operation_id })),
            );
        }
        Ok(())
    }
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
    fn host_smoke_capability_accepts_token_path_and_rejects_malformed_names() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/request.json").as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
        params.validate().unwrap();

        for (name, json) in [
            (
                "whitespace",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/request-invalid-capability-whitespace.json"
                ),
            ),
            (
                "empty-segment",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/request-invalid-capability-empty-segment.json"
                ),
            ),
        ] {
            let command = crate::Command::from_json_bytes(json.as_bytes()).unwrap();
            let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
            let err = match params.validate() {
                Ok(()) => panic!("{name} should reject malformed capability"),
                Err(err) => err,
            };
            assert_eq!(err.code, ErrorCode::InvalidParams);
            assert!(err.message.contains("capability"));
        }
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
        params.validate().unwrap();

        let err = serde_json::from_value::<RuntimeCancelParams>(serde_json::json!({
            "requestId": 1,
            "extra": true
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn runtime_cancel_rejects_zero_target_request_id() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-target-zero.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: RuntimeCancelParams = serde_json::from_value(command.params).unwrap();
        let err = params.validate().unwrap_err();

        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["requestId"], 0);
    }

    #[test]
    fn host_completion_params_reject_zero_operation_id() {
        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/complete-operation-zero.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let err = complete.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["operationId"], 0);

        let error_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/error-operation-zero.json")
                .as_bytes(),
        )
        .unwrap();
        let error: HostErrorParams = serde_json::from_value(error_command.params).unwrap();
        let err = error.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["operationId"], 0);
    }

    #[test]
    fn runtime_status_params_accept_empty_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-runtime-status.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let _params: RuntimeStatusParams = serde_json::from_value(command.params).unwrap();

        let err = serde_json::from_value::<RuntimeStatusParams>(serde_json::json!({
            "includePayloads": true
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }
}

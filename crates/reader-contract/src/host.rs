use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::command::is_valid_token_path;
use crate::{methods, CoreError};

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn default_smoke_capability() -> String {
    "host.smoke.echo".to_string()
}

fn deserialize_host_smoke_params<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "runtime.hostSmoke params.params must be a JSON object",
        ))
    }
}

fn deserialize_host_complete_result<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "host.complete result must be a JSON object",
        ))
    }
}

fn deserialize_runtime_ping_pong<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = bool::deserialize(deserializer)?;
    if value {
        Ok(value)
    } else {
        Err(de::Error::custom("runtime.ping pong must be true"))
    }
}

fn deserialize_runtime_ping_method<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == methods::RUNTIME_PING {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "runtime.ping method must be runtime.ping",
        ))
    }
}

fn deserialize_pending_host_operation_state<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == "pending" {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "runtime.status pending host operation state must be pending",
        ))
    }
}

fn deserialize_positive_pending_host_operation_id<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    if value > 0 {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "runtime.status pending host operation ids must be greater than 0",
        ))
    }
}

fn deserialize_pending_host_operation_capability<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if is_valid_token_path(&value) {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "runtime.status pending host operation capability must be dot-separated non-empty tokens without whitespace",
        ))
    }
}

fn deserialize_runtime_shutdown_shutting_down<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = bool::deserialize(deserializer)?;
    if value {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "runtime.shutdown shuttingDown must be true",
        ))
    }
}

fn deserialize_positive_runtime_shutdown_cancelled_request_ids<'de, D>(
    deserializer: D,
) -> Result<Vec<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<u64>::deserialize(deserializer)?;
    if values.iter().all(|request_id| *request_id > 0) {
        Ok(values)
    } else {
        Err(de::Error::custom(
            "runtime.shutdown cancelledRequestIds items must be greater than 0",
        ))
    }
}

fn deserialize_positive_runtime_status_active_request_ids<'de, D>(
    deserializer: D,
) -> Result<Vec<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<u64>::deserialize(deserializer)?;
    if values.iter().all(|request_id| *request_id > 0) {
        Ok(values)
    } else {
        Err(de::Error::custom(
            "runtime.status activeRequestIds items must be greater than 0",
        ))
    }
}

/// Parameters for `runtime.hostSmoke`, a local driver method that exercises the
/// host bus without involving reader business modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSmokeParams {
    #[serde(default = "default_smoke_capability")]
    pub capability: String,
    #[serde(
        default = "empty_object",
        deserialize_with = "deserialize_host_smoke_params"
    )]
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

/// Result data for `runtime.ping`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimePingData {
    #[serde(deserialize_with = "deserialize_runtime_ping_pong")]
    pub pong: bool,
    #[serde(deserialize_with = "deserialize_runtime_ping_method")]
    pub method: String,
}

/// Parameters for `runtime.cancel`, the JSON-protocol counterpart of the C ABI
/// `rc_runtime_cancel`. Lets a host driving Core purely over the JSON protocol
/// cancel an in-flight request by its `requestId`.
///
/// Result data: `{ "cancelled": <bool> }` - `true` if the target was active
/// and got cancelled, `false` if the target was unknown or already completed.
/// The cancelled original request receives a separate `CANCELLED` error event
/// on its own `requestId`.
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

/// Result data for `runtime.cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCancelData {
    pub cancelled: bool,
}

/// Parameters for `runtime.status`.
///
/// The command is read-only and accepts no method-specific fields. It gives
/// hosts a protocol-level snapshot of request/host-operation liveness without
/// exposing host payloads or platform state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStatusParams {}

/// Parameters for `runtime.shutdown`.
///
/// The command accepts no method-specific fields. A valid shutdown request asks
/// Core to reject future commands, cancel other active requests, and finish the
/// worker lifecycle after emitting the shutdown result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeShutdownParams {}

/// Result data for `runtime.shutdown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeShutdownData {
    #[serde(deserialize_with = "deserialize_runtime_shutdown_shutting_down")]
    pub shutting_down: bool,
    #[serde(deserialize_with = "deserialize_positive_runtime_shutdown_cancelled_request_ids")]
    pub cancelled_request_ids: Vec<u64>,
}

/// One pending host operation reported by `runtime.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingHostOperationStatus {
    #[serde(deserialize_with = "deserialize_positive_pending_host_operation_id")]
    pub operation_id: u64,
    #[serde(deserialize_with = "deserialize_positive_pending_host_operation_id")]
    pub request_id: u64,
    #[serde(deserialize_with = "deserialize_pending_host_operation_capability")]
    pub capability: String,
    #[serde(deserialize_with = "deserialize_pending_host_operation_state")]
    pub state: String,
}

/// Result data for `runtime.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStatus {
    pub active_request_count: u64,
    #[serde(deserialize_with = "deserialize_positive_runtime_status_active_request_ids")]
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
    #[serde(deserialize_with = "deserialize_host_complete_result")]
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
    fn runtime_ping_data_requires_true_pong_and_runtime_ping_method() {
        let data: RuntimePingData = serde_json::from_value(serde_json::json!({
            "pong": true,
            "method": "runtime.ping"
        }))
        .unwrap();
        assert!(data.pong);
        assert_eq!(data.method, methods::RUNTIME_PING);

        for (label, value, expected) in [
            (
                "pong",
                serde_json::json!({
                    "pong": false,
                    "method": "runtime.ping"
                }),
                "pong",
            ),
            (
                "method",
                serde_json::json!({
                    "pong": true,
                    "method": "core.ping"
                }),
                "method",
            ),
            (
                "unknown field",
                serde_json::json!({
                    "pong": true,
                    "method": "runtime.ping",
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<RuntimePingData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected runtime.ping data error for {label}: {err}"
            );
        }
    }

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

        let command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/request-unknown-field.json")
                .as_bytes(),
        )
        .unwrap();
        let err = serde_json::from_value::<HostSmokeParams>(command.params).unwrap_err();
        assert!(err.to_string().contains("unknown field"));

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/request-params-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = serde_json::from_value::<HostSmokeParams>(command.params).unwrap_err();
        assert!(
            err.to_string().contains("params.params"),
            "unexpected hostSmoke params error: {err}"
        );
    }

    #[test]
    fn host_completion_params_reject_zero_operation_id_and_unknown_fields() {
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

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/complete-unknown-field.json")
                .as_bytes(),
        )
        .unwrap();
        let err =
            serde_json::from_value::<HostCompleteParams>(complete_command.params).unwrap_err();
        assert!(err.to_string().contains("unknown field"));

        let complete_command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/complete-result-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            serde_json::from_value::<HostCompleteParams>(complete_command.params).unwrap_err();
        assert!(
            err.to_string().contains("host.complete result"),
            "unexpected host.complete result error: {err}"
        );

        let error_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/error-unknown-field.json")
                .as_bytes(),
        )
        .unwrap();
        let err = serde_json::from_value::<HostErrorParams>(error_command.params).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
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

        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = serde_json::from_value::<RuntimeCancelParams>(command.params).unwrap_err();
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
    fn runtime_cancel_data_accepts_boolean_result_and_rejects_invalid_shape() {
        for cancelled in [false, true] {
            let data: RuntimeCancelData = serde_json::from_value(serde_json::json!({
                "cancelled": cancelled
            }))
            .unwrap();
            assert_eq!(data.cancelled, cancelled);
        }

        for (label, value, expected) in [
            (
                "non-boolean",
                serde_json::json!({ "cancelled": "yes" }),
                "boolean",
            ),
            (
                "unknown field",
                serde_json::json!({
                    "cancelled": true,
                    "requestId": 301
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<RuntimeCancelData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected runtime.cancel data error for {label}: {err}"
            );
        }
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

        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-runtime-status-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = serde_json::from_value::<RuntimeStatusParams>(command.params).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn runtime_shutdown_params_accept_empty_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-runtime-shutdown.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let _params: RuntimeShutdownParams = serde_json::from_value(command.params).unwrap();

        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-runtime-shutdown-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = serde_json::from_value::<RuntimeShutdownParams>(command.params).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn runtime_shutdown_data_requires_true_shutdown_and_positive_cancelled_ids() {
        for cancelled_request_ids in [serde_json::json!([]), serde_json::json!([301])] {
            let data: RuntimeShutdownData = serde_json::from_value(serde_json::json!({
                "shuttingDown": true,
                "cancelledRequestIds": cancelled_request_ids
            }))
            .unwrap();
            assert!(data.shutting_down);
            assert!(data
                .cancelled_request_ids
                .iter()
                .all(|request_id| *request_id > 0));
        }

        let err = serde_json::from_value::<RuntimeShutdownData>(serde_json::json!({
            "shuttingDown": false,
            "cancelledRequestIds": []
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("shuttingDown"),
            "unexpected runtime.shutdown shuttingDown error: {err}"
        );

        let err = serde_json::from_value::<RuntimeShutdownData>(serde_json::json!({
            "shuttingDown": true,
            "cancelledRequestIds": [0]
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("cancelledRequestIds"),
            "unexpected runtime.shutdown cancelledRequestIds error: {err}"
        );
    }

    #[test]
    fn runtime_status_requires_positive_active_request_ids() {
        let status: RuntimeStatus = serde_json::from_value(serde_json::json!({
            "activeRequestCount": 1,
            "activeRequestIds": [301],
            "pendingHostOperationCount": 0,
            "pendingHostOperations": [],
            "shuttingDown": false
        }))
        .unwrap();
        assert_eq!(status.active_request_ids, vec![301]);

        let err = serde_json::from_value::<RuntimeStatus>(serde_json::json!({
            "activeRequestCount": 1,
            "activeRequestIds": [0],
            "pendingHostOperationCount": 0,
            "pendingHostOperations": [],
            "shuttingDown": false
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("activeRequestIds"),
            "unexpected runtime.status activeRequestIds error: {err}"
        );
    }

    #[test]
    fn pending_host_operation_status_requires_pending_state() {
        let status: PendingHostOperationStatus = serde_json::from_value(serde_json::json!({
            "operationId": 1,
            "requestId": 301,
            "capability": "host.smoke.echo",
            "state": "pending"
        }))
        .unwrap();
        assert_eq!(status.state, "pending");

        let err = serde_json::from_value::<PendingHostOperationStatus>(serde_json::json!({
            "operationId": 1,
            "requestId": 301,
            "capability": "host.smoke.echo",
            "state": "completed"
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("state"),
            "unexpected pending operation state error: {err}"
        );
    }

    #[test]
    fn pending_host_operation_status_requires_positive_ids() {
        for (field, json) in [
            (
                "operationId",
                serde_json::json!({
                    "operationId": 0,
                    "requestId": 301,
                    "capability": "host.smoke.echo",
                    "state": "pending"
                }),
            ),
            (
                "requestId",
                serde_json::json!({
                    "operationId": 1,
                    "requestId": 0,
                    "capability": "host.smoke.echo",
                    "state": "pending"
                }),
            ),
        ] {
            let err = serde_json::from_value::<PendingHostOperationStatus>(json).unwrap_err();
            assert!(
                err.to_string().contains(field) || err.to_string().contains("ids"),
                "unexpected pending operation id error for {field}: {err}"
            );
        }
    }

    #[test]
    fn pending_host_operation_status_requires_capability_token_path() {
        for capability in ["", "host. smoke.echo", "host..echo", "host"] {
            let err = serde_json::from_value::<PendingHostOperationStatus>(serde_json::json!({
                "operationId": 1,
                "requestId": 301,
                "capability": capability,
                "state": "pending"
            }))
            .unwrap_err();
            assert!(
                err.to_string().contains("capability"),
                "unexpected pending operation capability error: {err}"
            );
        }
    }
}

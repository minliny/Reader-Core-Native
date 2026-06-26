use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::command::is_valid_token_path;
use crate::{methods, CoreError};

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(|object| object.is_empty())
}

fn validate_optional_non_blank(value: Option<&str>, field: &'static str) -> Result<(), String> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        Err(format!("{field} must not be blank"))
    } else {
        Ok(())
    }
}

fn validate_required_non_blank(value: &str, field: &'static str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be blank"))
    } else {
        Ok(())
    }
}

fn validate_optional_positive(value: Option<u64>, field: &'static str) -> Result<(), String> {
    if value.is_some_and(|value| value == 0) {
        Err(format!("{field} must be greater than 0"))
    } else {
        Ok(())
    }
}

fn validate_object_value(value: &Value, field: &'static str) -> Result<(), String> {
    if value.is_object() {
        Ok(())
    } else {
        Err(format!("{field} must be a JSON object"))
    }
}

fn validate_exactly_one_string_payload(
    first: Option<&String>,
    second: Option<&String>,
    first_field: &'static str,
    second_field: &'static str,
    label: &'static str,
) -> Result<(), String> {
    match (first, second) {
        (Some(_), None) | (None, Some(_)) => Ok(()),
        (None, None) => Err(format!("{label} requires {first_field} or {second_field}")),
        (Some(_), Some(_)) => Err(format!(
            "{label} must not include both {first_field} and {second_field}"
        )),
    }
}

fn default_smoke_capability() -> HostCapability {
    HostCapability::HostSmokeEcho
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

fn deserialize_host_error_diagnostic_details<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "host.error diagnostics.details must be a JSON object",
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

/// Host-owned capability Core may request through `host.request`.
///
/// The JSON representation is the stable dot-path string. Keeping the Rust type
/// closed prevents platform adapters from treating arbitrary strings as Core
/// business semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HostCapability {
    HostSmokeEcho,
    HttpExecute,
    CookieGet,
    CookieSet,
    WebViewEvaluateJavaScript,
    FileRead,
    FileWrite,
    CacheGet,
    CachePut,
    LogEmit,
    TimeNow,
    SystemInfo,
    PersistenceGet,
    PersistencePut,
}

impl HostCapability {
    pub const ALL: &'static [HostCapability] = &[
        HostCapability::HostSmokeEcho,
        HostCapability::HttpExecute,
        HostCapability::CookieGet,
        HostCapability::CookieSet,
        HostCapability::WebViewEvaluateJavaScript,
        HostCapability::FileRead,
        HostCapability::FileWrite,
        HostCapability::CacheGet,
        HostCapability::CachePut,
        HostCapability::LogEmit,
        HostCapability::TimeNow,
        HostCapability::SystemInfo,
        HostCapability::PersistenceGet,
        HostCapability::PersistencePut,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            HostCapability::HostSmokeEcho => "host.smoke.echo",
            HostCapability::HttpExecute => "http.execute",
            HostCapability::CookieGet => "cookie.get",
            HostCapability::CookieSet => "cookie.set",
            HostCapability::WebViewEvaluateJavaScript => "webview.evaluateJavaScript",
            HostCapability::FileRead => "file.read",
            HostCapability::FileWrite => "file.write",
            HostCapability::CacheGet => "cache.get",
            HostCapability::CachePut => "cache.put",
            HostCapability::LogEmit => "log.emit",
            HostCapability::TimeNow => "time.now",
            HostCapability::SystemInfo => "system.info",
            HostCapability::PersistenceGet => "persistence.get",
            HostCapability::PersistencePut => "persistence.put",
        }
    }

    pub fn parse(value: &str) -> Result<Self, HostCapabilityParseError> {
        if !is_valid_token_path(value) {
            return Err(HostCapabilityParseError::malformed(value));
        }

        match value {
            "host.smoke.echo" => Ok(HostCapability::HostSmokeEcho),
            "http.execute" => Ok(HostCapability::HttpExecute),
            "cookie.get" => Ok(HostCapability::CookieGet),
            "cookie.set" => Ok(HostCapability::CookieSet),
            "webview.evaluateJavaScript" => Ok(HostCapability::WebViewEvaluateJavaScript),
            "file.read" => Ok(HostCapability::FileRead),
            "file.write" => Ok(HostCapability::FileWrite),
            "cache.get" => Ok(HostCapability::CacheGet),
            "cache.put" => Ok(HostCapability::CachePut),
            "log.emit" => Ok(HostCapability::LogEmit),
            "time.now" => Ok(HostCapability::TimeNow),
            "system.info" => Ok(HostCapability::SystemInfo),
            "persistence.get" => Ok(HostCapability::PersistenceGet),
            "persistence.put" => Ok(HostCapability::PersistencePut),
            _ => Err(HostCapabilityParseError::unsupported(value)),
        }
    }

    pub fn all_as_str() -> Vec<&'static str> {
        Self::ALL
            .iter()
            .copied()
            .map(HostCapability::as_str)
            .collect()
    }
}

impl fmt::Display for HostCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for HostCapability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HostCapability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        HostCapability::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilityParseError {
    value: String,
    reason: &'static str,
}

impl HostCapabilityParseError {
    fn malformed(value: &str) -> Self {
        Self {
            value: value.to_string(),
            reason: "must be dot-separated non-empty tokens without whitespace",
        }
    }

    fn unsupported(value: &str) -> Self {
        Self {
            value: value.to_string(),
            reason: "is not in the Core host capability contract",
        }
    }
}

impl fmt::Display for HostCapabilityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "host capability {:?} {}", self.value, self.reason)
    }
}

impl std::error::Error for HostCapabilityParseError {}

/// Machine-readable host capability failure code supplied by the platform.
///
/// These codes describe host-owned capability execution failures. Core still
/// owns the enclosing `CoreError` and request correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostErrorCode {
    CapabilityUnavailable,
    PermissionDenied,
    Timeout,
    NetworkError,
    TlsError,
    HttpError,
    InvalidResponse,
    Cancelled,
    Internal,
}

/// Host execution phase where a capability failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostErrorPhase {
    Request,
    Transport,
    Response,
    Decode,
    Storage,
    Runtime,
}

/// Optional diagnostics attached to `host.error`.
///
/// Hosts must report capability execution facts only. They do not interpret
/// Legado rules or mutate Core-owned business semantics through this structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostErrorDiagnostics {
    pub code: HostErrorCode,
    pub phase: HostErrorPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(
        default = "empty_object",
        skip_serializing_if = "is_empty_object",
        deserialize_with = "deserialize_host_error_diagnostic_details"
    )]
    pub details: Value,
}

/// WebView document input supplied by Core for `webview.evaluateJavaScript`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostWebViewDocumentKind {
    Html,
    Url,
}

/// Document input for `webview.evaluateJavaScript`.
///
/// `kind: "html"` treats `body` as the HTML document body and may include a
/// `baseUrl`. `kind: "url"` treats `url` as navigation input. Hosts must not
/// infer Legado rule meaning from either form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostWebViewDocument {
    pub kind: HostWebViewDocumentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl HostWebViewDocument {
    pub fn validate(&self) -> Result<(), CoreError> {
        match self.kind {
            HostWebViewDocumentKind::Html => {
                validate_optional_non_blank(self.body.as_deref(), "webview document body")
                    .and_then(|_| {
                        validate_optional_non_blank(
                            self.base_url.as_deref(),
                            "webview document baseUrl",
                        )
                    })
                    .map_err(|message| {
                        CoreError::invalid_params(message)
                            .with_details(serde_json::json!({ "document": self }))
                    })?;
                if self.url.is_some() {
                    return Err(CoreError::invalid_params(
                        "webview html document must not include url",
                    )
                    .with_details(serde_json::json!({ "document": self })));
                }
                if self.body.is_none() {
                    return Err(
                        CoreError::invalid_params("webview html document requires body")
                            .with_details(serde_json::json!({ "document": self })),
                    );
                }
            }
            HostWebViewDocumentKind::Url => {
                validate_optional_non_blank(self.url.as_deref(), "webview document url").map_err(
                    |message| {
                        CoreError::invalid_params(message)
                            .with_details(serde_json::json!({ "document": self }))
                    },
                )?;
                if self.body.is_some() || self.base_url.is_some() {
                    return Err(CoreError::invalid_params(
                        "webview url document must not include body or baseUrl",
                    )
                    .with_details(serde_json::json!({ "document": self })));
                }
                if self.url.is_none() {
                    return Err(
                        CoreError::invalid_params("webview url document requires url")
                            .with_details(serde_json::json!({ "document": self })),
                    );
                }
            }
        }
        Ok(())
    }
}

/// Request params for `capability: "webview.evaluateJavaScript"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostWebViewEvaluateJavaScriptRequest {
    pub document: HostWebViewDocument,
    pub java_script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_millis: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

impl HostWebViewEvaluateJavaScriptRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        self.document.validate()?;
        validate_required_non_blank(&self.java_script, "webview.evaluateJavaScript javaScript")
            .and_then(|_| {
                validate_optional_non_blank(
                    self.profile_id.as_deref(),
                    "webview.evaluateJavaScript profileId",
                )
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })?;
        if self.timeout_millis.is_some_and(|timeout| timeout == 0) {
            return Err(CoreError::invalid_params(
                "webview.evaluateJavaScript timeoutMillis must be greater than 0",
            )
            .with_details(serde_json::json!({ "timeoutMillis": self.timeout_millis })));
        }
        Ok(())
    }
}

/// Result payload accepted for a `webview.evaluateJavaScript` host completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostWebViewEvaluateJavaScriptResponse {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl HostWebViewEvaluateJavaScriptResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_optional_non_blank(
            self.final_url.as_deref(),
            "webview.evaluateJavaScript finalUrl",
        )
        .and_then(|_| {
            validate_optional_non_blank(self.title.as_deref(), "webview.evaluateJavaScript title")
        })
        .map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "result": self,
            }))
        })
    }
}

/// Request params for `capability: "file.read"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostFileReadRequest {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

impl HostFileReadRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.path, "file.read path")
            .and_then(|_| {
                validate_optional_non_blank(self.encoding.as_deref(), "file.read encoding")
            })
            .and_then(|_| validate_optional_positive(self.max_bytes, "file.read maxBytes"))
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `file.read` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostFileReadResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_length: Option<u64>,
}

impl HostFileReadResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_exactly_one_string_payload(
            self.content.as_ref(),
            self.content_base64.as_ref(),
            "content",
            "contentBase64",
            "file.read result",
        )
        .and_then(|_| validate_optional_non_blank(self.encoding.as_deref(), "file.read encoding"))
        .map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "result": self,
            }))
        })
    }
}

/// Request params for `capability: "file.write"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostFileWriteRequest {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_directories: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<bool>,
}

impl HostFileWriteRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.path, "file.write path")
            .and_then(|_| {
                validate_exactly_one_string_payload(
                    self.content.as_ref(),
                    self.content_base64.as_ref(),
                    "content",
                    "contentBase64",
                    "file.write params",
                )
            })
            .and_then(|_| {
                validate_optional_non_blank(self.encoding.as_deref(), "file.write encoding")
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `file.write` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostFileWriteResponse {
    pub written: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_length: Option<u64>,
}

impl HostFileWriteResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !self.written {
            return Err(
                CoreError::invalid_params("file.write result written must be true")
                    .with_details(serde_json::json!({ "result": self })),
            );
        }
        Ok(())
    }
}

/// Request params for `capability: "cache.get"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCacheGetRequest {
    pub namespace: String,
    pub key: String,
}

impl HostCacheGetRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.namespace, "cache.get namespace")
            .and_then(|_| validate_required_non_blank(&self.key, "cache.get key"))
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `cache.get` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCacheGetResponse {
    pub hit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

impl HostCacheGetResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.hit {
            validate_exactly_one_string_payload(
                self.value.as_ref(),
                self.value_base64.as_ref(),
                "value",
                "valueBase64",
                "cache.get hit result",
            )
        } else if self.value.is_some() || self.value_base64.is_some() {
            Err("cache.get miss result must not include value or valueBase64".to_string())
        } else {
            Ok(())
        }
        .and_then(|_| {
            validate_optional_non_blank(self.expires_at.as_deref(), "cache.get expiresAt")
        })
        .map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "result": self,
            }))
        })
    }
}

/// Request params for `capability: "cache.put"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCachePutRequest {
    pub namespace: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_millis: Option<u64>,
}

impl HostCachePutRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.namespace, "cache.put namespace")
            .and_then(|_| validate_required_non_blank(&self.key, "cache.put key"))
            .and_then(|_| {
                validate_exactly_one_string_payload(
                    self.value.as_ref(),
                    self.value_base64.as_ref(),
                    "value",
                    "valueBase64",
                    "cache.put params",
                )
            })
            .and_then(|_| validate_optional_positive(self.ttl_millis, "cache.put ttlMillis"))
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `cache.put` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCachePutResponse {
    pub stored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

impl HostCachePutResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !self.stored {
            return Err(
                CoreError::invalid_params("cache.put result stored must be true")
                    .with_details(serde_json::json!({ "result": self })),
            );
        }
        validate_optional_non_blank(self.expires_at.as_deref(), "cache.put expiresAt").map_err(
            |message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "result": self,
                }))
            },
        )
    }
}

/// Cookie record exchanged with cookie host capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCookieRecord {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
}

impl HostCookieRecord {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.name, "cookie.name")
            .and_then(|_| validate_optional_non_blank(self.domain.as_deref(), "cookie.domain"))
            .and_then(|_| validate_optional_non_blank(self.path.as_deref(), "cookie.path"))
            .and_then(|_| {
                validate_optional_non_blank(self.expires_at.as_deref(), "cookie.expiresAt")
            })
            .and_then(|_| validate_optional_non_blank(self.same_site.as_deref(), "cookie.sameSite"))
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "cookie": self,
                }))
            })
    }
}

/// Request params for `capability: "cookie.get"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCookieGetRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl HostCookieGetRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.url.is_none() && self.domain.is_none() && self.session_id.is_none() {
            return Err(
                CoreError::invalid_params("cookie.get requires url, domain, or sessionId")
                    .with_details(serde_json::json!({ "params": self })),
            );
        }
        validate_optional_non_blank(self.url.as_deref(), "cookie.get url")
            .and_then(|_| validate_optional_non_blank(self.domain.as_deref(), "cookie.get domain"))
            .and_then(|_| validate_optional_non_blank(self.name.as_deref(), "cookie.get name"))
            .and_then(|_| {
                validate_optional_non_blank(self.session_id.as_deref(), "cookie.get sessionId")
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `cookie.get` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCookieGetResponse {
    #[serde(default)]
    pub cookies: Vec<HostCookieRecord>,
}

impl HostCookieGetResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        for cookie in &self.cookies {
            cookie.validate()?;
        }
        Ok(())
    }
}

/// Request params for `capability: "cookie.set"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCookieSetRequest {
    pub cookie: HostCookieRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl HostCookieSetRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        self.cookie.validate()?;
        validate_optional_non_blank(self.url.as_deref(), "cookie.set url")
            .and_then(|_| {
                validate_optional_non_blank(self.session_id.as_deref(), "cookie.set sessionId")
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `cookie.set` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCookieSetResponse {
    pub stored: bool,
}

impl HostCookieSetResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.stored {
            Ok(())
        } else {
            Err(
                CoreError::invalid_params("cookie.set result stored must be true")
                    .with_details(serde_json::json!({ "result": self })),
            )
        }
    }
}

/// Log severity supplied by Core for `log.emit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Request params for `capability: "log.emit"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostLogEmitRequest {
    pub level: HostLogLevel,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default = "empty_object", skip_serializing_if = "is_empty_object")]
    pub fields: Value,
}

impl HostLogEmitRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.message, "log.emit message")
            .and_then(|_| validate_optional_non_blank(self.target.as_deref(), "log.emit target"))
            .and_then(|_| validate_object_value(&self.fields, "log.emit fields"))
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `log.emit` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostLogEmitResponse {
    pub emitted: bool,
}

impl HostLogEmitResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.emitted {
            Ok(())
        } else {
            Err(
                CoreError::invalid_params("log.emit result emitted must be true")
                    .with_details(serde_json::json!({ "result": self })),
            )
        }
    }
}

/// Request params for `capability: "time.now"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostTimeNowRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl HostTimeNowRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_optional_non_blank(self.clock.as_deref(), "time.now clock")
            .and_then(|_| {
                validate_optional_non_blank(self.timezone.as_deref(), "time.now timezone")
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `time.now` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostTimeNowResponse {
    pub unix_millis: u64,
    pub iso8601: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl HostTimeNowResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.iso8601, "time.now iso8601")
            .and_then(|_| {
                validate_optional_non_blank(self.timezone.as_deref(), "time.now timezone")
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "result": self,
                }))
            })
    }
}

/// Request params for `capability: "system.info"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSystemInfoRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

impl HostSystemInfoRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.keys.iter().any(|key| key.trim().is_empty()) {
            return Err(
                CoreError::invalid_params("system.info keys must not be blank")
                    .with_details(serde_json::json!({ "params": self })),
            );
        }
        Ok(())
    }
}

/// Result payload accepted for a `system.info` host completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSystemInfoResponse {
    pub info: Value,
}

impl HostSystemInfoResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_object_value(&self.info, "system.info result info").map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "result": self,
            }))
        })
    }
}

/// Request params for `capability: "persistence.get"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPersistenceGetRequest {
    pub namespace: String,
    pub key: String,
}

impl HostPersistenceGetRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.namespace, "persistence.get namespace")
            .and_then(|_| validate_required_non_blank(&self.key, "persistence.get key"))
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `persistence.get` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPersistenceGetResponse {
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl HostPersistenceGetResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.found {
            validate_exactly_one_string_payload(
                self.value.as_ref(),
                self.value_base64.as_ref(),
                "value",
                "valueBase64",
                "persistence.get found result",
            )
        } else if self.value.is_some() || self.value_base64.is_some() {
            Err("persistence.get miss result must not include value or valueBase64".to_string())
        } else {
            Ok(())
        }
        .and_then(|_| {
            validate_optional_non_blank(self.revision.as_deref(), "persistence.get revision")
        })
        .map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "result": self,
            }))
        })
    }
}

/// Request params for `capability: "persistence.put"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPersistencePutRequest {
    pub namespace: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
}

impl HostPersistencePutRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.namespace, "persistence.put namespace")
            .and_then(|_| validate_required_non_blank(&self.key, "persistence.put key"))
            .and_then(|_| {
                validate_exactly_one_string_payload(
                    self.value.as_ref(),
                    self.value_base64.as_ref(),
                    "value",
                    "valueBase64",
                    "persistence.put params",
                )
            })
            .and_then(|_| {
                validate_optional_non_blank(
                    self.expected_revision.as_deref(),
                    "persistence.put expectedRevision",
                )
            })
            .map_err(|message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "params": self,
                }))
            })
    }
}

/// Result payload accepted for a `persistence.put` host completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPersistencePutResponse {
    pub stored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl HostPersistencePutResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !self.stored {
            return Err(
                CoreError::invalid_params("persistence.put result stored must be true")
                    .with_details(serde_json::json!({ "result": self })),
            );
        }
        validate_optional_non_blank(self.revision.as_deref(), "persistence.put revision").map_err(
            |message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "result": self,
                }))
            },
        )
    }
}

/// Parameters for `runtime.hostSmoke`, a local driver method that exercises the
/// host bus without involving reader business modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSmokeParams {
    #[serde(default = "default_smoke_capability")]
    pub capability: HostCapability,
    #[serde(
        default = "empty_object",
        deserialize_with = "deserialize_host_smoke_params"
    )]
    pub params: Value,
}

impl HostSmokeParams {
    pub fn validate(&self) -> Result<(), CoreError> {
        match self.capability {
            HostCapability::WebViewEvaluateJavaScript => {
                let request = serde_json::from_value::<HostWebViewEvaluateJavaScriptRequest>(
                    self.params.clone(),
                )
                .map_err(|err| {
                    CoreError::invalid_params("invalid params for webview.evaluateJavaScript")
                        .with_details(serde_json::json!({
                            "source": err.to_string(),
                            "capability": self.capability,
                        }))
                })?;
                request.validate()?;
            }
            HostCapability::FileRead => {
                parse_host_smoke_request::<HostFileReadRequest>(
                    self.capability,
                    &self.params,
                    "file.read",
                )?
                .validate()?;
            }
            HostCapability::FileWrite => {
                parse_host_smoke_request::<HostFileWriteRequest>(
                    self.capability,
                    &self.params,
                    "file.write",
                )?
                .validate()?;
            }
            HostCapability::CacheGet => {
                parse_host_smoke_request::<HostCacheGetRequest>(
                    self.capability,
                    &self.params,
                    "cache.get",
                )?
                .validate()?;
            }
            HostCapability::CachePut => {
                parse_host_smoke_request::<HostCachePutRequest>(
                    self.capability,
                    &self.params,
                    "cache.put",
                )?
                .validate()?;
            }
            HostCapability::CookieGet => {
                parse_host_smoke_request::<HostCookieGetRequest>(
                    self.capability,
                    &self.params,
                    "cookie.get",
                )?
                .validate()?;
            }
            HostCapability::CookieSet => {
                parse_host_smoke_request::<HostCookieSetRequest>(
                    self.capability,
                    &self.params,
                    "cookie.set",
                )?
                .validate()?;
            }
            HostCapability::LogEmit => {
                parse_host_smoke_request::<HostLogEmitRequest>(
                    self.capability,
                    &self.params,
                    "log.emit",
                )?
                .validate()?;
            }
            HostCapability::TimeNow => {
                parse_host_smoke_request::<HostTimeNowRequest>(
                    self.capability,
                    &self.params,
                    "time.now",
                )?
                .validate()?;
            }
            HostCapability::SystemInfo => {
                parse_host_smoke_request::<HostSystemInfoRequest>(
                    self.capability,
                    &self.params,
                    "system.info",
                )?
                .validate()?;
            }
            HostCapability::PersistenceGet => {
                parse_host_smoke_request::<HostPersistenceGetRequest>(
                    self.capability,
                    &self.params,
                    "persistence.get",
                )?
                .validate()?;
            }
            HostCapability::PersistencePut => {
                parse_host_smoke_request::<HostPersistencePutRequest>(
                    self.capability,
                    &self.params,
                    "persistence.put",
                )?
                .validate()?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn parse_host_smoke_request<T>(
    capability: HostCapability,
    params: &Value,
    label: &'static str,
) -> Result<T, CoreError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value::<T>(params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {label}")).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "capability": capability,
            }),
        )
    })
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
    pub capability: HostCapability,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<HostErrorDiagnostics>,
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
        assert!(error.diagnostics.is_none());

        let diagnostic_error_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/error-diagnostics.json")
                .as_bytes(),
        )
        .unwrap();
        let diagnostic_error: HostErrorParams =
            serde_json::from_value(diagnostic_error_command.params).unwrap();
        let diagnostics = diagnostic_error
            .diagnostics
            .expect("host.error diagnostics should parse");
        assert_eq!(diagnostics.code, HostErrorCode::Timeout);
        assert_eq!(diagnostics.phase, HostErrorPhase::Transport);
        assert_eq!(diagnostics.details["timeoutMillis"], 30000);
    }

    #[test]
    fn host_smoke_capability_accepts_known_capability_and_rejects_unsupported_names() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/request.json").as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
        assert_eq!(params.capability, HostCapability::HostSmokeEcho);
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
            (
                "unsupported",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/request-unsupported-capability.json"
                ),
            ),
        ] {
            let command = crate::Command::from_json_bytes(json.as_bytes()).unwrap();
            let err = match serde_json::from_value::<HostSmokeParams>(command.params) {
                Ok(_) => panic!("{name} should reject host capability"),
                Err(err) => err,
            };
            assert!(
                err.to_string().contains("host capability"),
                "unexpected capability parse error for {name}: {err}"
            );
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
    fn webview_evaluate_javascript_request_and_response_fixtures_parse() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/webview-request.json")
                .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
        assert_eq!(params.capability, HostCapability::WebViewEvaluateJavaScript);
        params.validate().unwrap();

        let request: HostWebViewEvaluateJavaScriptRequest =
            serde_json::from_value(params.params).unwrap();
        assert_eq!(request.document.kind, HostWebViewDocumentKind::Html);
        assert_eq!(
            request.document.base_url.as_deref(),
            Some("https://books.example.test/detail")
        );
        assert!(request.java_script.contains("querySelector"));
        assert_eq!(request.timeout_millis, Some(3000));
        assert_eq!(request.profile_id.as_deref(), Some("core-webview-fixture"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/webview-complete.json")
                .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostWebViewEvaluateJavaScriptResponse =
            serde_json::from_value(complete.result).unwrap();
        response.validate().unwrap();
        assert_eq!(response.value, serde_json::json!("Dune"));
        assert_eq!(
            response.final_url.as_deref(),
            Some("https://books.example.test/detail")
        );
    }

    #[test]
    fn webview_evaluate_javascript_contract_rejects_invalid_boundaries() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/webview-request-blank-javascript.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
        let err = params.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("javaScript"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/webview-complete-blank-final-url.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostWebViewEvaluateJavaScriptResponse =
            serde_json::from_value(complete.result).unwrap();
        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("finalUrl"));
    }

    #[test]
    fn file_cache_request_and_response_fixtures_parse() {
        let file_read_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/file-read-request.json")
                .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(file_read_command.params).unwrap();
        assert_eq!(params.capability, HostCapability::FileRead);
        params.validate().unwrap();
        let request: HostFileReadRequest = serde_json::from_value(params.params).unwrap();
        assert_eq!(request.path, "core-cache/books/basic.json");
        assert_eq!(request.encoding.as_deref(), Some("utf-8"));
        assert_eq!(request.byte_offset, Some(0));
        assert_eq!(request.max_bytes, Some(4096));

        let file_write_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/file-write-request.json")
                .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(file_write_command.params).unwrap();
        assert_eq!(params.capability, HostCapability::FileWrite);
        params.validate().unwrap();
        let request: HostFileWriteRequest = serde_json::from_value(params.params).unwrap();
        assert_eq!(request.path, "core-cache/books/basic.json");
        assert_eq!(request.content.as_deref(), Some("{\"books\":[]}"));
        assert_eq!(request.create_directories, Some(true));
        assert_eq!(request.append, Some(false));

        let cache_get_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/cache-get-request.json")
                .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(cache_get_command.params).unwrap();
        assert_eq!(params.capability, HostCapability::CacheGet);
        params.validate().unwrap();
        let request: HostCacheGetRequest = serde_json::from_value(params.params).unwrap();
        assert_eq!(request.namespace, "remote.response");
        assert_eq!(request.key, "search/basic");

        let cache_put_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/cache-put-request.json")
                .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(cache_put_command.params).unwrap();
        assert_eq!(params.capability, HostCapability::CachePut);
        params.validate().unwrap();
        let request: HostCachePutRequest = serde_json::from_value(params.params).unwrap();
        assert_eq!(request.namespace, "remote.response");
        assert_eq!(request.key, "search/basic");
        assert_eq!(request.value.as_deref(), Some("{\"books\":[]}"));
        assert_eq!(request.ttl_millis, Some(60000));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/file-read-complete.json")
                .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostFileReadResponse = serde_json::from_value(complete.result).unwrap();
        response.validate().unwrap();
        assert_eq!(response.content.as_deref(), Some("cached body"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/file-write-complete.json")
                .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostFileWriteResponse = serde_json::from_value(complete.result).unwrap();
        response.validate().unwrap();
        assert!(response.written);

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/cache-get-complete-hit.json")
                .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostCacheGetResponse = serde_json::from_value(complete.result).unwrap();
        response.validate().unwrap();
        assert!(response.hit);
        assert_eq!(response.value.as_deref(), Some("{\"books\":[]}"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/host/cache-put-complete.json")
                .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostCachePutResponse = serde_json::from_value(complete.result).unwrap();
        response.validate().unwrap();
        assert!(response.stored);
    }

    #[test]
    fn file_cache_contract_rejects_invalid_boundaries() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/file-read-request-blank-path.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
        let err = params.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("file.read path"));

        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/cache-put-request-missing-value.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: HostSmokeParams = serde_json::from_value(command.params).unwrap();
        let err = params.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("value"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/file-read-complete-missing-content.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostFileReadResponse = serde_json::from_value(complete.result).unwrap();
        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("content"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/file-write-complete-not-written.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostFileWriteResponse = serde_json::from_value(complete.result).unwrap();
        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("written"));

        let complete_command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/host/cache-get-complete-invalid-hit.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let complete: HostCompleteParams = serde_json::from_value(complete_command.params).unwrap();
        let response: HostCacheGetResponse = serde_json::from_value(complete.result).unwrap();
        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("value"));
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

        for (name, json, expected) in [
            (
                "diagnostics details",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/error-diagnostics-details-not-object.json"
                ),
                "diagnostics.details",
            ),
            (
                "diagnostics unknown field",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/error-diagnostics-unknown-field.json"
                ),
                "unknown field",
            ),
        ] {
            let command = crate::Command::from_json_bytes(json.as_bytes()).unwrap();
            let err = match serde_json::from_value::<HostErrorParams>(command.params) {
                Ok(_) => panic!("{name} should reject invalid diagnostics"),
                Err(err) => err,
            };
            assert!(
                err.to_string().contains(expected),
                "unexpected host.error diagnostics parse error for {name}: {err}"
            );
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
    fn pending_host_operation_status_requires_known_host_capability() {
        for capability in ["", "host. smoke.echo", "host..echo", "host", "custom.valid"] {
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

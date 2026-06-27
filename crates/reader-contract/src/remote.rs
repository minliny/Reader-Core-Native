//! Typed params for the remote-reading vertical commands.
//!
//! These mirror the V1 "minimal vertical" pipeline: source import → search →
//! detail → toc → chapter → progress. Each command can take a prefetched
//! response body for deterministic tests, or a host HTTP request descriptor
//! that Core emits as `capability: "http.execute"` (see
//! `protocol/compatibility.md`).

use std::collections::BTreeMap;

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::CoreError;

fn empty_string() -> String {
    String::new()
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_true() -> bool {
    true
}

fn deserialize_http_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_http_url_scalar(&value).map_err(de::Error::custom)?;
    Ok(value)
}

fn validate_http_url_scalar(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        Err("http.execute request url must be non-empty")
    } else {
        Ok(())
    }
}

fn deserialize_http_method<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_http_method_scalar(&value).map_err(de::Error::custom)?;
    Ok(value)
}

fn validate_http_method_scalar(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        Err("http.execute request method must be non-empty")
    } else {
        Ok(())
    }
}

fn deserialize_http_headers<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    validate_http_headers_shape(&value).map_err(de::Error::custom)?;
    Ok(value)
}

fn validate_http_headers_shape(value: &Value) -> Result<(), &'static str> {
    if value.is_object() {
        Ok(())
    } else {
        Err("http.execute request headers must be an object")
    }
}

fn validate_optional_non_blank(value: Option<&str>, field: &'static str) -> Result<(), String> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        Err(format!("http.execute {field} must not be blank"))
    } else {
        Ok(())
    }
}

fn validate_required_non_blank(value: &str, field: &'static str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("http.execute {field} must not be blank"))
    } else {
        Ok(())
    }
}

fn deserialize_non_blank_source_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_source_name_scalar(&value).map_err(de::Error::custom)?;
    Ok(value)
}

fn validate_source_name_scalar(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        Err("source.import name must be non-empty")
    } else {
        Ok(())
    }
}

fn deserialize_non_blank_source_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom("sourceId must be non-empty"))
    } else {
        Ok(value)
    }
}

fn deserialize_non_blank_book_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom("bookId must be non-empty"))
    } else {
        Ok(value)
    }
}

fn deserialize_non_blank_book_title<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom("book.search title must be non-empty"))
    } else {
        Ok(value)
    }
}

fn deserialize_book_search_books<'de, D>(
    deserializer: D,
) -> Result<Vec<BookSearchBookData>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if !value.is_array() {
        return Err(de::Error::custom("book.search books must be an array"));
    }
    serde_json::from_value(value).map_err(de::Error::custom)
}

fn deserialize_book_detail_data_book<'de, D>(
    deserializer: D,
) -> Result<BookDetailBookData, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(de::Error::custom("book.detail book must be an object"));
    }
    serde_json::from_value(value).map_err(de::Error::custom)
}

fn deserialize_book_toc_entries<'de, D>(deserializer: D) -> Result<Vec<BookTocEntryData>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if !value.is_array() {
        return Err(de::Error::custom("book.toc toc must be an array"));
    }
    serde_json::from_value(value).map_err(de::Error::custom)
}

fn deserialize_remote_http_status<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<u16>::deserialize(deserializer)?;
    let Some(status) = value else {
        return Err(de::Error::custom(
            "remote http diagnostics status must be an integer",
        ));
    };
    if !(100..=599).contains(&status) {
        Err(de::Error::custom(
            "remote http diagnostics status must be between 100 and 599",
        ))
    } else {
        Ok(Some(status))
    }
}

fn deserialize_remote_http_headers<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err(de::Error::custom(
            "remote http diagnostics headers must be an object",
        ))
    }
}

fn deserialize_remote_http_session<'de, D>(
    deserializer: D,
) -> Result<Option<HostHttpSession>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<HostHttpSession>::deserialize(deserializer)?;
    if let Some(session) = &value {
        session
            .validate()
            .map_err(|err| de::Error::custom(err.message))?;
    }
    Ok(value)
}

fn deserialize_remote_http_redirects<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<HostHttpRedirect>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Vec<HostHttpRedirect>>::deserialize(deserializer)?;
    if let Some(redirects) = &value {
        for redirect in redirects {
            redirect
                .validate()
                .map_err(|err| de::Error::custom(err.message))?;
        }
    }
    Ok(value)
}

fn deserialize_remote_http_cookies<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<HostHttpCookie>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Vec<HostHttpCookie>>::deserialize(deserializer)?;
    if let Some(cookies) = &value {
        for cookie in cookies {
            cookie
                .validate()
                .map_err(|err| de::Error::custom(err.message))?;
        }
    }
    Ok(value)
}

fn deserialize_remote_http_diagnostics<'de, D>(
    deserializer: D,
) -> Result<Option<RemoteHttpDiagnosticsData>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(de::Error::custom(
            "remote http diagnostics must be an object",
        ));
    }
    if value.as_object().is_some_and(|object| object.is_empty()) {
        return Err(de::Error::custom(
            "remote http diagnostics must include status or headers",
        ));
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(de::Error::custom)
}

fn deserialize_source_import_imported<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = bool::deserialize(deserializer)?;
    if value {
        Ok(value)
    } else {
        Err(de::Error::custom("source.import imported must be true"))
    }
}

fn deserialize_object_or_null<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    validate_object_or_null(&value).map_err(de::Error::custom)?;
    Ok(value)
}

fn deserialize_book_source_object_or_null<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() || value.is_null() {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "source.import bookSource must be an object or null",
        ))
    }
}

fn validate_object_or_null(value: &Value) -> Result<(), &'static str> {
    if value.is_object() || value.is_null() {
        Ok(())
    } else {
        Err("source.import rules must be an object or null")
    }
}

fn deserialize_inline_source<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    if let Some(source) = &value {
        validate_inline_source_shape(source).map_err(de::Error::custom)?;
    }
    Ok(value)
}

fn validate_inline_source_shape(value: &Value) -> Result<(), &'static str> {
    if value.is_object() {
        Ok(())
    } else {
        Err("inline source must be an object or null")
    }
}

fn deserialize_book_detail_book<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(de::Error::custom("book.detail book must be an object"))
    }
}

fn deserialize_chapter_progress<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    validate_chapter_progress_scalar(value).map_err(de::Error::custom)?;
    Ok(value)
}

fn validate_chapter_progress_scalar(value: f64) -> Result<(), &'static str> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err("chapterProgress must be between 0 and 1")
    }
}

fn deserialize_reading_progress_stored<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = bool::deserialize(deserializer)?;
    if value {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "reading.progress.update stored must be true",
        ))
    }
}

/// Host HTTP request description emitted as `capability: "http.execute"`.
///
/// Core owns request semantics; platform hosts own the actual socket/TLS stack
/// and answer with `host.complete { result: { body: "..." } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHttpRequest {
    #[serde(deserialize_with = "deserialize_http_url")]
    pub url: String,
    #[serde(
        default = "default_http_method",
        deserialize_with = "deserialize_http_method"
    )]
    pub method: String,
    #[serde(
        default = "empty_object",
        deserialize_with = "deserialize_http_headers"
    )]
    pub headers: Value,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub charset: Option<String>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub max_redirects: Option<u32>,
    #[serde(default)]
    pub retry: Option<HostHttpRetryPolicy>,
    #[serde(default)]
    pub use_platform_cookie_jar: Option<bool>,
    #[serde(default)]
    pub session: Option<HostHttpSession>,
}

/// Retry policy requested by Core and executed by the host HTTP stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHttpRetryPolicy {
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff_millis: Option<u64>,
}

impl HostHttpRetryPolicy {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.max_attempts == 0 {
            return Err(CoreError::invalid_params(
                "http.execute request retry.maxAttempts must be greater than 0",
            )
            .with_details(serde_json::json!({ "retry": self })));
        }
        Ok(())
    }
}

/// Session/cookie jar handle. The host must treat this as an opaque Core-owned
/// session identifier unless a later platform contract says otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHttpSession {
    pub id: String,
}

impl HostHttpSession {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_optional_non_blank(Some(&self.id), "session.id").map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({ "session": self }))
        })
    }
}

/// One redirect hop observed by the host HTTP stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHttpRedirect {
    pub status: u16,
    pub from_url: String,
    pub to_url: String,
    #[serde(default)]
    pub headers: Option<Value>,
}

impl HostHttpRedirect {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !(300..=399).contains(&self.status) {
            return Err(CoreError::invalid_params(
                "http.execute redirect.status must be between 300 and 399",
            )
            .with_details(serde_json::json!({ "redirect": self })));
        }
        validate_required_non_blank(&self.from_url, "redirect.fromUrl")
            .and_then(|_| validate_required_non_blank(&self.to_url, "redirect.toUrl"))
            .map_err(|message| {
                CoreError::invalid_params(message)
                    .with_details(serde_json::json!({ "redirect": self }))
            })?;
        if let Some(headers) = &self.headers {
            if !headers.is_object() {
                return Err(CoreError::invalid_params(
                    "http.execute redirect.headers must be an object",
                )
                .with_details(serde_json::json!({ "redirect": self })));
            }
        }
        Ok(())
    }
}

/// Cookie metadata observed or applied by the host HTTP stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHttpCookie {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub http_only: Option<bool>,
    #[serde(default)]
    pub secure: Option<bool>,
    #[serde(default)]
    pub same_site: Option<String>,
}

impl HostHttpCookie {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_required_non_blank(&self.name, "cookie.name")
            .and_then(|_| validate_optional_non_blank(self.domain.as_deref(), "cookie.domain"))
            .and_then(|_| validate_optional_non_blank(self.path.as_deref(), "cookie.path"))
            .and_then(|_| {
                validate_optional_non_blank(self.expires_at.as_deref(), "cookie.expiresAt")
            })
            .and_then(|_| validate_optional_non_blank(self.same_site.as_deref(), "cookie.sameSite"))
            .map_err(|message| {
                CoreError::invalid_params(message)
                    .with_details(serde_json::json!({ "cookie": self }))
            })
    }
}

impl HostHttpRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        if let Err(message) = validate_http_url_scalar(&self.url) {
            return Err(
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "field": "url",
                    "url": self.url,
                })),
            );
        }

        if let Err(message) = validate_http_method_scalar(&self.method) {
            return Err(
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "field": "method",
                    "method": self.method,
                })),
            );
        }

        validate_http_headers_shape(&self.headers).map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "field": "headers",
                "headers": self.headers,
            }))
        })?;

        validate_optional_non_blank(self.charset.as_deref(), "request charset").map_err(
            |message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "field": "charset",
                    "charset": self.charset,
                }))
            },
        )?;

        if let Some(retry) = &self.retry {
            retry.validate()?;
        }

        if let Some(session) = &self.session {
            session.validate()?;
        }

        Ok(())
    }
}

/// Host HTTP response accepted for `http.execute` completion.
///
/// This is intentionally scoped to remote-reading continuations. The host bus
/// remains generic, but once Core has emitted `capability: "http.execute"` the
/// completion payload has a typed v1 shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostHttpResponse {
    pub body: String,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub headers: Option<Value>,
    #[serde(default)]
    pub final_url: Option<String>,
    #[serde(default)]
    pub charset_hint: Option<String>,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub session: Option<HostHttpSession>,
    #[serde(default)]
    pub redirects: Option<Vec<HostHttpRedirect>>,
    #[serde(default)]
    pub cookies: Option<Vec<HostHttpCookie>>,
}

impl HostHttpResponse {
    pub fn validate(&self) -> Result<(), CoreError> {
        if let Some(status) = self.status {
            if !(100..=599).contains(&status) {
                return Err(CoreError::invalid_params(
                    "http.execute host result.status must be between 100 and 599",
                )
                .with_details(serde_json::json!({ "status": status })));
            }
        }

        if let Some(headers) = &self.headers {
            if !headers.is_null() && !headers.is_object() {
                return Err(CoreError::invalid_params(
                    "http.execute host result.headers must be an object",
                )
                .with_details(serde_json::json!({ "headers": headers })));
            }
        }

        validate_optional_non_blank(self.final_url.as_deref(), "result finalUrl").map_err(
            |message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "finalUrl": self.final_url,
                }))
            },
        )?;

        validate_optional_non_blank(self.charset_hint.as_deref(), "result charsetHint").map_err(
            |message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "charsetHint": self.charset_hint,
                }))
            },
        )?;

        validate_optional_non_blank(self.body_base64.as_deref(), "result bodyBase64").map_err(
            |message| {
                CoreError::invalid_params(message).with_details(serde_json::json!({
                    "bodyBase64": self.body_base64,
                }))
            },
        )?;

        if let Some(session) = &self.session {
            session.validate()?;
        }

        if let Some(redirects) = &self.redirects {
            for redirect in redirects {
                redirect.validate()?;
            }
        }

        if let Some(cookies) = &self.cookies {
            for cookie in cookies {
                cookie.validate()?;
            }
        }

        Ok(())
    }
}

/// Parameters for `source.import`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceImportParams {
    /// Stable source identifier. If omitted, one is assigned.
    #[serde(default = "empty_string")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_non_blank_source_name")]
    pub name: String,
    #[serde(default = "empty_string")]
    pub base_url: String,
    /// Extraction rules keyed by stage (`search`/`detail`/`toc`/`chapter`).
    /// Each value is a JSON array of rule-step specs understood by
    /// `reader-content`.
    #[serde(default, deserialize_with = "deserialize_object_or_null")]
    pub rules: Value,
    /// Optional raw Legado BookSource payload. Core preserves this separately
    /// from V1 `rules` so DSL migration can happen without losing source data.
    #[serde(
        default,
        rename = "bookSource",
        deserialize_with = "deserialize_book_source_object_or_null"
    )]
    pub book_source: Value,
}

/// Result data for `source.import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceImportData {
    #[serde(deserialize_with = "deserialize_non_blank_source_id")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_non_blank_source_name")]
    pub name: String,
    #[serde(deserialize_with = "deserialize_source_import_imported")]
    pub imported: bool,
}

/// Parameters for `book.search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSearchParams {
    pub source_id: String,
    /// Pre-fetched search response body (HTML or JSON).
    #[serde(default = "empty_string")]
    pub search_response: String,
    /// Optional host HTTP request. If `searchResponse` is empty/missing, Core
    /// emits `http.execute` and continues parsing after host completion.
    #[serde(default)]
    pub search_request: Option<HostHttpRequest>,
    /// Optional inline source definition. If present, it is used instead of
    /// looking up `source_id` in storage (useful for smoke tests).
    #[serde(default, deserialize_with = "deserialize_inline_source")]
    pub source: Option<Value>,
}

/// Optional HTTP diagnostics attached to remote-reading results that resumed
/// from a host `http.execute` completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteHttpDiagnosticsData {
    #[serde(default, deserialize_with = "deserialize_remote_http_status")]
    pub status: Option<u16>,
    #[serde(default, deserialize_with = "deserialize_remote_http_headers")]
    pub headers: Option<Value>,
    #[serde(default)]
    pub final_url: Option<String>,
    #[serde(default)]
    pub charset_hint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_remote_http_session")]
    pub session: Option<HostHttpSession>,
    #[serde(default, deserialize_with = "deserialize_remote_http_redirects")]
    pub redirects: Option<Vec<HostHttpRedirect>>,
    #[serde(default, deserialize_with = "deserialize_remote_http_cookies")]
    pub cookies: Option<Vec<HostHttpCookie>>,
}

/// Minimal stable book shape returned by `book.search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSearchBookData {
    #[serde(deserialize_with = "deserialize_non_blank_book_id")]
    pub book_id: String,
    #[serde(deserialize_with = "deserialize_non_blank_book_title")]
    pub title: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Result data for `book.search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSearchData {
    #[serde(deserialize_with = "deserialize_non_blank_source_id")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_book_search_books")]
    pub books: Vec<BookSearchBookData>,
    #[serde(default, deserialize_with = "deserialize_remote_http_diagnostics")]
    pub http: Option<RemoteHttpDiagnosticsData>,
}

/// Stable book detail object returned by `book.detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookDetailBookData {
    #[serde(deserialize_with = "deserialize_non_blank_book_id")]
    pub book_id: String,
    pub title: String,
    pub author: String,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub intro: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub last_chapter: Option<String>,
}

/// Result data for `book.detail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookDetailData {
    #[serde(deserialize_with = "deserialize_non_blank_source_id")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_book_detail_data_book")]
    pub book: BookDetailBookData,
    #[serde(default, deserialize_with = "deserialize_remote_http_diagnostics")]
    pub http: Option<RemoteHttpDiagnosticsData>,
}

/// Stable table-of-contents entry returned by `book.toc`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookTocEntryData {
    pub index: u32,
    pub title: String,
    pub url: String,
}

/// Result data for `book.toc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookTocData {
    #[serde(deserialize_with = "deserialize_non_blank_source_id")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_non_blank_book_id")]
    pub book_id: String,
    #[serde(deserialize_with = "deserialize_book_toc_entries")]
    pub toc: Vec<BookTocEntryData>,
    #[serde(default, deserialize_with = "deserialize_remote_http_diagnostics")]
    pub http: Option<RemoteHttpDiagnosticsData>,
}

/// Execution path that produced a `chapter.content` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChapterContentVia {
    Rule,
    Js,
}

/// Result data for `chapter.content`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterContentData {
    #[serde(deserialize_with = "deserialize_non_blank_source_id")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_non_blank_book_id")]
    pub book_id: String,
    pub chapter_title: String,
    pub content: Value,
    pub via: ChapterContentVia,
    #[serde(default, deserialize_with = "deserialize_remote_http_diagnostics")]
    pub http: Option<RemoteHttpDiagnosticsData>,
}

/// Parameters for `book.detail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookDetailParams {
    pub source_id: String,
    /// Base book to merge metadata into (must contain at least `bookId`).
    #[serde(deserialize_with = "deserialize_book_detail_book")]
    pub book: Value,
    /// Pre-fetched detail response body.
    #[serde(default = "empty_string")]
    pub detail_response: String,
    #[serde(default)]
    pub detail_request: Option<HostHttpRequest>,
    #[serde(default, deserialize_with = "deserialize_inline_source")]
    pub source: Option<Value>,
}

/// Parameters for `book.toc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookTocParams {
    pub source_id: String,
    pub book_id: String,
    /// Pre-fetched toc response body.
    #[serde(default = "empty_string")]
    pub toc_response: String,
    #[serde(default)]
    pub toc_request: Option<HostHttpRequest>,
    #[serde(default, deserialize_with = "deserialize_inline_source")]
    pub source: Option<Value>,
}

/// Parameters for `chapter.content`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterContentParams {
    pub source_id: String,
    pub book_id: String,
    /// Optional chapter title (informational; surfaced in the result).
    #[serde(default = "empty_string")]
    pub chapter_title: String,
    /// Pre-fetched chapter response body.
    #[serde(default = "empty_string")]
    pub chapter_response: String,
    #[serde(default)]
    pub chapter_request: Option<HostHttpRequest>,
    /// Optional JS rule script. If present and it calls a host capability
    /// (`java.get`/`java.post`) without a registered callback, the command
    /// returns a structured `unsupported` error rather than pretending a
    /// network call happened.
    #[serde(default)]
    pub js_rule: Option<String>,
    #[serde(default, deserialize_with = "deserialize_inline_source")]
    pub source: Option<Value>,
}

/// Parameters for `reading.progress.update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingProgressUpdateParams {
    pub book_id: String,
    #[serde(default)]
    pub chapter_index: u32,
    #[serde(default)]
    pub chapter_offset: u64,
    #[serde(default, deserialize_with = "deserialize_chapter_progress")]
    pub chapter_progress: f64,
}

impl ReadingProgressUpdateParams {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_chapter_progress_scalar(self.chapter_progress).map_err(|message| {
            CoreError::invalid_params(message).with_details(serde_json::json!({
                "field": "chapterProgress",
                "chapterProgress": self.chapter_progress,
            }))
        })
    }
}

/// Result data for `reading.progress.update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingProgressUpdateData {
    pub book_id: String,
    pub chapter_index: u32,
    pub chapter_offset: u64,
    #[serde(deserialize_with = "deserialize_chapter_progress")]
    pub chapter_progress: f64,
    #[serde(deserialize_with = "deserialize_reading_progress_stored")]
    pub stored: bool,
}

// ===========================================================================
// RSS vertical (V1 minimal)
// ===========================================================================

fn deserialize_non_blank_feed_xml<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom("rss.parse xml must be non-empty"))
    } else {
        Ok(value)
    }
}

fn deserialize_non_blank_subscription_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom(
            "rss.refresh subscriptionId must be non-empty",
        ))
    } else {
        Ok(value)
    }
}

fn deserialize_required_object<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(de::Error::custom("params field must be an object"))
    }
}

/// Parameters for `rss.parse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssParseParams {
    /// Optional feed URL used to resolve relative links and self-reference guards.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub feed_url: String,
    /// RSS/Atom XML feed text. Must be non-empty.
    #[serde(deserialize_with = "deserialize_non_blank_feed_xml")]
    pub xml: String,
}

/// One entry in the `rss.parse` result. Mirrors the stable fields of
/// `reader_rss::RssEntry`; unknown feed fields are dropped at the protocol
/// boundary to keep the V1 wire shape stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssParseEntryData {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
}

/// Result data for `rss.parse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssParseData {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub entries: Vec<RssParseEntryData>,
}

/// Parameters for `rss.refresh`. Maps directly onto `reader_rss::RssRefreshPolicy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshParams {
    #[serde(deserialize_with = "deserialize_non_blank_subscription_id")]
    pub subscription_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_interval_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<i64>,
    #[serde(default)]
    pub force_refresh: bool,
    #[serde(default)]
    pub evaluated_at: i64,
}

/// Result data for `rss.refresh`. Mirrors `reader_rss::RssRefreshDecision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshData {
    pub subscription_id: String,
    pub should_fetch: bool,
    /// Stable reason code (`disabled`/`forced`/`missingLastFetchedAt`/...).
    pub reason: String,
    pub evaluated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_eligible_fetch_at: Option<i64>,
}

// ===========================================================================
// Sync vertical (V1 minimal)
// ===========================================================================

/// Parameters for `sync.merge`. `local`/`remote` are snapshot objects validated
/// by `reader_sync::SyncSnapshot` at runtime; the contract layer only enforces
/// that they are JSON objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncMergeParams {
    #[serde(deserialize_with = "deserialize_required_object")]
    pub local: Value,
    #[serde(deserialize_with = "deserialize_required_object")]
    pub remote: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub merged_snapshot_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub merged_device_id: String,
    #[serde(default)]
    pub merged_created_at: i64,
}

/// Result data for `sync.merge`. The merged snapshot is returned as a JSON
/// object whose shape is owned by `reader_sync::SyncSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncMergeData {
    pub snapshot: Value,
    #[serde(default)]
    pub conflicts: Vec<Value>,
}

/// Parameters for `sync.backup`. `package`/`policy` are validated by
/// `reader_sync` at runtime; the contract layer only enforces object shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBackupParams {
    #[serde(deserialize_with = "deserialize_required_object")]
    pub package: Value,
    #[serde(deserialize_with = "deserialize_required_object")]
    pub policy: Value,
}

/// Result data for `sync.backup`. The restore plan is returned as a JSON
/// object whose shape is owned by `reader_sync::BackupRestorePlan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBackupData {
    pub plan: Value,
}

// ===========================================================================
// Local-book vertical (V1 minimal)
// ===========================================================================

/// Parameters for `local_book.parse`.
///
/// Two mutually exclusive input modes:
/// * **text** — already-decoded UTF-8 TXT content (legacy V1 path; host owns
///   GBK/GB18030 decoding). Routed to `parse_txt_text`.
/// * **bytesBase64** — base64-encoded raw book bytes for binary formats
///   (EPUB/PDF/MOBI/AZW). Routed to `parse_local_book` which detects format
///   from magic bytes (with optional `format` / `fileName` hint).
///
/// At least one of `text` / `bytesBase64` must be non-empty; validated by
/// [`LocalBookParseParams::validate_local_book_parse_params`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookParseParams {
    #[serde(deserialize_with = "deserialize_non_blank_book_id")]
    pub book_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// Already-decoded TXT content. Used for the legacy text path. Required
    /// when `bytesBase64` is absent; rejected-by-validate when both are
    /// empty/blank.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Base64-encoded raw book bytes for binary formats (EPUB/PDF/MOBI/AZW).
    /// Core decodes here; host never sends raw bytes over the JSON wire.
    /// Required when `text` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_base64: Option<String>,
    /// Optional format hint (e.g. `"epub"`, `"pdf"`, `"mobi"`, `"txt"`).
    /// When present, overrides `fileName` extension for declared-format
    /// detection. If absent, Core falls back to `fileName` extension, then
    /// to magic-byte auto-detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

impl LocalBookParseParams {
    /// Validate that at least one of `text` / `bytesBase64` is non-empty.
    /// Called by the runtime before dispatch; surfaces as `INVALID_PARAMS`.
    pub fn validate_local_book_parse_params(&self) -> Result<(), CoreError> {
        let has_text = !self.text.trim().is_empty();
        let has_bytes = self
            .bytes_base64
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
        if !has_text && !has_bytes {
            return Err(CoreError::invalid_params(
                "local_book.parse requires at least one of `text` or `bytesBase64`",
            ));
        }
        Ok(())
    }

    /// True iff the binary (`bytesBase64`) path should be used.
    pub fn prefers_binary_path(&self) -> bool {
        self.bytes_base64
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    }
}

/// Result data for `local_book.parse`. The parsed book is returned as a JSON
/// object whose shape is owned by `reader_local_book::LocalBook`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookParseData {
    pub book: Value,
    pub format: String,
    pub encoding: String,
    pub byte_len: u64,
    pub char_len: u64,
    pub chapter_count: u32,
}

/// Parameters for `local_book.catalog`. Performs an upsert: replaces any
/// existing entry with the same `stable_book_id` and returns the updated
/// catalog snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookCatalogParams {
    #[serde(deserialize_with = "deserialize_required_object")]
    pub catalog: Value,
    #[serde(deserialize_with = "deserialize_required_object")]
    pub entry: Value,
    #[serde(default)]
    pub chapters: Vec<Value>,
    #[serde(default)]
    pub resources: Vec<Value>,
}

/// Result data for `local_book.catalog`. The updated catalog snapshot is
/// returned as a JSON object whose shape is owned by
/// `reader_local_book::LocalBookCatalogSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookCatalogData {
    pub catalog: Value,
}

/// Helper: parse a typed params object from a `Command`'s free-form params,
/// producing a structured `INVALID_PARAMS` error on failure.
pub fn parse_params<T: for<'de> Deserialize<'de>>(
    method: &str,
    params: &Value,
) -> Result<T, CoreError> {
    serde_json::from_value::<T>(params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {method}")).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": method,
            }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[test]
    fn host_http_response_accepts_metadata_and_extra_fields() {
        let response: HostHttpResponse = serde_json::from_value(serde_json::json!({
            "status": 200,
            "headers": { "content-type": "application/json" },
            "body": "{\"books\":[]}",
            "finalUrl": "https://books.example.test/search",
            "charsetHint": "gbk",
            "bodyBase64": "eyJib29rcyI6W119",
            "session": { "id": "core-session-main" },
            "redirects": [
                {
                    "status": 302,
                    "fromUrl": "https://books.example.test/search",
                    "toUrl": "https://books.example.test/search?q=empty",
                    "headers": { "location": "/search?q=empty" }
                }
            ],
            "cookies": [
                {
                    "name": "sid",
                    "value": "new",
                    "domain": "books.example.test",
                    "path": "/",
                    "httpOnly": true,
                    "secure": true,
                    "sameSite": "Lax"
                }
            ]
        }))
        .unwrap();

        response.validate().unwrap();
        assert_eq!(response.status, Some(200));
        assert_eq!(
            response.headers.unwrap()["content-type"],
            "application/json"
        );
        assert_eq!(
            response.final_url.as_deref(),
            Some("https://books.example.test/search")
        );
        assert_eq!(response.charset_hint.as_deref(), Some("gbk"));
        assert_eq!(response.body_base64.as_deref(), Some("eyJib29rcyI6W119"));
        assert_eq!(
            response.session.as_ref().map(|session| session.id.as_str()),
            Some("core-session-main")
        );
        assert_eq!(
            response.redirects.as_ref().unwrap()[0].to_url,
            "https://books.example.test/search?q=empty"
        );
        assert_eq!(response.cookies.as_ref().unwrap()[0].name, "sid");
    }

    #[test]
    fn host_http_request_defaults_method_and_rejects_blank_method() {
        let request: HostHttpRequest = serde_json::from_value(serde_json::json!({
            "url": "https://books.example.test/search",
            "charset": "gbk",
            "followRedirects": false,
            "maxRedirects": 0,
            "retry": { "maxAttempts": 2, "backoffMillis": 50 },
            "usePlatformCookieJar": false,
            "session": { "id": "core-session-main" }
        }))
        .unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.headers, serde_json::json!({}));
        assert_eq!(request.charset.as_deref(), Some("gbk"));
        assert_eq!(request.follow_redirects, Some(false));
        assert_eq!(request.max_redirects, Some(0));
        assert_eq!(
            request.retry.as_ref().map(|retry| retry.max_attempts),
            Some(2)
        );
        assert_eq!(request.use_platform_cookie_jar, Some(false));
        assert_eq!(
            request.session.as_ref().map(|session| session.id.as_str()),
            Some("core-session-main")
        );
        request.validate().unwrap();

        for method in ["", "   "] {
            let err = serde_json::from_value::<HostHttpRequest>(serde_json::json!({
                "url": "https://books.example.test/search",
                "method": method
            }))
            .unwrap_err();
            assert!(
                err.to_string().contains("method"),
                "unexpected parse error: {err}"
            );
        }
    }

    #[test]
    fn host_http_request_rejects_invalid_retry_and_session() {
        let request: HostHttpRequest = serde_json::from_value(serde_json::json!({
            "url": "https://books.example.test/search",
            "retry": { "maxAttempts": 0 }
        }))
        .unwrap();
        let err = request.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("retry.maxAttempts"));

        let request: HostHttpRequest = serde_json::from_value(serde_json::json!({
            "url": "https://books.example.test/search",
            "session": { "id": "   " }
        }))
        .unwrap();
        let err = request.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("session.id"));
    }

    #[test]
    fn host_http_request_rejects_blank_url() {
        for url in ["", "   "] {
            let err = serde_json::from_value::<HostHttpRequest>(serde_json::json!({
                "url": url
            }))
            .unwrap_err();
            assert!(
                err.to_string().contains("url"),
                "unexpected parse error: {err}"
            );
        }
    }

    #[test]
    fn host_http_request_rejects_non_object_headers() {
        for headers in [
            serde_json::json!(["Accept", "application/json"]),
            serde_json::json!(null),
        ] {
            let err = serde_json::from_value::<HostHttpRequest>(serde_json::json!({
                "url": "https://books.example.test/search",
                "headers": headers
            }))
            .unwrap_err();
            assert!(
                err.to_string().contains("headers"),
                "unexpected parse error: {err}"
            );
        }
    }

    #[test]
    fn host_http_response_rejects_invalid_status() {
        let response: HostHttpResponse = serde_json::from_value(serde_json::json!({
            "status": 99,
            "body": "{\"books\":[]}"
        }))
        .unwrap();

        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("status"));
        assert_eq!(err.details["status"], 99);
    }

    #[test]
    fn host_http_response_rejects_invalid_headers_shape() {
        let response: HostHttpResponse = serde_json::from_value(serde_json::json!({
            "headers": ["content-type", "application/json"],
            "body": "{\"books\":[]}"
        }))
        .unwrap();

        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("headers"));
    }

    #[test]
    fn host_http_response_rejects_invalid_redirect_and_cookie_metadata() {
        let response: HostHttpResponse = serde_json::from_value(serde_json::json!({
            "body": "{\"books\":[]}",
            "redirects": [
                {
                    "status": 200,
                    "fromUrl": "https://books.example.test/search",
                    "toUrl": "https://books.example.test/search?q=empty"
                }
            ]
        }))
        .unwrap();
        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("redirect.status"));

        let response: HostHttpResponse = serde_json::from_value(serde_json::json!({
            "body": "{\"books\":[]}",
            "cookies": [
                {
                    "name": "   ",
                    "value": "new"
                }
            ]
        }))
        .unwrap();
        let err = response.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("cookie.name"));
    }

    #[test]
    fn source_import_params_parse_fixture_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-source-import.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: SourceImportParams =
            parse_params(crate::methods::SOURCE_IMPORT, &command.params)
                .expect("valid source.import params should parse");
        assert_eq!(params.source_id, "conformance-source");
        assert_eq!(params.name, "Conformance Source");
        assert_eq!(params.base_url, "https://books.example.test");
        assert_eq!(params.rules, serde_json::json!({}));
        assert_eq!(params.book_source, serde_json::Value::Null);

        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-source-import-legado-booksource.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: SourceImportParams =
            parse_params(crate::methods::SOURCE_IMPORT, &command.params)
                .expect("valid Legado source.import params should parse");
        assert_eq!(params.source_id, "legado-compat-source");
        assert_eq!(
            params.book_source["ruleSearch"],
            serde_json::json!("div.list&&div.item;div.name&&a@text")
        );
        assert_eq!(
            params.book_source["enabledCookieJar"],
            serde_json::json!(true)
        );
        assert_eq!(
            params.book_source["futureLegadoField"],
            serde_json::json!({
                "nested": true,
                "rawRule": "span.future@text"
            })
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-source-import-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            parse_params::<SourceImportParams>(crate::methods::SOURCE_IMPORT, &command.params)
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::SOURCE_IMPORT);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-source-import-name-whitespace.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            parse_params::<SourceImportParams>(crate::methods::SOURCE_IMPORT, &command.params)
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::SOURCE_IMPORT);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("name")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-source-import-rules-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            parse_params::<SourceImportParams>(crate::methods::SOURCE_IMPORT, &command.params)
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::SOURCE_IMPORT);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("rules") || source.contains("object")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-source-import-booksource-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            parse_params::<SourceImportParams>(crate::methods::SOURCE_IMPORT, &command.params)
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::SOURCE_IMPORT);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("bookSource")),
            "unexpected source detail: {}",
            err.details["source"]
        );
    }

    #[test]
    fn source_import_data_parses_result_and_rejects_invalid_shape() {
        let data: SourceImportData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "name": "Conformance Source",
            "imported": true
        }))
        .unwrap();
        assert_eq!(data.source_id, "conformance-source");
        assert_eq!(data.name, "Conformance Source");
        assert!(data.imported);

        for (label, value, expected) in [
            (
                "sourceId",
                serde_json::json!({
                    "sourceId": " ",
                    "name": "Conformance Source",
                    "imported": true
                }),
                "sourceId",
            ),
            (
                "name",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "name": " ",
                    "imported": true
                }),
                "name",
            ),
            (
                "imported",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "name": "Conformance Source",
                    "imported": false
                }),
                "imported",
            ),
            (
                "unknown field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "name": "Conformance Source",
                    "imported": true,
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<SourceImportData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected source.import data error for {label}: {err}"
            );
        }
    }

    #[test]
    fn book_search_data_parses_result_and_rejects_invalid_shape() {
        let data: BookSearchData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "books": [
                {
                    "bookId": "1",
                    "title": "Dune",
                    "author": "Herbert"
                }
            ]
        }))
        .unwrap();
        assert_eq!(data.source_id, "conformance-source");
        assert_eq!(data.books.len(), 1);
        assert_eq!(data.books[0].book_id, "1");
        assert_eq!(data.books[0].title, "Dune");
        assert_eq!(data.books[0].extra["author"], serde_json::json!("Herbert"));
        assert!(data.http.is_none());

        let data: BookSearchData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "books": [],
            "http": {
                "status": 200,
                "headers": { "content-type": "application/json" }
            }
        }))
        .unwrap();
        let http = data.http.expect("http diagnostics");
        assert_eq!(http.status, Some(200));
        assert_eq!(
            http.headers.unwrap()["content-type"],
            serde_json::json!("application/json")
        );

        for (label, value, expected) in [
            (
                "sourceId",
                serde_json::json!({
                    "sourceId": " ",
                    "books": []
                }),
                "sourceId",
            ),
            (
                "books",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "books": {}
                }),
                "books",
            ),
            (
                "bookId",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "books": [
                        { "title": "Dune" }
                    ]
                }),
                "bookId",
            ),
            (
                "title",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "books": [
                        { "bookId": "1", "title": " " }
                    ]
                }),
                "title",
            ),
            (
                "http.status",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "books": [],
                    "http": { "status": 99 }
                }),
                "status",
            ),
            (
                "http.headers",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "books": [],
                    "http": { "headers": ["content-type", "application/json"] }
                }),
                "headers",
            ),
            (
                "unknown field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "books": [],
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<BookSearchData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected book.search data error for {label}: {err}"
            );
        }
    }

    #[test]
    fn book_detail_data_parses_result_and_rejects_invalid_shape() {
        let data: BookDetailData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "book": {
                "bookId": "1",
                "title": "Dune",
                "author": "Frank Herbert",
                "intro": "desert"
            }
        }))
        .unwrap();
        assert_eq!(data.source_id, "conformance-source");
        assert_eq!(data.book.book_id, "1");
        assert_eq!(data.book.title, "Dune");
        assert_eq!(data.book.author, "Frank Herbert");
        assert_eq!(data.book.intro.as_deref(), Some("desert"));
        assert!(data.http.is_none());

        let data: BookDetailData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "book": {
                "bookId": "1",
                "title": "Dune",
                "author": "Frank Herbert"
            },
            "http": {
                "status": 200,
                "headers": { "content-type": "application/json" }
            }
        }))
        .unwrap();
        let http = data.http.expect("http diagnostics");
        assert_eq!(http.status, Some(200));
        assert_eq!(
            http.headers.unwrap()["content-type"],
            serde_json::json!("application/json")
        );

        for (label, value, expected) in [
            (
                "sourceId",
                serde_json::json!({
                    "sourceId": " ",
                    "book": {
                        "bookId": "1",
                        "title": "Dune",
                        "author": "Frank Herbert"
                    }
                }),
                "sourceId",
            ),
            (
                "book",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "book": []
                }),
                "book",
            ),
            (
                "bookId",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "book": {
                        "bookId": " ",
                        "title": "Dune",
                        "author": "Frank Herbert"
                    }
                }),
                "bookId",
            ),
            (
                "title",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "book": {
                        "bookId": "1",
                        "author": "Frank Herbert"
                    }
                }),
                "title",
            ),
            (
                "unknown book field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "book": {
                        "bookId": "1",
                        "title": "Dune",
                        "author": "Frank Herbert",
                        "extra": true
                    }
                }),
                "unknown field",
            ),
            (
                "http.status",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "book": {
                        "bookId": "1",
                        "title": "Dune",
                        "author": "Frank Herbert"
                    },
                    "http": { "status": 99 }
                }),
                "status",
            ),
            (
                "unknown top-level field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "book": {
                        "bookId": "1",
                        "title": "Dune",
                        "author": "Frank Herbert"
                    },
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<BookDetailData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected book.detail data error for {label}: {err}"
            );
        }
    }

    #[test]
    fn book_toc_data_parses_result_and_rejects_invalid_shape() {
        let data: BookTocData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "bookId": "1",
            "toc": [
                { "index": 0, "title": "C1", "url": "u1" },
                { "index": 1, "title": "C2", "url": "u2" }
            ]
        }))
        .unwrap();
        assert_eq!(data.source_id, "conformance-source");
        assert_eq!(data.book_id, "1");
        assert_eq!(data.toc.len(), 2);
        assert_eq!(data.toc[0].index, 0);
        assert_eq!(data.toc[0].title, "C1");
        assert_eq!(data.toc[0].url, "u1");
        assert!(data.http.is_none());

        let data: BookTocData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "bookId": "1",
            "toc": [],
            "http": {
                "status": 200,
                "headers": { "content-type": "application/json" }
            }
        }))
        .unwrap();
        let http = data.http.expect("http diagnostics");
        assert_eq!(http.status, Some(200));
        assert_eq!(
            http.headers.unwrap()["content-type"],
            serde_json::json!("application/json")
        );

        for (label, value, expected) in [
            (
                "sourceId",
                serde_json::json!({
                    "sourceId": " ",
                    "bookId": "1",
                    "toc": []
                }),
                "sourceId",
            ),
            (
                "bookId",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": " ",
                    "toc": []
                }),
                "bookId",
            ),
            (
                "toc",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "toc": {}
                }),
                "toc",
            ),
            (
                "index",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "toc": [
                        { "title": "C1", "url": "u1" }
                    ]
                }),
                "index",
            ),
            (
                "title",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "toc": [
                        { "index": 0, "url": "u1" }
                    ]
                }),
                "title",
            ),
            (
                "unknown toc field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "toc": [
                        { "index": 0, "title": "C1", "url": "u1", "extra": true }
                    ]
                }),
                "unknown field",
            ),
            (
                "http.status",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "toc": [],
                    "http": { "status": 99 }
                }),
                "status",
            ),
            (
                "unknown top-level field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "toc": [],
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<BookTocData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected book.toc data error for {label}: {err}"
            );
        }
    }

    #[test]
    fn chapter_content_data_parses_result_and_rejects_invalid_shape() {
        let data: ChapterContentData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "bookId": "1",
            "chapterTitle": "C1",
            "content": "Hello\nWorld",
            "via": "rule"
        }))
        .unwrap();
        assert_eq!(data.source_id, "conformance-source");
        assert_eq!(data.book_id, "1");
        assert_eq!(data.chapter_title, "C1");
        assert_eq!(data.content, serde_json::json!("Hello\nWorld"));
        assert_eq!(data.via, ChapterContentVia::Rule);
        assert!(data.http.is_none());

        let data: ChapterContentData = serde_json::from_value(serde_json::json!({
            "sourceId": "conformance-source",
            "bookId": "1",
            "chapterTitle": "C1",
            "content": { "status": "ok", "words": 42 },
            "via": "js",
            "http": {
                "status": 200,
                "headers": { "content-type": "application/json" }
            }
        }))
        .unwrap();
        assert_eq!(data.content["status"], serde_json::json!("ok"));
        assert_eq!(data.via, ChapterContentVia::Js);
        let http = data.http.expect("http diagnostics");
        assert_eq!(http.status, Some(200));
        assert_eq!(
            http.headers.unwrap()["content-type"],
            serde_json::json!("application/json")
        );

        for (label, value, expected) in [
            (
                "sourceId",
                serde_json::json!({
                    "sourceId": " ",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "content": "Hello",
                    "via": "rule"
                }),
                "sourceId",
            ),
            (
                "bookId",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": " ",
                    "chapterTitle": "C1",
                    "content": "Hello",
                    "via": "rule"
                }),
                "bookId",
            ),
            (
                "chapterTitle",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "content": "Hello",
                    "via": "rule"
                }),
                "chapterTitle",
            ),
            (
                "content",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "via": "rule"
                }),
                "content",
            ),
            (
                "via",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "content": "Hello",
                    "via": "native"
                }),
                "unknown variant",
            ),
            (
                "http.status",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "content": "Hello",
                    "via": "rule",
                    "http": { "status": 99 }
                }),
                "status",
            ),
            (
                "unknown top-level field",
                serde_json::json!({
                    "sourceId": "conformance-source",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "content": "Hello",
                    "via": "rule",
                    "extra": true
                }),
                "unknown field",
            ),
        ] {
            let err = serde_json::from_value::<ChapterContentData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected chapter.content data error for {label}: {err}"
            );
        }
    }

    #[test]
    fn book_search_params_parse_fixture_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/commands/valid-book-search.json")
                .as_bytes(),
        )
        .unwrap();
        let params: BookSearchParams = parse_params(crate::methods::BOOK_SEARCH, &command.params)
            .expect("valid book.search params should parse");
        assert_eq!(params.source_id, "conformance-source");
        assert!(params.search_response.contains("\"Dune\""));
        assert!(params.search_request.is_none());
        assert_eq!(
            params.source.as_ref().unwrap()["sourceId"],
            "conformance-source"
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-search-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookSearchParams>(crate::methods::BOOK_SEARCH, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_SEARCH);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-method-empty.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookSearchParams>(crate::methods::BOOK_SEARCH, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_SEARCH);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("method")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-headers-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookSearchParams>(crate::methods::BOOK_SEARCH, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_SEARCH);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("headers")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-search-request-url-whitespace.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookSearchParams>(crate::methods::BOOK_SEARCH, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_SEARCH);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("url")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-search-source-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookSearchParams>(crate::methods::BOOK_SEARCH, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_SEARCH);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("inline source")),
            "unexpected source detail: {}",
            err.details["source"]
        );
    }

    #[test]
    fn book_detail_params_parse_fixture_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/commands/valid-book-detail.json")
                .as_bytes(),
        )
        .unwrap();
        let params: BookDetailParams = parse_params(crate::methods::BOOK_DETAIL, &command.params)
            .expect("valid book.detail params should parse");
        assert_eq!(params.source_id, "conformance-source");
        assert_eq!(params.book["bookId"], "1");
        assert!(params.detail_response.contains("Frank Herbert"));
        assert!(params.detail_request.is_none());
        assert_eq!(
            params.source.as_ref().unwrap()["sourceId"],
            "conformance-source"
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-detail-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookDetailParams>(crate::methods::BOOK_DETAIL, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_DETAIL);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-detail-book-not-object.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<BookDetailParams>(crate::methods::BOOK_DETAIL, &command.params)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_DETAIL);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("book.detail book")),
            "unexpected source detail: {}",
            err.details["source"]
        );
    }

    #[test]
    fn book_toc_params_parse_fixture_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!("../../../protocol/fixtures/conformance/commands/valid-book-toc.json")
                .as_bytes(),
        )
        .unwrap();
        let params: BookTocParams = parse_params(crate::methods::BOOK_TOC, &command.params)
            .expect("valid book.toc params should parse");
        assert_eq!(params.source_id, "conformance-source");
        assert_eq!(params.book_id, "1");
        assert!(params.toc_response.contains("\"C1\""));
        assert!(params.toc_request.is_none());
        assert_eq!(
            params.source.as_ref().unwrap()["sourceId"],
            "conformance-source"
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-book-toc-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            parse_params::<BookTocParams>(crate::methods::BOOK_TOC, &command.params).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::BOOK_TOC);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected source detail: {}",
            err.details["source"]
        );
    }

    #[test]
    fn chapter_content_params_parse_fixture_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-chapter-content.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: ChapterContentParams =
            parse_params(crate::methods::CHAPTER_CONTENT, &command.params)
                .expect("valid chapter.content params should parse");
        assert_eq!(params.source_id, "conformance-source");
        assert_eq!(params.book_id, "1");
        assert_eq!(params.chapter_title, "C1");
        assert!(params.chapter_response.contains("Hello"));
        assert!(params.chapter_request.is_none());
        assert!(params.js_rule.is_none());
        assert_eq!(
            params.source.as_ref().unwrap()["sourceId"],
            "conformance-source"
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-chapter-content-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err =
            parse_params::<ChapterContentParams>(crate::methods::CHAPTER_CONTENT, &command.params)
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(err.details["method"], crate::methods::CHAPTER_CONTENT);
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected source detail: {}",
            err.details["source"]
        );
    }

    #[test]
    fn reading_progress_update_params_parse_fixture_and_reject_unknown_fields() {
        let command: crate::Command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-reading-progress-update.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let params: ReadingProgressUpdateParams =
            parse_params(crate::methods::READING_PROGRESS_UPDATE, &command.params)
                .expect("valid reading.progress.update params should parse");
        assert_eq!(params.book_id, "1");
        assert_eq!(params.chapter_index, 2);
        assert_eq!(params.chapter_offset, 128);
        assert_eq!(params.chapter_progress, 0.5);

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-reading-progress-update-unknown-field.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<ReadingProgressUpdateParams>(
            crate::methods::READING_PROGRESS_UPDATE,
            &command.params,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(
            err.details["method"],
            crate::methods::READING_PROGRESS_UPDATE
        );
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("unknown field")),
            "unexpected source detail: {}",
            err.details["source"]
        );

        let command = crate::Command::from_json_bytes(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-reading-progress-update-progress-out-of-range.json"
            )
            .as_bytes(),
        )
        .unwrap();
        let err = parse_params::<ReadingProgressUpdateParams>(
            crate::methods::READING_PROGRESS_UPDATE,
            &command.params,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(
            err.details["method"],
            crate::methods::READING_PROGRESS_UPDATE
        );
        assert!(
            err.details["source"]
                .as_str()
                .is_some_and(|source| source.contains("chapterProgress")),
            "unexpected source detail: {}",
            err.details["source"]
        );
    }

    #[test]
    fn reading_progress_update_data_parses_result_and_rejects_invalid_shape() {
        let data: ReadingProgressUpdateData = serde_json::from_value(serde_json::json!({
            "bookId": "1",
            "chapterIndex": 2,
            "chapterOffset": 128,
            "chapterProgress": 0.5,
            "stored": true
        }))
        .unwrap();
        assert_eq!(data.book_id, "1");
        assert_eq!(data.chapter_index, 2);
        assert_eq!(data.chapter_offset, 128);
        assert_eq!(data.chapter_progress, 0.5);
        assert!(data.stored);

        for (label, value, expected) in [
            (
                "unknown field",
                serde_json::json!({
                    "bookId": "1",
                    "chapterIndex": 2,
                    "chapterOffset": 128,
                    "chapterProgress": 0.5,
                    "stored": true,
                    "extra": true
                }),
                "unknown field",
            ),
            (
                "progress",
                serde_json::json!({
                    "bookId": "1",
                    "chapterIndex": 2,
                    "chapterOffset": 128,
                    "chapterProgress": 1.5,
                    "stored": true
                }),
                "chapterProgress",
            ),
            (
                "stored",
                serde_json::json!({
                    "bookId": "1",
                    "chapterIndex": 2,
                    "chapterOffset": 128,
                    "chapterProgress": 0.5,
                    "stored": false
                }),
                "stored",
            ),
        ] {
            let err = serde_json::from_value::<ReadingProgressUpdateData>(value)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {label}"));
            assert!(
                err.to_string().contains(expected),
                "unexpected reading.progress.update data error for {label}: {err}"
            );
        }
    }
}

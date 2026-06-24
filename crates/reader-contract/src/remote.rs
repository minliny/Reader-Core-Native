//! Typed params for the remote-reading vertical commands.
//!
//! These mirror the V1 "minimal vertical" pipeline: source import → search →
//! detail → toc → chapter → progress. Each command can take a prefetched
//! response body for deterministic tests, or a host HTTP request descriptor
//! that Core emits as `capability: "http.execute"` (see
//! `protocol/compatibility.md`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CoreError;

fn empty_string() -> String {
    String::new()
}

fn default_http_method() -> String {
    "GET".to_string()
}

/// Host HTTP request description emitted as `capability: "http.execute"`.
///
/// Core owns request semantics; platform hosts own the actual socket/TLS stack
/// and answer with `host.complete { result: { body: "..." } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHttpRequest {
    pub url: String,
    #[serde(default = "default_http_method")]
    pub method: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub body: Option<String>,
}

/// Parameters for `source.import`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceImportParams {
    /// Stable source identifier. If omitted, one is assigned.
    #[serde(default = "empty_string")]
    pub source_id: String,
    pub name: String,
    #[serde(default = "empty_string")]
    pub base_url: String,
    /// Extraction rules keyed by stage (`search`/`detail`/`toc`/`chapter`).
    /// Each value is a JSON array of rule-step specs understood by
    /// `reader-content`.
    #[serde(default)]
    pub rules: Value,
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
    #[serde(default)]
    pub source: Option<Value>,
}

/// Parameters for `book.detail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookDetailParams {
    pub source_id: String,
    /// Base book to merge metadata into (must contain at least `bookId`).
    pub book: Value,
    /// Pre-fetched detail response body.
    #[serde(default = "empty_string")]
    pub detail_response: String,
    #[serde(default)]
    pub detail_request: Option<HostHttpRequest>,
    #[serde(default)]
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
    #[serde(default)]
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
    #[serde(default)]
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
    #[serde(default)]
    pub chapter_progress: f64,
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

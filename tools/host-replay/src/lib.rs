//! Offline replay engine for `host.request` responses.
//!
//! This crate is **dev-time only**. It reads local replay fixtures and emits
//! JSON commands shaped like the Reader-Core `host.complete` / `host.error`
//! protocol commands. It never opens a socket and never modifies the protocol
//! schema — it only produces the JSON envelope a real host adapter would send.
//!
//! See `samples/host-replay/FORMAT.md` for the fixture format reference.
//!
//! ## Fixture format (`reader-host-replay/1`)
//!
//! A fixture is a single JSON object:
//!
//! ```jsonc
//! {
//!   "format": "reader-host-replay/1",
//!   "description": "...",
//!   "request": {
//!     "id": 502,                 // requestId label (the Core command blocked on this op)
//!     "operationId": 1,          // host operation id label
//!     "capability": "http.execute",
//!     "url": "https://example.test/search?q=dune",
//!     "urlPattern": null,        // optional wildcard pattern, overrides `url` for matching
//!     "method": "GET",
//!     "headers": { "Accept": "application/json" },
//!     "body": null
//!   },
//!   "response": {
//!     "status": 200,
//!     "headers": { "content-type": "application/json" },
//!     "body": "{\"books\":[]}",  // inline body string
//!     "bodyFile": null,          // OR sibling file path (text or base64)
//!     "bodyEncoding": "text",    // "text" | "base64"
//!     "bodyBase64": null,        // OR raw base64 (emitted as result.bodyBase64)
//!     "finalUrl": null,          // final URL after redirects
//!     "charsetHint": null
//!   },
//!   "redirectChain": [],         // optional: [{status, location, headers, setCookies}]
//!   "cookieJar": {},             // optional: { "<origin>": [Cookie, ...] }
//!   "outcome": "complete",       // "complete" | "error"
//!   "error": null,               // for outcome="error": {code, message, retryable, details}
//!   "tags": []
//! }
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Top-level replay fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub format: String,
    #[serde(default)]
    pub description: String,
    pub request: ReplayRequest,
    pub response: Option<ReplayResponse>,
    #[serde(default)]
    pub redirect_chain: Vec<RedirectStep>,
    #[serde(default)]
    pub cookie_jar: CookieJar,
    #[serde(default = "default_outcome")]
    pub outcome: Outcome,
    #[serde(default)]
    pub error: Option<ReplayError>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_outcome() -> Outcome {
    Outcome::Complete
}

/// The recorded `host.request` parameters (the request Core would have sent).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayRequest {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub operation_id: Option<u64>,
    #[serde(default = "default_capability")]
    pub capability: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub url_pattern: Option<String>,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub body: Value,
}

fn default_capability() -> String {
    "http.execute".to_string()
}

fn default_method() -> String {
    "GET".to_string()
}

/// The recorded host response (becomes `host.complete` result).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_file: Option<String>,
    #[serde(default = "default_body_encoding")]
    pub body_encoding: BodyEncoding,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub final_url: Option<String>,
    #[serde(default)]
    pub charset_hint: Option<String>,
}

fn default_body_encoding() -> BodyEncoding {
    BodyEncoding::Text
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BodyEncoding {
    Text,
    Base64,
}

/// One hop in a redirect chain (3xx → Location).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedirectStep {
    pub status: u16,
    pub location: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub set_cookies: Vec<String>,
}

/// Cookie jar snapshot keyed by origin (`"https://example.test"`).
pub type CookieJar = BTreeMap<String, Vec<Cookie>>;

/// A single cookie in a jar snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub same_site: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Complete,
    Error,
}

/// A transport-layer error payload (becomes `host.error` params.error).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub details: Value,
}

/// Errors produced by the replay engine.
#[derive(Debug)]
pub enum ReplayErrorKind {
    Io(String),
    Json(String),
    Invalid(String),
}

impl std::fmt::Display for ReplayErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayErrorKind::Io(m) => write!(f, "io: {m}"),
            ReplayErrorKind::Json(m) => write!(f, "json: {m}"),
            ReplayErrorKind::Invalid(m) => write!(f, "invalid: {m}"),
        }
    }
}

impl std::error::Error for ReplayErrorKind {}

/// Load a single fixture from a JSON file.
pub fn load_fixture(path: &Path) -> Result<Fixture, ReplayErrorKind> {
    let raw = fs::read_to_string(path)
        .map_err(|e| ReplayErrorKind::Io(format!("read {}: {e}", path.display())))?;
    let fixture: Fixture = serde_json::from_str(&raw).map_err(|e| {
        ReplayErrorKind::Json(format!("parse {}: {e}", path.display()))
    })?;
    if !fixture.format.starts_with("reader-host-replay/") {
        return Err(ReplayErrorKind::Invalid(format!(
            "{}: unsupported format {:?} (expected \"reader-host-replay/1\")",
            path.display(),
            fixture.format
        )));
    }
    Ok(fixture)
}

/// Load every `*.json` fixture directly under a directory (non-recursive).
///
/// Files that are valid JSON but do not declare a `reader-host-replay/*`
/// `format` (e.g. co-located response-body files) are silently skipped, so
/// body files can live next to their fixtures.
pub fn load_fixture_dir(dir: &Path) -> Result<Vec<(PathBuf, Fixture)>, ReplayErrorKind> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|e| ReplayErrorKind::Io(format!("read dir {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| ReplayErrorKind::Io(format!("dir entry: {e}")))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|e| ReplayErrorKind::Io(format!("read {}: {e}", path.display())))?;
        let probe: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue, // not JSON — skip silently
        };
        let is_fixture = probe
            .get("format")
            .and_then(|v| v.as_str())
            .map(|s| s.starts_with("reader-host-replay/"))
            .unwrap_or(false);
        if !is_fixture {
            continue;
        }
        let fixture: Fixture = serde_json::from_str(&raw).map_err(|e| {
            ReplayErrorKind::Json(format!("parse {}: {e}", path.display()))
        })?;
        out.push((path, fixture));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Resolve the response body to a JSON value suitable for the `result` object.
///
/// Returns one of:
/// - `None` — no body
/// - `Some(("body", String))` — text body → `result.body`
/// - `Some(("bodyBase64", String))` — binary body → `result.bodyBase64`
pub fn resolve_body(
    response: &ReplayResponse,
    fixture_dir: &Path,
) -> Result<Option<(&'static str, String)>, ReplayErrorKind> {
    if let Some(b64) = &response.body_base64 {
        return Ok(Some(("bodyBase64", b64.clone())));
    }
    if let Some(file) = &response.body_file {
        let body_path = fixture_dir.join(file);
        let bytes = fs::read(&body_path)
            .map_err(|e| ReplayErrorKind::Io(format!("read body {}: {e}", body_path.display())))?;
        match response.body_encoding {
            BodyEncoding::Text => {
                let text = String::from_utf8(bytes).map_err(|e| {
                    ReplayErrorKind::Invalid(format!(
                        "body file {} is not valid UTF-8 (use bodyEncoding=\"base64\" for binary): {e}",
                        body_path.display()
                    ))
                })?;
                Ok(Some(("body", text)))
            }
            BodyEncoding::Base64 => {
                Ok(Some(("bodyBase64", base64_encode(&bytes))))
            }
        }
    } else if let Some(body) = &response.body {
        Ok(Some(("body", body.clone())))
    } else {
        Ok(None)
    }
}

/// Compute the effective `finalUrl` for a fixture: explicit `response.finalUrl`,
/// else the last redirect hop's location, else the request url.
pub fn effective_final_url(fixture: &Fixture) -> String {
    if let Some(resp) = &fixture.response {
        if let Some(url) = &resp.final_url {
            return url.clone();
        }
    }
    if let Some(last) = fixture.redirect_chain.last() {
        return last.location.clone();
    }
    fixture.request.url.clone()
}

/// Build the `host.complete` `params.result` JSON value for a fixture.
pub fn build_result(
    fixture: &Fixture,
    fixture_dir: &Path,
) -> Result<Value, ReplayErrorKind> {
    let response = fixture
        .response
        .as_ref()
        .ok_or_else(|| ReplayErrorKind::Invalid("outcome=complete but response is missing".into()))?;

    let mut result = Map::new();
    result.insert("status".into(), json!(response.status));

    if !response.headers.is_null() {
        result.insert("headers".into(), response.headers.clone());
    }

    if let Some((key, body)) = resolve_body(response, fixture_dir)? {
        result.insert(key.into(), Value::String(body));
    }

    let final_url = effective_final_url(fixture);
    // Only emit finalUrl when it differs from the request url (i.e. a redirect
    // happened or the fixture explicitly recorded one).
    if final_url != fixture.request.url || response.final_url.is_some() {
        result.insert("finalUrl".into(), Value::String(final_url));
    }

    if let Some(hint) = &response.charset_hint {
        result.insert("charsetHint".into(), Value::String(hint.clone()));
    }

    Ok(Value::Object(result))
}

/// Build a full `host.complete` command envelope.
///
/// `request_id` is the host-chosen command id. `operation_id` correlates to
/// the pending host operation (normally the incoming host.request's operationId).
pub fn build_complete_command(
    fixture: &Fixture,
    fixture_dir: &Path,
    request_id: u64,
    operation_id: u64,
) -> Result<Value, ReplayErrorKind> {
    let result = build_result(fixture, fixture_dir)?;
    Ok(json!({
        "protocolVersion": 1,
        "requestId": request_id,
        "method": "host.complete",
        "params": {
            "operationId": operation_id,
            "result": result,
        }
    }))
}

/// Build a full `host.error` command envelope.
pub fn build_error_command(
    fixture: &Fixture,
    request_id: u64,
    operation_id: u64,
) -> Result<Value, ReplayErrorKind> {
    let err = fixture
        .error
        .as_ref()
        .ok_or_else(|| ReplayErrorKind::Invalid("outcome=error but error is missing".into()))?;
    let mut error = Map::new();
    error.insert("code".into(), Value::String(err.code.clone()));
    error.insert("message".into(), Value::String(err.message.clone()));
    error.insert("retryable".into(), json!(err.retryable));
    if !err.details.is_null() {
        error.insert("details".into(), err.details.clone());
    }
    Ok(json!({
        "protocolVersion": 1,
        "requestId": request_id,
        "method": "host.error",
        "params": {
            "operationId": operation_id,
            "error": Value::Object(error),
        }
    }))
}

/// Build the appropriate command (complete or error) for a fixture.
pub fn build_command(
    fixture: &Fixture,
    fixture_dir: &Path,
    request_id: u64,
    operation_id: u64,
) -> Result<Value, ReplayErrorKind> {
    match fixture.outcome {
        Outcome::Complete => build_complete_command(fixture, fixture_dir, request_id, operation_id),
        Outcome::Error => build_error_command(fixture, request_id, operation_id),
    }
}

/// A parsed incoming `host.request` event (what Core sends to the host).
#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub request_id: u64,
    pub operation_id: u64,
    pub capability: String,
    pub params: Value,
}

/// Parse a `host.request` event JSON line.
pub fn parse_incoming(line: &str) -> Result<IncomingRequest, ReplayErrorKind> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| ReplayErrorKind::Json(format!("parse host.request line: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| ReplayErrorKind::Invalid("host.request is not an object".into()))?;
    let request_id = obj
        .get("requestId")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ReplayErrorKind::Invalid("host.request missing requestId".into()))?;
    let operation_id = obj
        .get("operationId")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ReplayErrorKind::Invalid("host.request missing operationId".into()))?;
    let capability = obj
        .get("capability")
        .and_then(|v| v.as_str())
        .unwrap_or("http.execute")
        .to_string();
    let params = obj
        .get("params")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    Ok(IncomingRequest {
        request_id,
        operation_id,
        capability,
        params,
    })
}

/// Decide whether a fixture matches an incoming host.request.
///
/// Matching precedence:
/// 1. If the fixture has a `urlPattern`, wildcard-match the incoming url.
/// 2. Otherwise exact-normalized (method, url) match.
///
/// `request.id` / `request.operationId` are recorded labels only — the live
/// correlation key is the incoming `operationId`, which is echoed back into the
/// emitted command. They are intentionally NOT used for matching.
pub fn matches(fixture: &Fixture, incoming: &IncomingRequest) -> bool {
    if fixture.request.capability != incoming.capability {
        return false;
    }
    let incoming_url = incoming.params.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let incoming_method = incoming
        .params
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");

    if let Some(pattern) = &fixture.request.url_pattern {
        return wildcard_match(pattern, incoming_url)
            && method_eq(&fixture.request.method, incoming_method);
    }

    if !fixture.request.url.is_empty() {
        return normalize_url(&fixture.request.url) == normalize_url(incoming_url)
            && method_eq(&fixture.request.method, incoming_method);
    }

    // No url to match on: fall back to capability-only match.
    true
}

/// Case-insensitive method comparison.
pub fn method_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Normalize a URL for stable matching: lowercase scheme+host, drop trailing
/// slash on the path root, sort query keys. Keeps it simple and deterministic.
pub fn normalize_url(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => return url.to_string(),
    };
    let (authority, mut path_query) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, String::new()),
    };
    let authority = authority.to_ascii_lowercase();
    if path_query == "/" {
        path_query.clear();
    }
    let (path, query) = match path_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_query.as_str(), None),
    };
    let query = query.map(sort_query);
    match query {
        Some(q) => format!("{scheme}://{authority}{path}?{q}"),
        None => format!("{scheme}://{authority}{path}"),
    }
}

fn sort_query(query: &str) -> String {
    let mut pairs: Vec<&str> = query.split('&').collect();
    pairs.sort();
    pairs.join("&")
}

/// Shell-style wildcard match: `*` matches any sequence, `?` matches one char.
pub fn wildcard_match(pattern: &str, input: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = input.chars().collect();
    let (m, n) = (p.len(), s.len());
    // DP wildcard matching.
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;
    for i in 1..=m {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=m {
        for j in 1..=n {
            if p[i - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if p[i - 1] == '?' || p[i - 1] == s[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[m][n]
}

/// Extract `Set-Cookie` values from a response headers JSON value.
///
/// Headers may be a string or an array of strings (multi-valued). Returns all
/// `Set-Cookie` values (case-insensitive header name).
pub fn extract_set_cookies(headers: &Value) -> Vec<String> {
    let Some(obj) = headers.as_object() else {
        return Vec::new();
    };
    for (k, v) in obj {
        if k.eq_ignore_ascii_case("set-cookie") {
            return match v {
                Value::String(s) => vec![s.clone()],
                Value::Array(arr) => arr
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            };
        }
    }
    Vec::new()
}

/// Parse a `Set-Cookie` header into a `Cookie`, deriving domain from origin.
pub fn parse_set_cookie(header: &str, origin: &str) -> Cookie {
    let mut iter = header.split(';');
    let nv = iter.next().unwrap_or("").trim();
    let (name, value) = match nv.split_once('=') {
        Some((n, v)) => (n.trim().to_string(), v.trim().to_string()),
        None => (nv.to_string(), String::new()),
    };
    let mut cookie = Cookie {
        name,
        value,
        domain: origin_host(origin),
        path: Some("/".to_string()),
        secure: origin.starts_with("https://"),
        http_only: false,
        expires: None,
        same_site: None,
    };
    for attr in iter {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        let (k, v) = match attr.split_once('=') {
            Some((k, v)) => (k.trim(), Some(v.trim())),
            None => (attr, None),
        };
        match k.to_ascii_lowercase().as_str() {
            "domain" => cookie.domain = v.map(|s| s.trim_start_matches('.').to_string()),
            "path" => cookie.path = v.map(String::from),
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "expires" => cookie.expires = v.map(String::from),
            "samesite" => cookie.same_site = v.map(String::from),
            _ => {}
        }
    }
    cookie
}

fn origin_host(origin: &str) -> Option<String> {
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"))?;
    let host = rest.split('/').next().unwrap_or(rest);
    let host = host.split(':').next().unwrap_or(host);
    Some(host.to_string())
}

/// Origin (`scheme://host`) of a URL, for cookie-jar keying.
pub fn url_origin(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s, r),
        None => return String::new(),
    };
    let host = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{host}")
}

/// Merge a list of `Set-Cookie` headers into a jar under the request origin.
pub fn merge_set_cookies(jar: &mut CookieJar, origin: &str, headers: &[String]) {
    let bucket = jar.entry(origin.to_string()).or_default();
    for h in headers {
        let cookie = parse_set_cookie(h, origin);
        // Replace existing cookie with same name+path.
        bucket.retain(|c| !(c.name == cookie.name && c.path == cookie.path));
        bucket.push(cookie);
    }
}

/// Render a cookie jar as a `Cookie:` request header value for an origin.
pub fn cookie_header(jar: &CookieJar, origin: &str) -> Option<String> {
    let bucket = jar.get(origin)?;
    if bucket.is_empty() {
        return None;
    }
    let pairs: Vec<String> = bucket.iter().map(|c| format!("{}={}", c.name, c.value)).collect();
    Some(pairs.join("; "))
}

/// A trace of the redirect chain for diagnostics (emitted to stderr with `--trace`).
#[derive(Debug, Serialize)]
pub struct RedirectTrace {
    pub request_url: String,
    pub steps: Vec<RedirectStepSummary>,
    pub final_url: String,
    pub final_status: u16,
}

#[derive(Debug, Serialize)]
pub struct RedirectStepSummary {
    pub status: u16,
    pub location: String,
}

/// Build a redirect trace for a fixture.
pub fn redirect_trace(fixture: &Fixture) -> Option<RedirectTrace> {
    if fixture.redirect_chain.is_empty() {
        return None;
    }
    let steps: Vec<RedirectStepSummary> = fixture
        .redirect_chain
        .iter()
        .map(|s| RedirectStepSummary {
            status: s.status,
            location: s.location.clone(),
        })
        .collect();
    let final_status = fixture.response.as_ref().map(|r| r.status).unwrap_or(0);
    Some(RedirectTrace {
        request_url: fixture.request.url.clone(),
        steps,
        final_url: effective_final_url(fixture),
        final_status,
    })
}

/// Validate a fixture's internal consistency (used by `validate` command + tests).
pub fn validate(fixture: &Fixture) -> Result<(), ReplayErrorKind> {
    match fixture.outcome {
        Outcome::Complete => {
            if fixture.response.is_none() {
                return Err(ReplayErrorKind::Invalid(
                    "outcome=complete requires a `response`".into(),
                ));
            }
        }
        Outcome::Error => {
            if fixture.error.is_none() {
                return Err(ReplayErrorKind::Invalid(
                    "outcome=error requires an `error`".into(),
                ));
            }
        }
    }
    for (i, step) in fixture.redirect_chain.iter().enumerate() {
        if !(300..400).contains(&step.status) {
            return Err(ReplayErrorKind::Invalid(format!(
                "redirectChain[{i}] status {} is not a 3xx",
                step.status
            )));
        }
        if step.location.is_empty() {
            return Err(ReplayErrorKind::Invalid(format!(
                "redirectChain[{i}] missing location",
            )));
        }
    }
    Ok(())
}

// Minimal base64 encoder (std has no base64; we avoid an extra dep for a dev tool).
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx_complete() -> Fixture {
        let raw = r#"{
            "format": "reader-host-replay/1",
            "description": "search",
            "request": {
                "id": 502, "operationId": 1, "capability": "http.execute",
                "url": "https://example.test/search?q=dune", "method": "GET",
                "headers": {"Accept": "application/json"}, "body": null
            },
            "response": {
                "status": 200,
                "headers": {"content-type": "application/json"},
                "body": "{\"books\":[]}"
            }
        }"#;
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn builds_complete_envelope_shape() {
        let f = fx_complete();
        let cmd = build_complete_command(&f, Path::new("."), 7, 9).unwrap();
        assert_eq!(cmd["method"], "host.complete");
        assert_eq!(cmd["protocolVersion"], 1);
        assert_eq!(cmd["requestId"], 7);
        assert_eq!(cmd["params"]["operationId"], 9);
        assert_eq!(cmd["params"]["result"]["status"], 200);
        assert_eq!(cmd["params"]["result"]["body"], "{\"books\":[]}");
        // finalUrl omitted when no redirect and not explicitly set.
        assert!(cmd["params"]["result"].get("finalUrl").is_none());
    }

    #[test]
    fn matches_by_normalized_url_and_method() {
        let f = fx_complete();
        let incoming = IncomingRequest {
            request_id: 502,
            operation_id: 1,
            capability: "http.execute".into(),
            params: json!({"url": "https://EXAMPLE.test/search?q=dune", "method": "get"}),
        };
        assert!(matches(&f, &incoming));
    }

    #[test]
    fn wildcard_pattern_matches() {
        let mut f = fx_complete();
        f.request.url_pattern = Some("https://example.test/*".into());
        let incoming = IncomingRequest {
            request_id: 1,
            operation_id: 1,
            capability: "http.execute".into(),
            params: json!({"url": "https://example.test/anything?x=1", "method": "GET"}),
        };
        assert!(matches(&f, &incoming));
    }

    #[test]
    fn error_outcome_builds_host_error() {
        let raw = r#"{
            "format": "reader-host-replay/1",
            "request": {"url": "https://x.test/", "method": "GET"},
            "outcome": "error",
            "error": {"code": "HTTP_TRANSPORT_TIMEOUT", "message": "timed out", "retryable": true, "details": {"phase": "connect"}}
        }"#;
        let f: Fixture = serde_json::from_str(raw).unwrap();
        let cmd = build_error_command(&f, 3, 5).unwrap();
        assert_eq!(cmd["method"], "host.error");
        assert_eq!(cmd["params"]["operationId"], 5);
        assert_eq!(cmd["params"]["error"]["code"], "HTTP_TRANSPORT_TIMEOUT");
        assert_eq!(cmd["params"]["error"]["retryable"], true);
        assert_eq!(cmd["params"]["error"]["details"]["phase"], "connect");
    }

    #[test]
    fn redirect_chain_sets_final_url() {
        let mut f = fx_complete();
        f.redirect_chain = vec![RedirectStep {
            status: 302,
            location: "https://example.test/final".into(),
            headers: json!({}),
            set_cookies: vec![],
        }];
        let cmd = build_complete_command(&f, Path::new("."), 1, 1).unwrap();
        assert_eq!(cmd["params"]["result"]["finalUrl"], "https://example.test/final");
    }

    #[test]
    fn set_cookie_parsing_and_jar_merge() {
        let mut jar = CookieJar::new();
        merge_set_cookies(
            &mut jar,
            "https://example.test",
            &["sid=abc; Path=/; HttpOnly; Secure".into()],
        );
        let bucket = &jar["https://example.test"];
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].name, "sid");
        assert_eq!(bucket[0].value, "abc");
        assert!(bucket[0].http_only);
        assert!(bucket[0].secure);
        let header = cookie_header(&jar, "https://example.test").unwrap();
        assert_eq!(header, "sid=abc");
    }

    #[test]
    fn validate_rejects_inconsistent_fixture() {
        let mut f = fx_complete();
        f.outcome = Outcome::Error;
        assert!(validate(&f).is_err());
    }

    #[test]
    fn base64_encoding_roundtrip_shape() {
        let encoded = base64_encode(b"hi");
        assert_eq!(encoded, "aGk=");
    }

    #[test]
    fn camel_case_fields_deserialize() {
        // Regression: all multi-word fields use camelCase in fixtures and must
        // deserialize into the snake_case Rust fields.
        let raw = r#"{
            "format": "reader-host-replay/1",
            "request": {
                "operationId": 7, "urlPattern": "https://x.test/*",
                "url": "https://x.test/a", "method": "GET"
            },
            "response": {
                "status": 200,
                "headers": {"Set-Cookie": ["a=1; Path=/"]},
                "bodyFile": "body.txt",
                "bodyEncoding": "base64",
                "bodyBase64": null,
                "finalUrl": "https://x.test/b",
                "charsetHint": "utf-8"
            },
            "redirectChain": [
                {"status": 302, "location": "https://x.test/b", "setCookies": ["a=1"]}
            ],
            "cookieJar": {
                "https://x.test": [{"name": "a", "value": "1", "httpOnly": true, "sameSite": "Lax"}]
            },
            "error": {"code": "X", "message": "m", "retryable": false}
        }"#;
        let f: Fixture = serde_json::from_str(raw).unwrap();
        assert_eq!(f.request.operation_id, Some(7));
        assert_eq!(f.request.url_pattern.as_deref(), Some("https://x.test/*"));
        assert_eq!(f.response.as_ref().unwrap().body_file.as_deref(), Some("body.txt"));
        assert_eq!(f.response.as_ref().unwrap().body_encoding, BodyEncoding::Base64);
        assert_eq!(f.response.as_ref().unwrap().final_url.as_deref(), Some("https://x.test/b"));
        assert_eq!(f.response.as_ref().unwrap().charset_hint.as_deref(), Some("utf-8"));
        assert_eq!(f.redirect_chain.len(), 1);
        assert_eq!(f.redirect_chain[0].set_cookies, vec!["a=1".to_string()]);
        assert!(f.cookie_jar.contains_key("https://x.test"));
        assert!(f.cookie_jar["https://x.test"][0].http_only);
        assert_eq!(f.cookie_jar["https://x.test"][0].same_site.as_deref(), Some("Lax"));
        assert_eq!(f.error.as_ref().unwrap().code, "X");
    }

    #[test]
    fn load_fixture_dir_skips_non_fixture_json() {
        let dir = std::env::temp_dir().join("host-replay-dir-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("real.json"),
            r#"{"format":"reader-host-replay/1","request":{"url":"https://a.test/","method":"GET"},"response":{"status":200}}"#,
        )
        .unwrap();
        // Co-located body file: valid JSON, no `format` — must be skipped.
        std::fs::write(dir.join("body.json"), r#"{"token":"x"}"#).unwrap();
        let loaded = load_fixture_dir(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0.file_name().unwrap(), "real.json");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

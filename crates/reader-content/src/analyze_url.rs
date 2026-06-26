//! Legado AnalyzeUrl-equivalent request descriptor builder.
//!
//! Ports Swift `URLDSLParser.swift` + `BookSourceRequestBuilder.swift` into
//! Rust. The module owns:
//! - URL DSL parsing (`url, {"method":"POST",...}`) — see [`UrlDslParser`]
//! - Static template expansion (`{{key}}`/`{{page}}`/`pageMinus`/`pagePlus`)
//! - URL-embedded JS classification + evaluation (via `reader-js`)
//! - `HostHttpRequest` assembly (added in Task 3)
//!
//! Core produces descriptors; Host executes HTTP. Core opens no socket.

use reader_contract::remote::{HostHttpRequest, HostHttpRetryPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Parsed JSON options from a Legado URL DSL string.
///
/// Format: `url, {"method":"POST","body":"...","headers":{...},"charset":"gbk"}`.
/// Mirrors Swift `URLDSLOptions` and Legado `AnalyzeUrl.UrlOption`.
///
/// Deferred Legado fields (`type`, `webView`, `webJs`, `webViewDelayTime`,
/// `origin`, `serverID`, `timeout`) are silently dropped by serde — they are
/// tracked in the audit as host-responsibility or future-work items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlDslOptions {
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default = "default_charset")]
    pub charset: String,
    #[serde(default)]
    pub headers: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Legado `retry: Int?` — nullable integer. `None` means no retry override.
    #[serde(default)]
    pub retry: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_redirects: Option<bool>,
    /// DSL `js` option — post-parse JS that evaluates to the final URL.
    /// Executed by [`AnalyzeUrl::build_request_with_js`] (Task 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js: Option<String>,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_charset() -> String {
    "utf-8".to_string()
}

impl Default for UrlDslOptions {
    fn default() -> Self {
        Self {
            method: default_method(),
            charset: default_charset(),
            headers: Map::new(),
            body: None,
            retry: None,
            follow_redirects: None,
            js: None,
        }
    }
}

/// Classification of JS expressions found in a URL DSL string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsExpressionClassification {
    None,
    SafeExpression,
    RequiresJsSandbox,
}

/// Result of parsing a Legado URL DSL string.
#[derive(Debug, Clone, PartialEq)]
pub struct UrlDslResult {
    pub url: String,
    pub options: UrlDslOptions,
    pub has_js_expression: bool,
    pub js_expression: Option<String>,
    pub js_classification: JsExpressionClassification,
}

impl UrlDslResult {
    fn new(url: String, options: UrlDslOptions) -> Self {
        let (js_expression, js_classification) = classify_js_expression(&url);
        Self {
            has_js_expression: js_expression.is_some(),
            js_expression,
            js_classification,
            url,
            options,
        }
    }
}

/// Errors raised by URL DSL parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlDslParseError {
    MalformedJson(String),
}

impl std::fmt::Display for UrlDslParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlDslParseError::MalformedJson(detail) => {
                write!(f, "malformed URL DSL JSON: {detail}")
            }
        }
    }
}

impl std::error::Error for UrlDslParseError {}

/// Legado URL DSL parser. Ports Swift `URLDSLParser.parse(_:)`.
pub struct UrlDslParser;

impl UrlDslParser {
    /// Parse a Legado URL DSL string into [`UrlDslResult`].
    ///
    /// Grammar (mirrors Legado `AnalyzeUrl.initUrl`):
    /// - Empty string → empty URL, default options.
    /// - `METHOD,url` (legacy) → whole string is URL, default options.
    /// - `url` (no comma+brace) → plain URL, default options.
    /// - `url, {"method":"POST",...}` → split on the comma followed by `{`,
    ///   parse JSON options with Legado quirks (single quotes, semicolons).
    pub fn parse(raw: &str) -> Result<UrlDslResult, UrlDslParseError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(UrlDslResult::new(String::new(), UrlDslOptions::default()));
        }

        if has_legacy_method_prefix(trimmed) {
            return Ok(UrlDslResult::new(
                trimmed.to_string(),
                UrlDslOptions::default(),
            ));
        }

        let Some(comma_index) = find_dsl_separator(trimmed) else {
            return Ok(UrlDslResult::new(
                trimmed.to_string(),
                UrlDslOptions::default(),
            ));
        };

        let url_part = trimmed[..comma_index].trim();
        let options_part = trimmed[comma_index + 1..].trim();

        if options_part.is_empty() {
            return Ok(UrlDslResult::new(
                url_part.to_string(),
                UrlDslOptions::default(),
            ));
        }

        // find_dsl_separator already guarantees options_part starts with '{',
        // but double-check defensively.
        if !options_part.starts_with('{') {
            return Ok(UrlDslResult::new(
                trimmed.to_string(),
                UrlDslOptions::default(),
            ));
        }

        let normalized = normalize_legado_json(options_part);
        let options: UrlDslOptions = serde_json::from_str(&normalized)
            .map_err(|err| UrlDslParseError::MalformedJson(format!("{err}: {options_part}")))?;
        Ok(UrlDslResult::new(url_part.to_string(), options))
    }

    /// Classify a URL for embedded JS expressions. Public so callers can
    /// re-classify after mutating the URL.
    pub fn classify_js_expression(url: &str) -> (Option<String>, JsExpressionClassification) {
        classify_js_expression(url)
    }
}

/// Detect Legado legacy method prefix: `GET,url` / `POST,url` etc. The whole
/// string is then treated as the URL (method is implicit). Ports Swift
/// `hasLegacyMethodPrefix`.
fn has_legacy_method_prefix(raw: &str) -> bool {
    let Some(comma_index) = raw.find(',') else {
        return false;
    };
    let method = raw[..comma_index].trim().to_ascii_uppercase();
    matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS"
    )
}

/// Find the comma that separates URL from JSON options. Mirrors Legado
/// `paramPattern = \s*,\s*(?=\{)`: the comma must be outside quotes/brackets
/// AND followed by optional whitespace + `{`.
fn find_dsl_separator(input: &str) -> Option<usize> {
    let mut in_double = false;
    let mut in_single = false;
    let mut bracket_depth: i32 = 0;
    for (idx, ch) in input.char_indices() {
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '[' if !in_double && !in_single => bracket_depth += 1,
            ']' if !in_double && !in_single => bracket_depth -= 1,
            ',' if !in_double && !in_single && bracket_depth == 0 => {
                let rest = &input[idx + 1..];
                let trimmed = rest.trim_start();
                if trimmed.starts_with('{') {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

/// Normalize Legado JSON quirks: single quotes → double quotes, semicolons
/// between key-value pairs → commas. Ports Swift `normalizeLegadoJSON`.
fn normalize_legado_json(input: &str) -> String {
    let mut result = String::with_capacity(input.len() + 16);
    let mut state = QuoteState::Normal;
    for ch in input.chars() {
        match state {
            QuoteState::Normal => match ch {
                '\'' => {
                    state = QuoteState::InSingle;
                    result.push('"');
                }
                '"' => {
                    state = QuoteState::InDouble;
                    result.push('"');
                }
                ';' => result.push(','),
                _ => result.push(ch),
            },
            QuoteState::InSingle => match ch {
                '\'' => {
                    state = QuoteState::Normal;
                    result.push('"');
                }
                '"' => {
                    result.push_str("\\\"");
                }
                '\\' => {
                    result.push_str("\\\\");
                }
                _ => result.push(ch),
            },
            QuoteState::InDouble => match ch {
                '"' => {
                    state = QuoteState::Normal;
                    result.push('"');
                }
                '\\' => {
                    state = QuoteState::EscapeDouble;
                    result.push('\\');
                }
                _ => result.push(ch),
            },
            QuoteState::EscapeDouble => {
                result.push(ch);
                state = QuoteState::InDouble;
            }
        }
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    Normal,
    InSingle,
    InDouble,
    EscapeDouble,
}

/// Detect `@js:` and `<js>...</js>` patterns in a URL.
fn classify_js_expression(url: &str) -> (Option<String>, JsExpressionClassification) {
    let lower = url.to_ascii_lowercase();
    if let Some(idx) = lower.find("@js:") {
        let expr = url[idx + 4..].trim();
        return (
            Some(expr.to_string()),
            JsExpressionClassification::RequiresJsSandbox,
        );
    }
    if let Some(start_rel) = lower.find("<js>") {
        let start = start_rel + 4;
        if let Some(end_rel) = lower.rfind("</js>") {
            if end_rel >= start {
                let expr = url[start..end_rel].trim();
                return (
                    Some(expr.to_string()),
                    JsExpressionClassification::RequiresJsSandbox,
                );
            }
        }
    }
    (None, JsExpressionClassification::None)
}

// HostHttpRequest assembly + AnalyzeUrl builder are added in later tasks.

// ============================================================================
// Task 3: AnalyzeUrl::build_request — HostHttpRequest assembly (no JS)
// ============================================================================

/// Errors raised by [`AnalyzeUrl::build_request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyzeUrlError {
    Dsl(UrlDslParseError),
    InvalidUrl(String),
    JsUnsupported(String),
    /// JS expression evaluation failed (e.g. sandbox runtime error).
    JsExecution(String),
}

impl std::fmt::Display for AnalyzeUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyzeUrlError::Dsl(e) => write!(f, "URL DSL parse error: {e}"),
            AnalyzeUrlError::InvalidUrl(msg) => write!(f, "invalid URL: {msg}"),
            AnalyzeUrlError::JsUnsupported(msg) => {
                write!(f, "URL JS execution unsupported in this build: {msg}")
            }
            AnalyzeUrlError::JsExecution(msg) => write!(f, "URL JS execution failed: {msg}"),
        }
    }
}

impl std::error::Error for AnalyzeUrlError {}

impl From<UrlDslParseError> for AnalyzeUrlError {
    fn from(e: UrlDslParseError) -> Self {
        AnalyzeUrlError::Dsl(e)
    }
}

/// Default User-Agent when neither source headers nor DSL headers supply one.
/// Mirrors Legado's default iPhone UA.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) \
    AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1";

/// AnalyzeUrl builder. Ports Swift `BookSourceRequestBuilder.makeSearchRequest`
/// (non-JS path) + `buildURLDSLRequest`.
pub struct AnalyzeUrl;

impl AnalyzeUrl {
    /// Build a [`HostHttpRequest`] from a raw URL DSL string + context.
    ///
    /// Steps (mirrors Swift `prepareSearchRequest` + `buildSearchRequest`):
    /// 1. Expand static templates (`{{key}}`/`{{page}}`/page list) on the raw
    ///    string.
    /// 2. Parse URL DSL (`url, {...}`).
    /// 3. If URL contains `@js:`/`<js>` or DSL option `js` is set, return
    ///    [`AnalyzeUrlError::JsUnsupported`] — use
    ///    [`AnalyzeUrl::build_request_with_js`] (Task 6) for JS URLs.
    /// 4. Assemble `HostHttpRequest`: resolve relative URL, merge headers,
    ///    set default User-Agent, auto Content-Type for POST+body, wire
    ///    charset/retry/redirect.
    pub fn build_request(
        raw_url: &str,
        ctx: &AnalyzeUrlContext,
        base_url: &str,
        source_headers: &Map<String, Value>,
    ) -> Result<HostHttpRequest, AnalyzeUrlError> {
        let expanded = expand_static_templates(raw_url, ctx);
        let dsl = UrlDslParser::parse(&expanded)?;
        if dsl.has_js_expression || dsl.options.js.is_some() {
            return Err(AnalyzeUrlError::JsUnsupported(
                "URL contains @js:/<js> or DSL option js; use build_request_with_js".into(),
            ));
        }
        Self::assemble(dsl, ctx, base_url, source_headers)
    }

    /// Build a [`HostHttpRequest`] from a raw URL DSL string that may contain
    /// `@js:` / `<js>...</js>` expressions or a DSL `js` option.
    ///
    /// Mirrors Legado `AnalyzeUrl` JS path: the embedded JS expression is
    /// evaluated with `key`/`page`/`baseUrl` variables in scope, and the
    /// returned string is parsed as a URL DSL. Non-JS URLs delegate to
    /// [`AnalyzeUrl::build_request`].
    ///
    /// The `js_eval` closure receives `(expression, context_json)` and returns
    /// the evaluated string result. Callers wire this to a JS sandbox (e.g.
    /// `RemoteContentPipeline::evaluate_url_js`).
    pub fn build_request_with_js<F>(
        raw_url: &str,
        ctx: &AnalyzeUrlContext,
        base_url: &str,
        source_headers: &Map<String, Value>,
        js_eval: F,
    ) -> Result<HostHttpRequest, AnalyzeUrlError>
    where
        F: FnOnce(&str, &Value) -> Result<String, String>,
    {
        let expanded = expand_static_templates(raw_url, ctx);
        let (js_expr, classification) = classify_js_expression(&expanded);
        let dsl = UrlDslParser::parse(&expanded)?;

        // Determine the JS expression source: `@js:`/`<js>` in URL, or DSL `js` option.
        let active_js = js_expr
            .clone()
            .or_else(|| dsl.options.js.clone());

        let has_js = classification == JsExpressionClassification::RequiresJsSandbox
            || dsl.options.js.is_some()
            || dsl.has_js_expression;

        if !has_js {
            // No JS — fall through to the non-JS path.
            return Self::assemble(dsl, ctx, base_url, source_headers);
        }

        let Some(expr) = active_js else {
            return Err(AnalyzeUrlError::JsUnsupported(
                "URL classified as JS but no expression extracted".into(),
            ));
        };

        // Build the JS context: variables exposed to the script. Mirrors Legado
        // AnalyzeUrl.kt's `evalJS` variable scope.
        let js_context = serde_json::json!({
            "key": ctx.raw_keyword,
            "page": ctx.page,
            "baseUrl": base_url,
        });

        let result = js_eval(&expr, &js_context)
            .map_err(AnalyzeUrlError::JsExecution)?;

        // The JS result is a URL string (or a Legado DSL form
        // `url,{"method":"POST",...}`). Re-parse it.
        let result_dsl = UrlDslParser::parse(&result)?;
        Self::assemble(result_dsl, ctx, base_url, source_headers)
    }

    fn assemble(
        dsl: UrlDslResult,
        ctx: &AnalyzeUrlContext,
        base_url: &str,
        source_headers: &Map<String, Value>,
    ) -> Result<HostHttpRequest, AnalyzeUrlError> {
        let final_url = resolve_relative_url(&dsl.url, base_url);
        validate_absolute_http_url(&final_url)?;

        // Body: expand static templates (harmless no-op if already expanded).
        let body = dsl
            .options
            .body
            .as_deref()
            .map(|b| expand_static_templates(b, ctx));

        // Merge source-level headers with DSL headers (DSL wins).
        let mut headers = source_headers.clone();
        for (key, value) in &dsl.options.headers {
            headers.insert(key.clone(), value.clone());
        }
        if !headers.contains_key("User-Agent") {
            headers.insert(
                "User-Agent".to_string(),
                Value::String(DEFAULT_USER_AGENT.to_string()),
            );
        }
        // Apply static templates to header string values (e.g. Cookie with
        // {{key}}).
        let mut headers: Map<String, Value> = headers
            .into_iter()
            .map(|(k, v)| {
                let expanded = match v {
                    Value::String(s) => Value::String(expand_static_templates(&s, ctx)),
                    other => other,
                };
                (k, expanded)
            })
            .collect();

        let method = dsl.options.method.to_ascii_uppercase();
        // Auto Content-Type for POST + body when not set (mirrors Swift
        // `applyAutomaticContentTypeIfNeeded`).
        if method == "POST" && body.is_some() && !headers.contains_key("Content-Type") {
            let charset = content_type_charset(&dsl.options.charset);
            headers.insert(
                "Content-Type".to_string(),
                Value::String(format!("application/x-www-form-urlencoded; charset={charset}")),
            );
        }

        let retry = dsl.options.retry.and_then(|n| {
            if n > 0 {
                Some(HostHttpRetryPolicy {
                    max_attempts: n,
                    backoff_millis: None,
                })
            } else {
                None
            }
        });

        Ok(HostHttpRequest {
            url: final_url,
            method,
            headers: Value::Object(headers),
            body,
            charset: Some(dsl.options.charset.clone()),
            follow_redirects: dsl.options.follow_redirects,
            max_redirects: None,
            retry,
            use_platform_cookie_jar: None,
            session: None,
        })
    }
}

/// Resolve a possibly-relative URL against a base URL. Ports Swift
/// `resolveRelativeURL`.
fn resolve_relative_url(url: &str, base_url: &str) -> String {
    let url = url.trim();
    // Any absolute URL with a scheme (e.g. "http://", "https://", "file://")
    // is returned as-is. Non-http schemes are later rejected by
    // `validate_absolute_http_url`.
    if has_url_scheme(url) {
        return url.to_string();
    }
    let base = base_url.trim();
    if base.is_empty() {
        return url.to_string();
    }
    if url.starts_with("//") {
        let scheme = base.split("://").next().unwrap_or("https");
        return format!("{scheme}:{url}");
    }
    if url.starts_with('/') {
        if let Some(origin) = url_origin(base) {
            return format!("{origin}{url}");
        }
        return url.to_string();
    }
    // Relative path: append to base directory.
    let directory = if base.ends_with('/') {
        base.to_string()
    } else {
        base.rsplit_once('/')
            .map(|(prefix, _)| format!("{prefix}/"))
            .unwrap_or_else(|| format!("{base}/"))
    };
    format!("{directory}{url}")
}

/// Detect whether `url` has an RFC 3986 scheme prefix (`scheme://`).
fn has_url_scheme(url: &str) -> bool {
    let Some(idx) = url.find("://") else {
        return false;
    };
    let scheme = &url[..idx];
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

fn url_origin(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    let host = rest.split('/').next()?;
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}://{host}"))
    }
}

fn validate_absolute_http_url(url: &str) -> Result<(), AnalyzeUrlError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| AnalyzeUrlError::InvalidUrl(format!("missing scheme: {url}")))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(AnalyzeUrlError::InvalidUrl(format!(
            "scheme must be http/https, got {scheme}"
        )));
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err(AnalyzeUrlError::InvalidUrl(format!("missing host: {url}")));
    }
    Ok(())
}

fn content_type_charset(charset: &str) -> &str {
    match charset.trim().to_ascii_lowercase().as_str() {
        "gbk" | "gb2312" | "gb18030" | "windows-936" | "cp936" => "gbk",
        "big5" | "big5-hkscs" | "windows-950" => "big5",
        "iso-8859-1" | "latin1" | "latin-1" | "iso8859-1" => "iso-8859-1",
        _ => "utf-8",
    }
}

// ============================================================================
// Task 2: Static template expander + page list
// ============================================================================

/// Context for AnalyzeUrl template expansion. Mirrors Swift
/// `SearchRequestContext`: raw keyword, percent-encoded keyword, page, and
/// page-derived values (`pageMinus`/`pagePlus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzeUrlContext {
    pub raw_keyword: String,
    pub encoded_keyword: String,
    pub page: u32,
    pub page_string: String,
    pub page_minus_string: String,
    pub page_plus_string: String,
}

impl AnalyzeUrlContext {
    /// Build a context for a search query. `keyword` is URL-percent-encoded
    /// for `encoded_keyword` (Legado `{{key}}` substitutes the encoded form,
    /// matching Swift `SearchRequestContext.encodedKeyword`).
    pub fn for_search(keyword: &str, page: u32) -> Self {
        Self {
            raw_keyword: keyword.to_string(),
            encoded_keyword: percent_encode_query_component(keyword),
            page,
            page_string: page.to_string(),
            page_minus_string: page.saturating_sub(1).max(1).to_string(),
            page_plus_string: (page + 1).to_string(),
        }
    }

    /// Build a context for a non-search URL (TOC/detail/chapter). `page`
    /// defaults to 1; `keyword` is empty.
    pub fn for_url() -> Self {
        Self::for_search("", 1)
    }
}

impl Default for AnalyzeUrlContext {
    fn default() -> Self {
        Self::for_url()
    }
}

fn percent_encode_query_component(value: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// Expand Legado static templates `{{key}}`/`{{keyword}}`/`{{page}}`/
/// `{{pageMinus}}`/`{{pagePlus}}` and Legado page-list `<1,3,5>`/`<1-3>`
/// (takes the first value for a single-request build — Legado `pagePattern`).
///
/// Mirrors Swift `replacingStaticTemplates(in:context:)` +
/// `PageListExpander.expandURLs` first-value behavior.
pub fn expand_static_templates(raw: &str, ctx: &AnalyzeUrlContext) -> String {
    let with_page_list = expand_page_list(raw);
    let mut out = with_page_list;
    out = out.replace("{{key}}", &ctx.encoded_keyword);
    out = out.replace("{{keyword}}", &ctx.raw_keyword);
    out = out.replace("{{page}}", &ctx.page_string);
    out = out.replace("{{pageMinus}}", &ctx.page_minus_string);
    out = out.replace("{{pagePlus}}", &ctx.page_plus_string);
    out
}

/// Expand Legado `<a,b,c>` / `<a-b>` page-list pattern. For a single-request
/// build (AnalyzeUrl constructs one descriptor), use the first value. Ports
/// Swift `PageListExpander.expandURLs` first-value behavior.
///
/// Only the first numeric `<...>` pattern is expanded. Non-numeric angle-bracket
/// content (e.g. `<js>...</js>`) is left intact. Multiple page-list patterns in
/// one URL are a deferred V2 concern.
fn expand_page_list(input: &str) -> String {
    // Scan for `<` followed by numeric content (digits, comma, dash, space).
    // This avoids false positives on `<js>`/`<html>`/etc.
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Find the matching `>`.
            if let Some(end_rel) = input[i + 1..].find('>') {
                let end = i + 1 + end_rel;
                let body = &input[i + 1..end];
                let first = body.split(',').next().unwrap_or(body).trim();
                // Only treat as page-list if the first item parses as integer
                // or a `lo-hi` range. Otherwise skip this `<...>` (e.g. `<js>`).
                let is_numeric = if let Some((lo, _hi)) = first.split_once('-') {
                    !lo.trim().is_empty() && lo.trim().parse::<i64>().is_ok()
                } else {
                    first.parse::<i64>().is_ok()
                };
                if is_numeric {
                    let first_value: i64 = if let Some((lo, hi)) = first.split_once('-') {
                        let lo: i64 = lo.trim().parse().unwrap_or(1);
                        let _hi: i64 = hi.trim().parse().unwrap_or(lo);
                        lo
                    } else {
                        first.parse::<i64>().unwrap_or(1)
                    };
                    let mut out = String::with_capacity(input.len());
                    out.push_str(&input[..i]);
                    out.push_str(&first_value.to_string());
                    out.push_str(&input[end + 1..]);
                    return out;
                }
                // Not numeric — skip past this `>` and keep scanning.
                i = end + 1;
                continue;
            } else {
                // No matching `>` — stop.
                break;
            }
        }
        i += 1;
    }
    input.to_string()
}

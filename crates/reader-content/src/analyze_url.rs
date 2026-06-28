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
    /// Legado `headers` accepts either a JSON object (`{"k":"v"}`) or, in some
    /// sources, a string containing a single-quoted JSON object
    /// (`"{os:'pc'}"`). The custom deserializer normalizes the string form
    /// back into a map so downstream code always sees `Map<String, Value>`.
    #[serde(default, deserialize_with = "deserialize_headers")]
    pub headers: Map<String, Value>,
    /// Legado `body` accepts either a string (`"k=v"`) or a JSON object
    /// (`{"k":"v"}`). When a JSON object is supplied, it is serialized back
    /// to a JSON string so [`HostHttpRequest::body`] stays `Option<String>`.
    /// Mirrors Legado `AnalyzeUrl` body handling for sources like 番薯小说.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_body"
    )]
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

/// Deserialize the Legado `body` field, which accepts either a string
/// (`"k=v"`) or a JSON object (`{"k":"v"}`). A JSON object is serialized back
/// to a JSON string so the field stays `Option<String>`. Other JSON types
/// (numbers, arrays, bools) are rejected.
fn deserialize_body<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(Value::Object(_)) => {
            let json = serde_json::to_string(&value).map_err(|e| {
                Error::custom(format!("failed to serialize body object: {e}"))
            })?;
            Ok(Some(json))
        }
        Some(other) => Err(Error::custom(format!(
            "body must be a string or JSON object, got {}",
            json_value_type_name(&other)
        ))),
    }
}

fn json_value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Deserialize the Legado `headers` field. Accepts either a JSON object
/// (`{"k":"v"}`) or a string containing a (possibly single-quoted / unquoted-
/// key) JSON object (`"{os:'pc'}"`). The string form is normalized via
/// [`normalize_legado_json`] and re-parsed into a map. Non-object JSON types
/// (numbers, arrays, bools) are rejected; an empty or unparseable string
/// yields an empty map rather than failing the whole DSL parse.
fn deserialize_headers<'de, D>(deserializer: D) -> Result<Map<String, Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Object(map) => Ok(map),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(Map::new());
            }
            // Normalize single quotes / unquoted keys, then parse.
            let normalized = normalize_legado_json(trimmed);
            match serde_json::from_str::<Map<String, Value>>(&normalized) {
                Ok(map) => Ok(map),
                Err(_) => {
                    // If the string isn't a parseable JSON object, treat it as
                    // a single header with an empty key (mirrors Legado's
                    // lenient fallback for malformed header strings).
                    Ok(Map::new())
                }
            }
        }
        Value::Null => Ok(Map::new()),
        other => Err(Error::custom(format!(
            "headers must be an object or string, got {}",
            json_value_type_name(&other)
        ))),
    }
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
    /// - `url,...@js:expr` / `url,...<js>expr</js>` → the `@js:`/`<js>` tail
    ///   is stripped BEFORE DSL parsing so it doesn't break JSON option
    ///   parsing (e.g. 图书迷子-style sources). The stripped expression is
    ///   attached to [`UrlDslResult::js_expression`].
    pub fn parse(raw: &str) -> Result<UrlDslResult, UrlDslParseError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(UrlDslResult::new(String::new(), UrlDslOptions::default()));
        }

        // Strip @js:/<js> tail BEFORE DSL parsing so a trailing JS expression
        // doesn't break JSON option parsing (e.g.
        // `url,{"method":"POST"}@js:java.webView(...)`). The stripped
        // expression is attached to the result afterwards.
        let (dsl_part, stripped_js) = split_js_suffix(trimmed);

        if dsl_part.is_empty() {
            // Whole input was a JS expression (e.g. `@js:...` or `<js>...</js>`).
            return Ok(attach_stripped_js(
                UrlDslResult::new(String::new(), UrlDslOptions::default()),
                stripped_js,
            ));
        }

        if has_legacy_method_prefix(dsl_part) {
            return Ok(attach_stripped_js(
                UrlDslResult::new(dsl_part.to_string(), UrlDslOptions::default()),
                stripped_js,
            ));
        }

        let Some(comma_index) = find_dsl_separator(dsl_part) else {
            return Ok(attach_stripped_js(
                UrlDslResult::new(dsl_part.to_string(), UrlDslOptions::default()),
                stripped_js,
            ));
        };

        let url_part = dsl_part[..comma_index].trim();
        let options_part = dsl_part[comma_index + 1..].trim();

        if options_part.is_empty() {
            return Ok(attach_stripped_js(
                UrlDslResult::new(url_part.to_string(), UrlDslOptions::default()),
                stripped_js,
            ));
        }

        // find_dsl_separator already guarantees options_part starts with '{',
        // but double-check defensively.
        if !options_part.starts_with('{') {
            return Ok(attach_stripped_js(
                UrlDslResult::new(dsl_part.to_string(), UrlDslOptions::default()),
                stripped_js,
            ));
        }

        let normalized = normalize_legado_json(options_part);
        let options: UrlDslOptions = serde_json::from_str(&normalized)
            .map_err(|err| UrlDslParseError::MalformedJson(format!("{err}: {options_part}")))?;
        Ok(attach_stripped_js(
            UrlDslResult::new(url_part.to_string(), options),
            stripped_js,
        ))
    }

    /// Classify a URL for embedded JS expressions. Public so callers can
    /// re-classify after mutating the URL.
    pub fn classify_js_expression(url: &str) -> (Option<String>, JsExpressionClassification) {
        classify_js_expression(url)
    }
}

/// Strip a trailing `@js:` / `<js>...</js>` suffix from a URL DSL string,
/// returning `(dsl_part, js_expression)`. The split point is the first
/// `@js:` or `<js>` that occurs OUTSIDE quoted strings and OUTSIDE JSON
/// object braces, so `@js:` inside a body value (e.g.
/// `{"body":"q=test@js:foo"}`) is NOT treated as a separator.
///
/// For `@js:`, the expression is everything after `@js:` (trimmed). For
/// `<js>...</js>`, the expression is the content between the tags (trimmed);
/// if no closing `</js>` is found, the rest of the input is treated as the
/// expression.
fn split_js_suffix(input: &str) -> (&str, Option<String>) {
    let lower = input.to_ascii_lowercase();
    let mut in_double = false;
    let mut in_single = false;
    let mut brace_depth: i32 = 0;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '{' if !in_double && !in_single => brace_depth += 1,
            '}' if !in_double && !in_single => brace_depth -= 1,
            _ => {
                if !in_double && !in_single && brace_depth == 0 {
                    if lower[i..].starts_with("@js:") {
                        let dsl_part = &input[..i];
                        let expr = input[i + 4..].trim();
                        return (dsl_part, Some(expr.to_string()));
                    }
                    if lower[i..].starts_with("<js>") {
                        let dsl_part = &input[..i];
                        let start = i + 4;
                        if let Some(end_rel) = lower[start..].find("</js>") {
                            let end = start + end_rel;
                            let expr = input[start..end].trim();
                            return (dsl_part, Some(expr.to_string()));
                        } else {
                            // No closing </js> — treat the rest as the JS expression.
                            let expr = input[start..].trim();
                            return (dsl_part, Some(expr.to_string()));
                        }
                    }
                }
            }
        }
    }
    (input, None)
}

/// Attach a JS expression (stripped from the input tail) to a [`UrlDslResult`].
/// When `js` is `Some`, overrides the classification to `RequiresJsSandbox`.
/// When `js` is `None`, the result is returned unchanged.
fn attach_stripped_js(mut result: UrlDslResult, js: Option<String>) -> UrlDslResult {
    if let Some(expr) = js {
        result.js_expression = Some(expr);
        result.has_js_expression = true;
        result.js_classification = JsExpressionClassification::RequiresJsSandbox;
    }
    result
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
/// AND followed by optional whitespace + `{`. `{{...}}` Legado JS templates
/// in the URL are treated as opaque so a comma inside or right before them
/// is not mistaken for the DSL separator.
fn find_dsl_separator(input: &str) -> Option<usize> {
    let mut in_double = false;
    let mut in_single = false;
    let mut bracket_depth: i32 = 0;
    let mut in_js_template = false;
    for (idx, ch) in input.char_indices() {
        // Inside a {{...}} JS template: skip everything until the closing }}.
        if in_js_template {
            if input[idx..].starts_with("}}") {
                in_js_template = false;
            }
            continue;
        }
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '[' if !in_double && !in_single => bracket_depth += 1,
            ']' if !in_double && !in_single => bracket_depth -= 1,
            '{' if !in_double && !in_single && input[idx..].starts_with("{{") => {
                in_js_template = true;
                continue;
            }
            ',' if !in_double && !in_single && bracket_depth == 0 => {
                let rest = &input[idx + 1..];
                let trimmed = rest.trim_start();
                // Must be followed by `{` but NOT `{{` (which is a JS template).
                if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

/// Normalize Legado JSON quirks. Ports Swift `normalizeLegadoJSON` and
/// extends it to handle real-world Legado sources:
/// - Single quotes → double quotes (`{'k':'v'}` → `{"k":"v"}`)
/// - Semicolons between pairs → commas (`{"k":"v";"k2":"v2"}`)
/// - Unquoted keys → quoted keys (`{method: "POST"}` → `{"method": "POST"}`)
/// - Bare string values → quoted strings (`{"k":%E6}` → `{"k":"%E6"}`)
/// - Trailing characters after the matching `}` are truncated (e.g. trailing
///   JS expressions accidentally included in the DSL options).
fn normalize_legado_json(input: &str) -> String {
    let truncated = truncate_at_matching_brace(input);
    let chars: Vec<char> = truncated.chars().collect();
    let mut result = String::with_capacity(truncated.len() + 16);
    let mut state = QuoteState::Normal;
    // `ExpectKey` = after `{` or `,` at object depth; `ExpectValue` = after `:`.
    let mut position = ValuePosition::Other;
    let mut i = 0usize;

    while i < chars.len() {
        match state {
            QuoteState::Normal => {
                let ch = chars[i];
                match ch {
                    '\'' => {
                        state = QuoteState::InSingle;
                        result.push('"');
                    }
                    '"' => {
                        state = QuoteState::InDouble;
                        result.push('"');
                    }
                    ';' => {
                        // Semicolon between pairs → comma (Legado quirk).
                        result.push(',');
                        position = ValuePosition::ExpectKey;
                    }
                    '{' => {
                        result.push(ch);
                        position = ValuePosition::ExpectKey;
                    }
                    '[' => {
                        result.push(ch);
                        position = ValuePosition::Other;
                    }
                    '}' | ']' => {
                        result.push(ch);
                        position = ValuePosition::Other;
                    }
                    ',' => {
                        result.push(ch);
                        position = ValuePosition::ExpectKey;
                    }
                    ':' => {
                        result.push(ch);
                        position = ValuePosition::ExpectValue;
                    }
                    c if c.is_whitespace() => {
                        result.push(ch);
                    }
                    _ => {
                        match position {
                            ValuePosition::ExpectKey => {
                                // Unquoted key: read an identifier (letter/_ start)
                                // and wrap it in double quotes.
                                if is_ident_start(ch) {
                                    let start = i;
                                    while i < chars.len() {
                                        let c = chars[i];
                                        if is_ident_continue(c) {
                                            i += 1;
                                        } else {
                                            break;
                                        }
                                    }
                                    let key: String = chars[start..i].iter().collect();
                                    result.push('"');
                                    result.push_str(&key);
                                    result.push('"');
                                    position = ValuePosition::Other;
                                    continue; // `i` already at next char
                                }
                                result.push(ch);
                            }
                            ValuePosition::ExpectValue => {
                                // Bare value: if it's not a JSON literal start
                                // (digit, `t`/`f`/`n` for true/false/null, `"`,
                                // `'`, `{`, `[`), treat it as an unquoted string
                                // and read until the next `,` or `}`.
                                if is_bare_value_start(ch) {
                                    result.push(ch);
                                } else {
                                    let start = i;
                                    while i < chars.len() {
                                        let c = chars[i];
                                        if c == ',' || c == '}' || c == ']' {
                                            break;
                                        }
                                        i += 1;
                                    }
                                    // Trim trailing whitespace from the bare value.
                                    let mut end = i;
                                    while end > start && chars[end - 1].is_whitespace() {
                                        end -= 1;
                                    }
                                    let value: String = chars[start..end].iter().collect();
                                    result.push('"');
                                    // Escape any embedded double quotes.
                                    for vc in value.chars() {
                                        if vc == '"' {
                                            result.push('\\');
                                        }
                                        result.push(vc);
                                    }
                                    result.push('"');
                                    position = ValuePosition::Other;
                                    continue; // `i` already at delimiter
                                }
                                position = ValuePosition::Other;
                            }
                            ValuePosition::Other => {
                                result.push(ch);
                            }
                        }
                    }
                }
                // NOTE: do NOT `i += 1` here — the shared `i += 1` at the
                // end of the while loop advances for all non-`continue` arms.
            }
            QuoteState::InSingle => match chars[i] {
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
                _ => result.push(chars[i]),
            },
            QuoteState::InDouble => match chars[i] {
                '"' => {
                    state = QuoteState::Normal;
                    result.push('"');
                }
                '\\' => {
                    state = QuoteState::EscapeDouble;
                    result.push('\\');
                }
                _ => result.push(chars[i]),
            },
            QuoteState::EscapeDouble => {
                result.push(chars[i]);
                state = QuoteState::InDouble;
            }
        }
        i += 1;
    }
    result
}

/// Truncate `input` at the matching `}` for the first `{`, accounting for
/// nested braces, brackets, and quoted strings. This strips trailing junk
/// like JS expressions accidentally appended after the JSON object.
fn truncate_at_matching_brace(input: &str) -> String {
    let mut depth: i32 = 0;
    let mut in_double = false;
    let mut in_single = false;
    let mut end = input.len();
    for (i, ch) in input.char_indices() {
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '{' if !in_double && !in_single => depth += 1,
            '}' if !in_double && !in_single => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        end = i + ch.len_utf8();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    input[..end].to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    Normal,
    InSingle,
    InDouble,
    EscapeDouble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValuePosition {
    /// Default state — not expecting a key or value.
    Other,
    /// After `{` or `,` — expecting a key (or closing `}`).
    ExpectKey,
    /// After `:` — expecting a value.
    ExpectValue,
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == '$'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || ch == '-'
}

/// Whether `ch` starts a recognized JSON literal value that should NOT be
/// treated as a bare string (digits, `true`, `false`, `null`, or a quoted/
/// nested value start). Anything else is treated as a bare string.
fn is_bare_value_start(ch: char) -> bool {
    ch.is_ascii_digit()
        || ch == '-'
        || ch == '+'
        || ch == '"'
        || ch == '\''
        || ch == '{'
        || ch == '['
        || ch == 't'
        || ch == 'f'
        || ch == 'n'
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
        let active_js = js_expr.clone().or_else(|| dsl.options.js.clone());

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

        let result = js_eval(&expr, &js_context).map_err(AnalyzeUrlError::JsExecution)?;

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
                Value::String(format!(
                    "application/x-www-form-urlencoded; charset={charset}"
                )),
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
    // Per RFC 3986, host terminates at first `/`, `?`, or `#`. Legado
    // `bookSourceUrl` commonly appends `#<remark>` (source-key
    // disambiguator, e.g. `http://example.com#yc`); without stripping it,
    // absolute-path URL resolution produces `http://example.com#yc/path`
    // where the fragment swallows the path — the server serves the homepage.
    let host = rest.split(['/', '?', '#']).next()?;
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

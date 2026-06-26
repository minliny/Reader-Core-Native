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

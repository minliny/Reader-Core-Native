# AnalyzeUrl Request Builder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the S3/S4 "AnalyzeUrl 请求构建（零实现）" gap: give Rust Core the ability to construct a `HostHttpRequest` descriptor from a Legado book source's `searchUrl` / `tocUrl` / `chapterUrl` template string, so `book.search`/`book.detail`/`book.toc`/`chapter.content` no longer require the caller to pre-build the request.

**Architecture:** Port Swift `URLDSLParser.swift` + `BookSourceRequestBuilder.swift` into a new `reader-content::analyze_url` module. The module owns (1) URL DSL parsing (`url, {"method":"POST",...}`), (2) static template expansion (`{{key}}`/`{{page}}`/`pageMinus`/`pagePlus` + Legado `<1,3,5>`/`<1-3>` page list), (3) URL-embedded JS evaluation via the existing `reader-js` sandbox, and (4) `HostHttpRequest` assembly. `reader-runtime::remote` is then wired so that when `*_request` is `None` and `*_response` is empty, Core auto-builds the descriptor from the source's `searchUrl` template + the new `keyword`/`page` (or `bookUrl`/`chapterUrl`) fields on the params. Core still produces only descriptors — Host executes HTTP. No sockets in Core.

**Tech Stack:** Rust 1.75 / edition 2021; `reader-content` (existing `BookSourceRequestContext`, `LegadoJsBridge`); `reader-js` `JsSandbox` trait + `HostDescriptor`; `reader-contract::remote::HostHttpRequest`; `reader-domain::BookSourceSemantics`; `serde_json` for DSL option parsing; `percent-encoding` for URL component encoding.

**Migration sources (read-only references):**
- `Documents/Reader-Core/Sources/ReaderCoreNetwork/URLDSLParser.swift` (493 lines) — URL DSL grammar + Legado JSON quirks
- `Documents/Reader-Core/Sources/ReaderCoreNetwork/BookSourceRequestBuilder.swift` (1880 lines) — `makeSearchRequest` / `makeTOCRequest` / `makeContentRequest` + `makeSearchRequestExecutingURLJS`
- `Documents/legado/app/src/main/java/io/legado/app/model/analyzeRule/AnalyzeUrl.kt` (851 lines) — field-coverage baseline

**Red lines (per `docs/PROJECT_CHARTER.md`):**
- Core produces `HostHttpRequest` descriptor; Host executes; Core opens no socket.
- Migration fidelity: port Swift logic, do not reinvent.
- Capability baseline = Legado: must construct descriptors for real Legado `searchUrl` templates (e.g. `search?q={{key}},{"method":"POST","body":"k={{key}}","charset":"gbk"}`), not self-invented fixtures.
- Evidence: `reader-content` / `reader-runtime` / `reader-contract` / `reader-cli` unit + integration tests + CLI host-replay fixture suite.

**Legado `AnalyzeUrl.kt` field-coverage target** (verified from baseline read):
| Legado field | Coverage in this plan | Notes |
| --- | --- | --- |
| `method` | ✅ `HostHttpRequest.method` | GET/POST/PUT/DELETE/PATCH |
| `charset` | ✅ `HostHttpRequest.charset` | utf-8/gbk/big5/iso-8859-1 |
| `headers` / `header` | ✅ `HostHttpRequest.headers` | Both JSON object and newline-separated string forms |
| `body` | ✅ `HostHttpRequest.body` | String or form-encoded object |
| `retry` | ✅ `HostHttpRequest.retry` | `HostHttpRetryPolicy` |
| `type` | ⚠️ Deferred | Affects response decoding (`0` text / `1` image / `2` audio); Host decides transport. Tracked in audit. |
| `js` (post-parse JS) | ✅ Task 6 | Runs after URL DSL parse; result replaces `url` |
| `origin` | ⚠️ Source-book URL only | Used for relative-URL resolution (= `bookSourceUrl`) |
| `webView` / `webJs` / `webViewDelayTime` | ⚠️ Host responsibility | Core surfaces the flag via `HostHttpRequest.session.metadata` (Task 7) |
| `serverID` | ⚠️ Deferred | Multi-server routing; not in `HostHttpRequest` schema. Tracked in audit. |
| `{{key}}` / `{{page}}` / `pageMinus` / `pagePlus` | ✅ Task 2 | Static template expansion |
| `<1,3,5>` / `<1-3>` page list | ✅ Task 2 | Legado `pagePattern` |
| `@js:` / `<js>...</js>` URL JS | ✅ Task 6 | Via `reader-js` sandbox |
| `{{...}}` inline JS template | ✅ Task 6 | Non-static expressions route through sandbox |
| Cookie (`enabledCookieJar`) | ✅ `HostHttpRequest.use_platform_cookie_jar` | From `BookSource.enabledCookieJar` |
| Redirect (`followRedirects`) | ✅ `HostHttpRequest.follow_redirects` / `max_redirects` | DSL option `followRedirects` |

---

## File Structure

**Create:**
- `crates/reader-content/src/analyze_url.rs` — New module: `UrlDslOptions`, `UrlDslResult`, `UrlDslParser`, `AnalyzeUrlContext`, `AnalyzeUrl` builder, JS classification + evaluation.
- `crates/reader-content/tests/analyze_url.rs` — Integration tests for the builder (one test per fixture shape).
- `tests/fixtures/host_replay/analyze_url_build_suite.json` — Real Legado book source shapes (desensitized) exercising the auto-build path end-to-end through the runtime.

**Modify:**
- `crates/reader-content/src/lib.rs` — Add `pub mod analyze_url;` declaration (single line near `pub mod normalization;`).
- `crates/reader-contract/src/remote.rs` — Add `keyword: Option<String>` and `page: Option<u32>` to `BookSearchParams`; add `chapter_url: Option<String>` to `ChapterContentParams` (the existing `book_id`/`book` fields cover detail; `book_id` covers toc).
- `crates/reader-runtime/src/remote.rs` — In `book_search`/`book_detail`/`book_toc`/`chapter_content`: when `*_request` is `None` and `*_response` is empty, call `AnalyzeUrl::build_*_request` to construct the descriptor before falling through to `pending_or_missing_response`.
- `tools/reader-cli/src/conformance.rs` — Add one conformance case that exercises the auto-build path (input: source with `searchUrl` template + `keyword`, no `searchRequest`; expected: Core emits `host.request` with the expanded URL).

---

## Task 1: URL DSL Parser (port from Swift `URLDSLParser.swift`)

**Files:**
- Create: `crates/reader-content/src/analyze_url.rs`
- Create: `crates/reader-content/tests/analyze_url.rs`

- [ ] **Step 1.1: Write the failing test — plain URL**

Create `crates/reader-content/tests/analyze_url.rs`:

```rust
use reader_content::analyze_url::{UrlDslParser, UrlDslResult};

#[test]
fn parse_plain_url_yields_no_options() {
    let result = UrlDslParser::parse("https://example.test/search").expect("plain URL parses");
    assert_eq!(result.url, "https://example.test/search");
    assert_eq!(result.options.method, "GET");
    assert!(result.options.body.is_none());
    assert!(result.options.js.is_none());
    assert!(!result.has_js_expression);
}

#[test]
fn parse_empty_string_yields_empty_url() {
    let result = UrlDslParser::parse("").expect("empty string parses");
    assert_eq!(result.url, "");
}
```

- [ ] **Step 1.2: Run test to verify it fails**

Run: `cargo test -p reader-content --test analyze_url`
Expected: FAIL with "module `analyze_url` not found" or similar.

- [ ] **Step 1.3: Declare module + implement minimal parser**

Add to `crates/reader-content/src/lib.rs` (immediately after `pub mod normalization;`):

```rust
pub mod analyze_url;
```

Create `crates/reader-content/src/analyze_url.rs`:

```rust
//! Legado AnalyzeUrl-equivalent request descriptor builder.
//!
//! Ports Swift `URLDSLParser.swift` + `BookSourceRequestBuilder.swift` into
//! Rust. The module owns:
//! - URL DSL parsing (`url, {"method":"POST",...}`) — see [`UrlDslParser`]
//! - Static template expansion (`{{key}}`/`{{page}}`/`pageMinus`/`pagePlus`)
//! - URL-embedded JS classification + evaluation (via `reader-js`)
//! - [`HostHttpRequest`](reader_contract::remote::HostHttpRequest) assembly
//!
//! Core produces descriptors; Host executes HTTP. Core opens no socket.

use reader_contract::remote::HostHttpRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Parsed JSON options from a Legado URL DSL string.
///
/// Format: `url, {"method":"POST","body":"...","headers":{...},"charset":"gbk"}`.
/// Mirrors Swift `URLDSLOptions` and Legado `AnalyzeUrl.UrlOption`.
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
    #[serde(default)]
    pub retry: u32,
    #[serde(default = "default_type")]
    pub r#type: String,
    #[serde(default)]
    pub web_view: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_js: Option<String>,
    #[serde(default)]
    pub web_view_delay_time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_redirects: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js: Option<String>,
}

fn default_method() -> String {
    "GET".to_string()
}
fn default_charset() -> String {
    "utf-8".to_string()
}
fn default_type() -> String {
    "text".to_string()
}

impl Default for UrlDslOptions {
    fn default() -> Self {
        Self {
            method: default_method(),
            charset: default_charset(),
            headers: Map::new(),
            body: None,
            retry: 0,
            r#type: default_type(),
            web_view: false,
            web_js: None,
            web_view_delay_time: 0,
            origin: None,
            server_id: None,
            follow_redirects: None,
            timeout: None,
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
    pub fn parse(raw: &str) -> Result<UrlDslResult, UrlDslParseError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(UrlDslResult::new(String::new(), UrlDslOptions::default()));
        }

        if has_legacy_method_prefix(trimmed) {
            return Ok(UrlDslResult::new(trimmed.to_string(), UrlDslOptions::default()));
        }

        let Some(comma_index) = find_dsl_separator(trimmed) else {
            return Ok(UrlDslResult::new(trimmed.to_string(), UrlDslOptions::default()));
        };

        let url_part = trimmed[..comma_index].trim();
        let options_part = trimmed[comma_index + 1..].trim();

        if options_part.is_empty() {
            return Ok(UrlDslResult::new(url_part.to_string(), UrlDslOptions::default()));
        }

        if !options_part.starts_with('{') {
            return Ok(UrlDslResult::new(trimmed.to_string(), UrlDslOptions::default()));
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

fn has_legacy_method_prefix(raw: &str) -> bool {
    let Some(comma_index) = raw.find(',') else {
        return false;
    };
    let method = raw[..comma_index].trim().to_ascii_uppercase();
    matches!(method.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH")
}

/// Find the comma that separates URL from JSON options. Skips commas inside
/// `[]` or `"` (mirrors Swift `findDSLSeparator`).
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
            ',' if !in_double && !in_single && bracket_depth == 0 => return Some(idx),
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
    if lower.contains("<js>") {
        let start = lower.find("<js>").map(|i| i + 4)?;
        let end = lower.rfind("</js>")?;
        if end >= start {
            let expr = url[start..end].trim();
            return (
                Some(expr.to_string()),
                JsExpressionClassification::RequiresJsSandbox,
            );
        }
    }
    (None, JsExpressionClassification::None)
}

#[cfg(test)]
impl UrlDslOptions {
    pub fn method_upper(&self) -> &str {
        // Tests assert against uppercase method names.
        &self.method
    }
}
```

- [ ] **Step 1.4: Run test to verify it passes**

Run: `cargo test -p reader-content --test analyze_url parse_plain_url_yields_no_options parse_empty_string_yields_empty_url`
Expected: PASS for both tests.

- [ ] **Step 1.5: Add URL + JSON options test**

Append to `crates/reader-content/tests/analyze_url.rs`:

```rust
#[test]
fn parse_url_with_json_options_post_body() {
    let raw = r#"https://example.test/search, {"method":"POST","body":"k={{key}}","charset":"gbk"}"#;
    let result = UrlDslParser::parse(raw).expect("URL+JSON parses");
    assert_eq!(result.url, "https://example.test/search");
    assert_eq!(result.options.method, "POST");
    assert_eq!(result.options.body.as_deref(), Some("k={{key}}"));
    assert_eq!(result.options.charset, "gbk");
}

#[test]
fn parse_url_with_single_quoted_json_normalizes() {
    let raw = r#"https://example.test/search, {'method':'POST','body':'k=test'}"#;
    let result = UrlDslParser::parse(raw).expect("single-quoted JSON parses");
    assert_eq!(result.options.method, "POST");
    assert_eq!(result.options.body.as_deref(), Some("k=test"));
}

#[test]
fn parse_url_with_semicolon_separated_pairs_normalizes() {
    let raw = r#"https://example.test/search, {"method":"POST";"body":"k=test"}"#;
    let result = UrlDslParser::parse(raw).expect("semicolon-separated JSON parses");
    assert_eq!(result.options.method, "POST");
    assert_eq!(result.options.body.as_deref(), Some("k=test"));
}

#[test]
fn parse_url_with_legacy_method_prefix_keeps_url() {
    let raw = "POST,https://example.test/search";
    let result = UrlDslParser::parse(raw).expect("legacy method prefix parses");
    assert_eq!(result.url, raw);
    assert_eq!(result.options.method, "GET"); // DSL options unchanged
}

#[test]
fn parse_url_with_at_js_expression_classifies() {
    let raw = "https://example.test/search@js:result.replace(' ', '+')";
    let result = UrlDslParser::parse(raw).expect("@js: URL parses");
    assert!(result.has_js_expression);
    assert_eq!(
        result.js_classification,
        reader_content::analyze_url::JsExpressionClassification::RequiresJsSandbox
    );
    assert_eq!(
        result.js_expression.as_deref(),
        Some("result.replace(' ', '+')")
    );
}

#[test]
fn parse_malformed_json_returns_error() {
    let raw = "https://example.test/search, {not valid json";
    let result = UrlDslParser::parse(raw);
    assert!(result.is_err());
}
```

- [ ] **Step 1.6: Run new tests**

Run: `cargo test -p reader-content --test analyze_url`
Expected: All 7 tests PASS.

- [ ] **Step 1.7: Commit**

```bash
git add crates/reader-content/src/analyze_url.rs crates/reader-content/src/lib.rs crates/reader-content/tests/analyze_url.rs
git commit -m "feat(reader-content): port URL DSL parser from Swift URLDSLParser

Ports URLDSLOptions / URLDSLResult / URLDSLParser from Swift
URLDSLParser.swift. Handles Legado JSON quirks (single quotes,
semicolons), legacy method prefixes, @js:/<js> classification.
First piece of the AnalyzeUrl S3/S4 closure."
```

---

## Task 2: Static Template Expander + Page List

**Files:**
- Modify: `crates/reader-content/src/analyze_url.rs`
- Modify: `crates/reader-content/tests/analyze_url.rs`

- [ ] **Step 2.1: Write the failing test — static template expansion**

Append to `crates/reader-content/tests/analyze_url.rs`:

```rust
use reader_content::analyze_url::{AnalyzeUrlContext, expand_static_templates};

#[test]
fn expand_static_templates_replaces_key_and_page() {
    let ctx = AnalyzeUrlContext::for_search("mirror", 2);
    let out = expand_static_templates(
        "https://example.test/search?q={{key}}&p={{page}}&pm={{pageMinus}}&pp={{pagePlus}}",
        &ctx,
    );
    assert_eq!(
        out,
        "https://example.test/search?q=mirror&p=2&pm=1&pp=3"
    );
}

#[test]
fn expand_static_templates_replaces_keyword_alias() {
    let ctx = AnalyzeUrlContext::for_search("中文测试", 1);
    let out = expand_static_templates("q={{keyword}}", &ctx);
    assert_eq!(out, "q=中文测试");
}

#[test]
fn expand_page_list_takes_first_value_for_single_request() {
    let ctx = AnalyzeUrlContext::for_search("k", 1);
    let out = expand_static_templates("https://example.test/list?p=<1,3,5>", &ctx);
    assert_eq!(out, "https://example.test/list?p=1");
}

#[test]
fn expand_page_list_range_takes_first_value() {
    let ctx = AnalyzeUrlContext::for_search("k", 1);
    let out = expand_static_templates("https://example.test/list?p=<1-3>", &ctx);
    assert_eq!(out, "https://example.test/list?p=1");
}
```

- [ ] **Step 2.2: Run test to verify it fails**

Run: `cargo test -p reader-content --test analyze_url expand_static_templates`
Expected: FAIL — `AnalyzeUrlContext` and `expand_static_templates` not found.

- [ ] **Step 2.3: Implement `AnalyzeUrlContext` + `expand_static_templates` + page list**

Append to `crates/reader-content/src/analyze_url.rs`:

```rust
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
    /// for `encoded_keyword` (Legado `{{key}}` substitutes the encoded form).
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

fn percent_encode_query_component(value: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    // RFC 3986 sub-delims + query-reserved chars that need encoding.
    const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'!')
        .add(b'"')
        .add(b'#')
        .add(b'$')
        .add(b'%')
        .add(b'&')
        .add(b'\'')
        .add(b'(')
        .add(b')')
        .add(b'+')
        .add(b',')
        .add(b'/')
        .add(b':')
        .add(b';')
        .add(b'<')
        .add(b'=')
        .add(b'>')
        .add(b'?')
        .add(b'@')
        .add(b'[')
        .add(b'\\')
        .add(b']')
        .add(b'^')
        .add(b'`')
        .add(b'{')
        .add(b'|')
        .add(b'}');
    utf8_percent_encode(value, QUERY_ENCODE_SET).to_string()
}

/// Expand Legado static templates `{{key}}`/`{{keyword}}`/`{{page}}`/
/// `{{pageMinus}}`/`{{pagePlus}}` and Legado page-list `<1,3,5>`/`<1-3>`
/// (takes the first value for a single-request build — Legado `pagePattern`).
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
fn expand_page_list(input: &str) -> String {
    let Some(start) = input.find('<') else {
        return input.to_string();
    };
    let Some(end_rel) = input[start + 1..].find('>') else {
        return input.to_string();
    };
    let end = start + 1 + end_rel;
    let body = &input[start + 1..end];
    let first = body.split(',').next().unwrap_or(body).trim();
    let first_value = if let Some((lo, hi)) = first.split_once('-') {
        let lo: i64 = lo.trim().parse().unwrap_or(1);
        let _hi: i64 = hi.trim().parse().unwrap_or(lo);
        lo
    } else {
        first.parse::<i64>().unwrap_or(1)
    };
    let mut out = String::with_capacity(input.len());
    out.push_str(&input[..start]);
    out.push_str(&first_value.to_string());
    out.push_str(&input[end + 1..]);
    out
}
```

Add `percent-encoding` to `crates/reader-content/Cargo.toml` dependencies if not already present (it is a transitive dependency via other crates; check first).

- [ ] **Step 2.4: Verify Cargo.toml has `percent-encoding`**

Run: `grep -n "percent-encoding" crates/reader-content/Cargo.toml`
If empty, add to `[dependencies]`:

```toml
percent-encoding = "2"
```

- [ ] **Step 2.5: Run tests to verify they pass**

Run: `cargo test -p reader-content --test analyze_url`
Expected: All 11 tests PASS.

- [ ] **Step 2.6: Commit**

```bash
git add crates/reader-content/src/analyze_url.rs crates/reader-content/Cargo.toml crates/reader-content/tests/analyze_url.rs
git commit -m "feat(reader-content): add static template + page-list expander

AnalyzeUrlContext + expand_static_templates port Swift
SearchRequestContext + PageListExpander first-value behavior.
Handles {{key}}/{{keyword}}/{{page}}/{{pageMinus}}/{{pagePlus}}
and Legado <1,3,5>/<1-3> page-list patterns."
```

---

## Task 3: `AnalyzeUrl::build_request` — HostHttpRequest assembly (no JS yet)

**Files:**
- Modify: `crates/reader-content/src/analyze_url.rs`
- Modify: `crates/reader-content/tests/analyze_url.rs`

- [ ] **Step 3.1: Write the failing test — plain GET builds a descriptor**

Append to `crates/reader-content/tests/analyze_url.rs`:

```rust
use reader_content::analyze_url::AnalyzeUrl;
use reader_contract::remote::HostHttpRequest;

#[test]
fn build_request_plain_get_url() {
    let ctx = AnalyzeUrlContext::for_search("mirror", 1);
    let request = AnalyzeUrl::build_request(
        "https://example.test/search?q={{key}}",
        &ctx,
        "https://example.test", // base URL for relative resolution
        &Default::default(),
    )
    .expect("plain GET builds");
    assert_eq!(request.url, "https://example.test/search?q=mirror");
    assert_eq!(request.method, "GET");
    assert!(request.body.is_empty());
}

#[test]
fn build_request_post_with_body_and_charset() {
    let ctx = AnalyzeUrlContext::for_search("mirror", 1);
    let raw = r#"https://example.test/search, {"method":"POST","body":"k={{key}}","charset":"gbk"}"#;
    let request = AnalyzeUrl::build_request(raw, &ctx, "https://example.test", &Default::default())
        .expect("POST+body builds");
    assert_eq!(request.url, "https://example.test/search");
    assert_eq!(request.method, "POST");
    assert_eq!(request.body, "k=mirror");
    assert_eq!(request.charset.as_deref(), Some("gbk"));
}

#[test]
fn build_request_merges_source_headers_and_dsl_headers() {
    let ctx = AnalyzeUrlContext::for_search("k", 1);
    let mut source_headers = serde_json::Map::new();
    source_headers.insert(
        "User-Agent".to_string(),
        serde_json::Value::String("ReaderCoreTest".to_string()),
    );
    let raw = r#"https://example.test/search, {"method":"POST","body":"k=test","headers":{"X-Step":"search"}}"#;
    let request = AnalyzeUrl::build_request(raw, &ctx, "https://example.test", &source_headers)
        .expect("header merge builds");
    let headers = request.headers.as_object().expect("headers object");
    assert_eq!(headers["User-Agent"].as_str(), Some("ReaderCoreTest"));
    assert_eq!(headers["X-Step"].as_str(), Some("search"));
}

#[test]
fn build_request_resolves_relative_url_against_base() {
    let ctx = AnalyzeUrlContext::for_url();
    let request = AnalyzeUrl::build_request(
        "/book/123/chapter/1",
        &ctx,
        "https://example.test",
        &Default::default(),
    )
    .expect("relative URL builds");
    assert_eq!(request.url, "https://example.test/book/123/chapter/1");
}

#[test]
fn build_request_rejects_non_http_scheme() {
    let ctx = AnalyzeUrlContext::for_url();
    let result = AnalyzeUrl::build_request(
        "file:///etc/passwd",
        &ctx,
        "https://example.test",
        &Default::default(),
    );
    assert!(result.is_err());
}
```

- [ ] **Step 3.2: Run test to verify it fails**

Run: `cargo test -p reader-content --test analyze_url build_request`
Expected: FAIL — `AnalyzeUrl` type not found.

- [ ] **Step 3.3: Implement `AnalyzeUrl::build_request`**

Append to `crates/reader-content/src/analyze_url.rs`:

```rust
/// Errors raised by [`AnalyzeUrl::build_request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyzeUrlError {
    Dsl(UrlDslParseError),
    InvalidUrl(String),
    JsUnsupported(String),
}

impl std::fmt::Display for AnalyzeUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyzeUrlError::Dsl(e) => write!(f, "URL DSL parse error: {e}"),
            AnalyzeUrlError::InvalidUrl(msg) => write!(f, "invalid URL: {msg}"),
            AnalyzeUrlError::JsUnsupported(msg) => {
                write!(f, "URL JS execution unsupported in this build: {msg}")
            }
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
    /// 3. Re-expand static templates on the URL part and the body.
    /// 4. Resolve relative URL against `base_url`.
    /// 5. Validate the URL is `http`/`https`.
    /// 6. Merge source-level headers with DSL headers (DSL wins).
    /// 7. Set default User-Agent if none.
    /// 8. Assemble `HostHttpRequest` with method/charset/body/headers/retry/
    ///    redirect/cookie-jar.
    ///
    /// JS execution (URL `@js:`/`<js>` and DSL option `js`) is handled in
    /// Task 6 via [`AnalyzeUrl::build_request_with_js`].
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

    fn assemble(
        dsl: UrlDslResult,
        ctx: &AnalyzeUrlContext,
        base_url: &str,
        source_headers: &Map<String, Value>,
    ) -> Result<HostHttpRequest, AnalyzeUrlError> {
        let final_url = resolve_relative_url(&dsl.url, base_url);
        validate_absolute_http_url(&final_url)?;

        let body = dsl
            .options
            .body
            .as_deref()
            .map(|b| expand_static_templates(b, ctx))
            .unwrap_or_default();

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
        // Apply static templates to header string values.
        let headers: Map<String, Value> = headers
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
        let mut headers = headers;
        if method == "POST" && !body.is_empty() && !headers.contains_key("Content-Type") {
            let charset = content_type_charset(&dsl.options.charset);
            headers.insert(
                "Content-Type".to_string(),
                Value::String(format!(
                    "application/x-www-form-urlencoded; charset={charset}"
                )),
            );
        }

        let retry = if dsl.options.retry > 0 {
            Some(reader_contract::remote::HostHttpRetryPolicy {
                max_attempts: dsl.options.retry,
                backoff_millis: None,
            })
        } else {
            None
        };

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

fn resolve_relative_url(url: &str, base_url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
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
        return Err(AnalyzeUrlError::InvalidUrl(format!(
            "missing host: {url}"
        )));
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
```

- [ ] **Step 3.4: Run tests to verify they pass**

Run: `cargo test -p reader-content --test analyze_url`
Expected: All 16 tests PASS.

- [ ] **Step 3.5: Commit**

```bash
git add crates/reader-content/src/analyze_url.rs crates/reader-content/tests/analyze_url.rs
git commit -m "feat(reader-content): AnalyzeUrl::build_request assembles HostHttpRequest

Ports Swift BookSourceRequestBuilder.buildSearchRequest (non-JS path):
static template expansion, URL DSL parse, relative-URL resolution,
header merge (source + DSL), auto Content-Type for POST+body,
charset + retry + redirect wiring into HostHttpRequest descriptor.
JS path (build_request_with_js) follows in next task."
```

---

## Task 4: Wire `book.search` auto-build into runtime

**Files:**
- Modify: `crates/reader-contract/src/remote.rs`
- Modify: `crates/reader-runtime/src/remote.rs`
- Modify: `crates/reader-runtime/src/remote.rs` tests if present

- [ ] **Step 4.1: Write the failing test — auto-build from `searchUrl` template**

Create or append to `crates/reader-runtime/tests/auto_build_search.rs`:

```rust
use reader_contract::{remote::BookSearchParams, Command};
use reader_runtime::Runtime;
use serde_json::json;

#[test]
fn book_search_auto_builds_request_from_search_url_template() {
    let runtime = Runtime::with_in_memory_storage();
    let source = json!({
        "sourceId": "auto-build-source",
        "name": "Auto Build Source",
        "baseUrl": "https://example.test",
        "rules": {},
        "bookSource": {
            "bookSourceName": "Auto Build Source",
            "bookSourceUrl": "https://example.test",
            "searchUrl": "https://example.test/search?q={{key}}",
            "ruleSearch": "$.books[*]"
        }
    });
    runtime.dispatch(Command::new(
        1001,
        "source.import",
        json!({ "sourceId": "auto-build-source", "name": "Auto Build Source", "baseUrl": "https://example.test", "bookSource": source["bookSource"] }),
    ));

    let cmd = Command::new(
        1002,
        "book.search",
        json!({
            "sourceId": "auto-build-source",
            "keyword": "mirror",
            "page": 1,
            "source": source["bookSource"],
        }),
    );
    let pending = runtime.dispatch(cmd).expect_pending();
    assert_eq!(pending.capability.as_str(), "http.execute");
    let params = pending.params.as_object().expect("params object");
    assert_eq!(params["url"].as_str(), Some("https://example.test/search?q=mirror"));
    assert_eq!(params["method"].as_str(), Some("GET"));
}
```

> Note: the exact `Runtime` / `dispatch` / `expect_pending` API surface should be matched to the existing `reader-runtime` test helpers — inspect `crates/reader-runtime/tests/` for the established pattern and reuse it.

- [ ] **Step 4.2: Run test to verify it fails**

Run: `cargo test -p reader-runtime --test auto_build_search`
Expected: FAIL — `keyword` field unknown on `BookSearchParams`, or auto-build path not taken.

- [ ] **Step 4.3: Add `keyword`/`page` fields to `BookSearchParams`**

In `crates/reader-contract/src/remote.rs`, locate `BookSearchParams` and add the two fields:

```rust
pub struct BookSearchParams {
    pub source_id: String,
    #[serde(default = "empty_string")]
    pub search_response: String,
    #[serde(default)]
    pub search_request: Option<HostHttpRequest>,
    /// Search keyword. When `search_request` is `None` and `search_response`
    /// is empty, Core auto-builds the request from `source.bookSource.searchUrl`
    /// + this keyword (AnalyzeUrl S3/S4 closure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    /// 1-based page number for paginated search URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_inline_source")]
    pub source: Option<Value>,
}
```

Also update any `BookSearchParams::validate` / builder / test fixtures that need the new fields. The fields are `Option` + `#[serde(default)]`, so existing fixtures that omit them remain valid.

- [ ] **Step 4.4: Wire `book_search` to auto-build**

In `crates/reader-runtime/src/remote.rs`, locate `fn book_search` and add the auto-build path before `pending_or_missing_response`:

```rust
fn book_search(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let mut params: BookSearchParams =
        parse_params(contract::methods::BOOK_SEARCH, &cmd.params)?;
    // Auto-build path: if no search_request and no search_response, try to
    // construct the descriptor from source.bookSource.searchUrl + keyword.
    if params.search_request.is_none()
        && params.search_response.is_empty()
        && params.keyword.as_deref().is_some_and(|k| !k.is_empty())
    {
        if let Some(request) = build_search_request_from_source(&params, state)? {
            params.search_request = Some(request);
        }
    }
    if let Some(pending) = pending_or_missing_response(
        &params.search_response,
        params.search_request.clone(),
        "searchResponse",
        "searchRequest",
        RemoteHostContinuation::BookSearch(params.clone()),
    )? {
        return Ok(pending);
    }
    book_search_from_params(params, state).map(RemoteCommandResult::Complete)
}

fn build_search_request_from_source(
    params: &BookSearchParams,
    state: &RemoteState,
) -> Result<Option<HostHttpRequest>, CoreError> {
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let Some(book_source) = source.legado_book_source() else {
        return Ok(None);
    };
    let Some(search_url) = book_source.search_url.as_deref() else {
        return Ok(None);
    };
    let keyword = params.keyword.as_deref().unwrap_or("");
    let page = params.page.unwrap_or(1);
    let ctx = reader_content::analyze_url::AnalyzeUrlContext::for_search(keyword, page);
    let mut source_headers = serde_json::Map::new();
    if let Some(headers) = book_source.header.as_ref().and_then(|h| h.as_object()) {
        source_headers = headers.clone();
    }
    let request = reader_content::analyze_url::AnalyzeUrl::build_request(
        search_url,
        &ctx,
        &source.base_url,
        &source_headers,
    )
    .map_err(|err| {
        CoreError::invalid_params(format!("failed to build search request: {err}"))
    })?;
    Ok(Some(request))
}
```

- [ ] **Step 4.5: Run the new test to verify it passes**

Run: `cargo test -p reader-runtime --test auto_build_search`
Expected: PASS.

- [ ] **Step 4.6: Run the full runtime test suite to ensure no regressions**

Run: `cargo test -p reader-runtime`
Expected: All existing tests still PASS (the new fields are optional with serde defaults).

- [ ] **Step 4.7: Commit**

```bash
git add crates/reader-contract/src/remote.rs crates/reader-runtime/src/remote.rs crates/reader-runtime/tests/auto_build_search.rs
git commit -m "feat(reader-runtime): book.search auto-builds HostHttpRequest from searchUrl

Adds keyword/page to BookSearchParams. When search_request is None
and search_response is empty, Core calls AnalyzeUrl::build_request
on source.bookSource.searchUrl to construct the descriptor. Caller
no longer needs to pre-build the request for static-template sources.
JS-bearing sources still require the explicit JS path (next task)."
```

---

## Task 5: Wire `book.detail` / `book.toc` / `chapter.content` auto-build

**Files:**
- Modify: `crates/reader-contract/src/remote.rs`
- Modify: `crates/reader-runtime/src/remote.rs`
- Modify: `crates/reader-runtime/tests/auto_build_search.rs` (or new tests)

- [ ] **Step 5.1: Write failing tests for detail/toc/chapter auto-build**

Append to `crates/reader-runtime/tests/auto_build_search.rs`:

```rust
#[test]
fn book_detail_auto_builds_request_from_book_url() {
    let runtime = Runtime::with_in_memory_storage();
    let source = json!({
        "bookSourceName": "Auto Build",
        "bookSourceUrl": "https://example.test",
        "ruleBookInfo": "$.detail"
    });
    runtime.dispatch(Command::new(2001, "source.import", json!({
        "sourceId": "auto-build-source",
        "name": "Auto Build",
        "baseUrl": "https://example.test",
        "bookSource": source,
    })));

    let cmd = Command::new(2002, "book.detail", json!({
        "sourceId": "auto-build-source",
        "book": { "bookId": "/book/777", "title": "Test" },
        "source": source,
    }));
    let pending = runtime.dispatch(cmd).expect_pending();
    let params = pending.params.as_object().expect("params object");
    assert_eq!(params["url"].as_str(), Some("https://example.test/book/777"));
    assert_eq!(params["method"].as_str(), Some("GET"));
}

#[test]
fn book_toc_auto_builds_request_from_toc_url() {
    let runtime = Runtime::with_in_memory_storage();
    let source = json!({
        "bookSourceName": "Auto Build",
        "bookSourceUrl": "https://example.test",
        "ruleToc": "$.chapters"
    });
    runtime.dispatch(Command::new(3001, "source.import", json!({
        "sourceId": "auto-build-source",
        "name": "Auto Build",
        "baseUrl": "https://example.test",
        "bookSource": source,
    })));

    let cmd = Command::new(3002, "book.toc", json!({
        "sourceId": "auto-build-source",
        "bookId": "https://example.test/book/777",
        "tocUrl": "https://example.test/book/777/catalog",
        "source": source,
    }));
    let pending = runtime.dispatch(cmd).expect_pending();
    let params = pending.params.as_object().expect("params object");
    assert_eq!(params["url"].as_str(), Some("https://example.test/book/777/catalog"));
}

#[test]
fn chapter_content_auto_builds_request_from_chapter_url() {
    let runtime = Runtime::with_in_memory_storage();
    let source = json!({
        "bookSourceName": "Auto Build",
        "bookSourceUrl": "https://example.test",
        "ruleContent": "article#chapter@html"
    });
    runtime.dispatch(Command::new(4001, "source.import", json!({
        "sourceId": "auto-build-source",
        "name": "Auto Build",
        "baseUrl": "https://example.test",
        "bookSource": source,
    })));

    let cmd = Command::new(4002, "chapter.content", json!({
        "sourceId": "auto-build-source",
        "bookId": "https://example.test/book/777",
        "chapterTitle": "Chapter 1",
        "chapterUrl": "https://example.test/book/777/chapter/1",
        "source": source,
    }));
    let pending = runtime.dispatch(cmd).expect_pending();
    let params = pending.params.as_object().expect("params object");
    assert_eq!(params["url"].as_str(), Some("https://example.test/book/777/chapter/1"));
}
```

- [ ] **Step 5.2: Run tests to verify they fail**

Run: `cargo test -p reader-runtime --test auto_build_search`
Expected: FAIL — `tocUrl` / `chapterUrl` unknown fields, auto-build paths not taken.

- [ ] **Step 5.3: Add `tocUrl` / `chapterUrl` fields + wire auto-build**

In `crates/reader-contract/src/remote.rs`:

```rust
// BookTocParams: add
#[serde(default, skip_serializing_if = "Option::is_none")]
pub toc_url: Option<String>,

// ChapterContentParams: add
#[serde(default, skip_serializing_if = "Option::is_none")]
pub chapter_url: Option<String>,
```

In `crates/reader-runtime/src/remote.rs`, add auto-build paths mirroring Task 4:

```rust
fn book_detail(...) -> Result<RemoteCommandResult, CoreError> {
    let mut params: BookDetailParams = parse_params(...)?;
    if params.detail_request.is_none() && params.detail_response.is_empty() {
        if let Some(book_id) = non_empty_book_id(&params.book) {
            // detail URL = book.bookId (Legado convention: bookId is the detail URL).
            let ctx = reader_content::analyze_url::AnalyzeUrlContext::for_url();
            if let Some(request) = build_url_request_from_source(
                &params.source_id, &params.source, &book_id, state,
            )? {
                params.detail_request = Some(request);
            }
        }
    }
    // ... existing pending_or_missing_response + book_detail_from_params
}

fn book_toc(...) -> Result<RemoteCommandResult, CoreError> {
    let mut params: BookTocParams = parse_params(...)?;
    if params.toc_request.is_none() && params.toc_response.is_empty() {
        if let Some(toc_url) = params.toc_url.clone() {
            let ctx = reader_content::analyze_url::AnalyzeUrlContext::for_url();
            if let Some(request) = build_url_request_from_source(
                &params.source_id, &params.source, &toc_url, state,
            )? {
                params.toc_request = Some(request);
            }
        }
    }
    // ...
}

fn chapter_content(...) -> Result<RemoteCommandResult, CoreError> {
    let mut params: ChapterContentParams = parse_params(...)?;
    if params.js_rule.is_none()
        && params.chapter_request.is_none()
        && params.chapter_response.is_empty()
    {
        if let Some(chapter_url) = params.chapter_url.clone() {
            let ctx = reader_content::analyze_url::AnalyzeUrlContext::for_url();
            if let Some(request) = build_url_request_from_source(
                &params.source_id, &params.source, &chapter_url, state,
            )? {
                params.chapter_request = Some(request);
            }
        }
    }
    // ...
}

fn build_url_request_from_source(
    source_id: &str,
    inline_source: &Option<Value>,
    raw_url: &str,
    state: &RemoteState,
) -> Result<Option<HostHttpRequest>, CoreError> {
    let source = resolve_source(state.storage(), source_id, inline_source)?;
    let book_source = source.legado_book_source();
    let base_url = source.base_url.clone();
    let mut source_headers = serde_json::Map::new();
    if let Some(bs) = book_source.as_ref() {
        if let Some(headers) = bs.header.as_ref().and_then(|h| h.as_object()) {
            source_headers = headers.clone();
        }
    }
    let ctx = reader_content::analyze_url::AnalyzeUrlContext::for_url();
    let request = reader_content::analyze_url::AnalyzeUrl::build_request(
        raw_url, &ctx, &base_url, &source_headers,
    ).map_err(|err| CoreError::invalid_params(format!("failed to build request: {err}")))?;
    Ok(Some(request))
}

fn non_empty_book_id(book: &Value) -> Option<String> {
    book.get("bookId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
```

- [ ] **Step 5.4: Run all auto-build tests**

Run: `cargo test -p reader-runtime --test auto_build_search`
Expected: All 4 tests PASS.

- [ ] **Step 5.5: Run full runtime test suite for regressions**

Run: `cargo test -p reader-runtime`
Expected: All tests PASS.

- [ ] **Step 5.6: Commit**

```bash
git add crates/reader-contract/src/remote.rs crates/reader-runtime/src/remote.rs crates/reader-runtime/tests/auto_build_search.rs
git commit -m "feat(reader-runtime): book.detail/toc/chapter auto-build descriptors

Adds tocUrl to BookTocParams and chapterUrl to ChapterContentParams.
When *_request is None and *_response is empty, Core auto-builds the
descriptor from the provided URL (resolved against source.baseUrl).
book.detail uses book.bookId as the detail URL (Legado convention)."
```

---

## Task 6: URL-embedded JS execution via `reader-js` (makeSearchRequestExecutingURLJS equivalent)

**Files:**
- Modify: `crates/reader-content/src/analyze_url.rs`
- Modify: `crates/reader-content/tests/analyze_url.rs`

- [ ] **Step 6.1: Write the failing test — `@js:` URL executes and yields final URL**

Append to `crates/reader-content/tests/analyze_url.rs`:

```rust
use reader_content::analyze_url::AnalyzeUrl;
use reader_js::QuickJsSandbox;

#[test]
fn build_request_with_js_evaluates_url_expression() {
    let sandbox = QuickJsSandbox::default();
    let ctx = AnalyzeUrlContext::for_search("mirror", 1);
    // @js: expression returns the final URL string.
    let raw = "https://example.test/search@js:result + '?from=js'";
    let request = AnalyzeUrl::build_request_with_js(
        raw, &ctx, "https://example.test", &Default::default(), &sandbox,
    ).expect("JS URL builds");
    // result starts as the pre-@js URL portion
    assert_eq!(request.url, "https://example.test/search?from=js");
}

#[test]
fn build_request_with_js_option_replaces_url() {
    let sandbox = QuickJsSandbox::default();
    let ctx = AnalyzeUrlContext::for_search("mirror", 1);
    let raw = r#"https://example.test/search, {"js":"'https://example.test/js-driven?q=' + key"}"#;
    let request = AnalyzeUrl::build_request_with_js(
        raw, &ctx, "https://example.test", &Default::default(), &sandbox,
    ).expect("DSL js option builds");
    assert_eq!(request.url, "https://example.test/js-driven?q=mirror");
}
```

- [ ] **Step 6.2: Run test to verify it fails**

Run: `cargo test -p reader-content --test analyze_url build_request_with_js`
Expected: FAIL — `build_request_with_js` not found.

- [ ] **Step 6.3: Implement `build_request_with_js`**

Append to `crates/reader-content/src/analyze_url.rs`:

```rust
use reader_js::{JsEvaluation, JsResult, JsSandbox, JsError};

impl AnalyzeUrl {
    /// Build a [`HostHttpRequest`] with URL-embedded JS execution. Ports
    /// Swift `makeSearchRequestExecutingURLJS`.
    ///
    /// JS execution order (mirrors Swift `resolveURLJSIfNeeded`):
    /// 1. Expand static templates on the raw URL.
    /// 2. Parse URL DSL.
    /// 3. If URL part contains `@js:`/`<js>`: evaluate, result replaces URL.
    /// 4. If DSL option `js` is set: evaluate, result replaces URL.
    /// 5. Assemble `HostHttpRequest` via [`AnalyzeUrl::assemble`].
    pub fn build_request_with_js(
        raw_url: &str,
        ctx: &AnalyzeUrlContext,
        base_url: &str,
        source_headers: &Map<String, Value>,
        sandbox: &dyn JsSandbox,
    ) -> Result<HostHttpRequest, AnalyzeUrlError> {
        let expanded = expand_static_templates(raw_url, ctx);
        let mut dsl = UrlDslParser::parse(&expanded)?;

        // URL-side @js:/<js>
        if dsl.has_js_expression {
            let expr = dsl.js_expression.clone().unwrap_or_default();
            let pre_url = strip_js_expression(&dsl.url);
            let evaluated = evaluate_url_js(&expr, &pre_url, ctx, sandbox)?;
            dsl = UrlDslResult {
                url: evaluated,
                options: dsl.options.clone(),
                has_js_expression: false,
                js_expression: None,
                js_classification: JsExpressionClassification::None,
            };
        }

        // DSL option js
        if let Some(js_str) = dsl.options.js.clone() {
            let trimmed = js_str.trim();
            if !trimmed.is_empty() {
                let evaluated = evaluate_url_js(trimmed, &dsl.url, ctx, sandbox)?;
                let mut options = dsl.options.clone();
                options.js = None;
                dsl = UrlDslResult {
                    url: evaluated,
                    options,
                    has_js_expression: false,
                    js_expression: None,
                    js_classification: JsExpressionClassification::None,
                };
            }
        }

        Self::assemble(dsl, ctx, base_url, source_headers)
    }
}

fn strip_js_expression(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if let Some(idx) = lower.find("@js:") {
        return url[..idx].to_string();
    }
    if let Some(start) = lower.find("<js>") {
        if let Some(end) = lower.rfind("</js>") {
            let mut out = String::with_capacity(url.len());
            out.push_str(&url[..start]);
            out.push_str(&url[end + 5..]);
            return out;
        }
    }
    url.to_string()
}

fn evaluate_url_js(
    expression: &str,
    result: &str,
    ctx: &AnalyzeUrlContext,
    sandbox: &dyn JsSandbox,
) -> Result<String, AnalyzeUrlError> {
    let wrapper = build_url_js_wrapper(expression, result, ctx);
    let JsEvaluation { value, .. } = sandbox
        .evaluate(&wrapper)
        .map_err(|err| AnalyzeUrlError::JsUnsupported(format!("JS eval failed: {err}")))?;
    let url_str = match value {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let trimmed = url_str.trim().to_string();
    if trimmed.is_empty() {
        return Err(AnalyzeUrlError::JsUnsupported(
            "URL JS returned empty value".into(),
        ));
    }
    Ok(trimmed)
}

fn build_url_js_wrapper(expression: &str, result: &str, ctx: &AnalyzeUrlContext) -> String {
    // Minimal wrapper exposing key/keyword/page/result/baseUrl. Ports a subset
    // of Swift `urlJSWrapperScript` — full `java.*` bindings are owned by the
    // sandbox's host callback registry.
    format!(
        r#"(function() {{
            var key = {key_literal};
            var keyword = key;
            var page = {page};
            var pageMinus = {page_minus};
            var pagePlus = {page_plus};
            var result = {result_literal};
            try {{
                return (function() {{ return {expression}; }})();
            }} catch (e) {{
                return String(e);
            }}
        }})();"#,
        key_literal = serde_json::to_string(&ctx.raw_keyword).unwrap_or_else(|_| "\"\"".into()),
        page = ctx.page,
        page_minus = ctx.page_minus_string,
        page_plus = ctx.page_plus_string,
        result_literal = serde_json::to_string(result).unwrap_or_else(|_| "\"\"".into()),
        expression = expression,
    )
}
```

- [ ] **Step 6.4: Run tests to verify they pass**

Run: `cargo test -p reader-content --test analyze_url`
Expected: All 18 tests PASS, including the 2 new JS tests.

- [ ] **Step 6.5: Commit**

```bash
git add crates/reader-content/src/analyze_url.rs crates/reader-content/tests/analyze_url.rs
git commit -m "feat(reader-content): AnalyzeUrl::build_request_with_js via reader-js

Ports Swift makeSearchRequestExecutingURLJS. Handles @js:/<js> in the
URL part and the DSL 'js' option. Wraps the expression with key/
keyword/page/pageMinus/pagePlus/result bindings. Uses the existing
reader-js JsSandbox trait (no new JS engine in Core)."
```

---

## Task 7: Real Legado book source host-replay fixture suite (desensitized)

**Files:**
- Create: `tests/fixtures/host_replay/analyze_url_build_suite.json`
- Modify: `crates/reader-cli/tests/host_replay.rs` (or create new test)

- [ ] **Step 7.1: Create the desensitized fixture suite**

Create `tests/fixtures/host_replay/analyze_url_build_suite.json` with steps that exercise the auto-build path — the caller passes ONLY `source.bookSource.searchUrl` template + `keyword`, NOT a pre-built `searchRequest`:

```json
{
  "steps": [
    {
      "name": "legado.gamma.search.auto_build.gbk_post",
      "completionRequestId": 4001,
      "command": {
        "protocolVersion": 1,
        "requestId": 4000,
        "method": "book.search",
        "params": {
          "sourceId": "legado-gamma-desensitized",
          "keyword": "nebula",
          "page": 1,
          "source": {
            "bookSourceName": "Legado Gamma Desensitized",
            "bookSourceUrl": "https://gamma.example.test",
            "enabledCookieJar": true,
            "header": {
              "User-Agent": "REDACTED_LEGADO_UA"
            },
            "searchUrl": "https://gamma.example.test/search.php,{\"method\":\"POST\",\"body\":\"searchkey={{key}}&page={{page}}\",\"charset\":\"gbk\"}",
            "ruleSearch": "$.books[*]",
            "ruleBookInfo": "$.detail",
            "ruleToc": "$.chapters",
            "ruleContent": "article#chapter@html"
          }
        }
      },
      "expectHostRequest": {
        "capability": "http.execute",
        "params": {
          "url": "https://gamma.example.test/search.php",
          "method": "POST",
          "headers": {
            "User-Agent": "REDACTED_LEGADO_UA",
            "Content-Type": "application/x-www-form-urlencoded; charset=gbk"
          },
          "body": "searchkey=nebula&page=1",
          "charset": "gbk"
        }
      },
      "hostResult": {
        "status": 200,
        "headers": {
          "content-type": "application/json; charset=gbk"
        },
        "finalUrl": "https://gamma.example.test/search.php",
        "charsetHint": "gbk",
        "body": "{\"books\":[{\"bookId\":\"gamma-2001\",\"title\":\"Nebula Protocol\",\"author\":\"Author G\"}]}"
      },
      "expectResult": {
        "sourceId": "legado-gamma-desensitized",
        "books": [
          {
            "bookId": "gamma-2001",
            "title": "Nebula Protocol",
            "author": "Author G"
          }
        ]
      }
    },
    {
      "name": "legado.gamma.detail.auto_build.relative_url",
      "completionRequestId": 4011,
      "command": {
        "protocolVersion": 1,
        "requestId": 4010,
        "method": "book.detail",
        "params": {
          "sourceId": "legado-gamma-desensitized",
          "book": { "bookId": "/book/2001", "title": "Nebula Protocol" },
          "source": {
            "bookSourceName": "Legado Gamma Desensitized",
            "bookSourceUrl": "https://gamma.example.test",
            "ruleBookInfo": "$.detail"
          }
        }
      },
      "expectHostRequest": {
        "capability": "http.execute",
        "params": {
          "url": "https://gamma.example.test/book/2001",
          "method": "GET",
          "headers": {
            "User-Agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1"
          },
          "body": "",
          "charset": "utf-8"
        }
      },
      "hostResult": {
        "status": 200,
        "headers": { "content-type": "application/json; charset=utf-8" },
        "finalUrl": "https://gamma.example.test/book/2001",
        "charsetHint": "utf-8",
        "body": "{\"detail\":{\"bookId\":\"/book/2001\",\"title\":\"Nebula Protocol\",\"author\":\"Author G\",\"intro\":\"Desensitized detail.\",\"tocUrl\":\"/book/2001/catalog\"}}"
      },
      "expectResult": {
        "sourceId": "legado-gamma-desensitized",
        "book": {
          "bookId": "/book/2001",
          "title": "Nebula Protocol",
          "author": "Author G",
          "intro": "Desensitized detail."
        }
      }
    },
    {
      "name": "legado.gamma.toc.auto_build.from_toc_url",
      "completionRequestId": 4021,
      "command": {
        "protocolVersion": 1,
        "requestId": 4020,
        "method": "book.toc",
        "params": {
          "sourceId": "legado-gamma-desensitized",
          "bookId": "/book/2001",
          "tocUrl": "/book/2001/catalog",
          "source": {
            "bookSourceName": "Legado Gamma Desensitized",
            "bookSourceUrl": "https://gamma.example.test",
            "ruleToc": "$.chapters"
          }
        }
      },
      "expectHostRequest": {
        "capability": "http.execute",
        "params": {
          "url": "https://gamma.example.test/book/2001/catalog",
          "method": "GET",
          "body": "",
          "charset": "utf-8"
        }
      },
      "hostResult": {
        "status": 200,
        "headers": { "content-type": "application/json; charset=utf-8" },
        "finalUrl": "https://gamma.example.test/book/2001/catalog",
        "charsetHint": "utf-8",
        "body": "{\"chapters\":[{\"title\":\"Chapter 1\",\"url\":\"/book/2001/chapter/1\"}]}"
      },
      "expectResult": {
        "sourceId": "legado-gamma-desensitized",
        "bookId": "/book/2001",
        "toc": [
          { "index": 0, "title": "Chapter 1", "url": "/book/2001/chapter/1" }
        ]
      }
    },
    {
      "name": "legado.gamma.chapter.auto_build.from_chapter_url",
      "completionRequestId": 4031,
      "command": {
        "protocolVersion": 1,
        "requestId": 4030,
        "method": "chapter.content",
        "params": {
          "sourceId": "legado-gamma-desensitized",
          "bookId": "/book/2001",
          "chapterTitle": "Chapter 1",
          "chapterUrl": "/book/2001/chapter/1",
          "source": {
            "bookSourceName": "Legado Gamma Desensitized",
            "bookSourceUrl": "https://gamma.example.test",
            "ruleContent": "article#chapter@html"
          }
        }
      },
      "expectHostRequest": {
        "capability": "http.execute",
        "params": {
          "url": "https://gamma.example.test/book/2001/chapter/1",
          "method": "GET",
          "body": "",
          "charset": "utf-8"
        }
      },
      "hostResult": {
        "status": 200,
        "headers": { "content-type": "text/html; charset=utf-8" },
        "finalUrl": "https://gamma.example.test/book/2001/chapter/1",
        "charsetHint": "utf-8",
        "body": "<html><body><article id=\"chapter\"><p>Auto-built chapter content.</p></article></body></html>"
      },
      "expectResult": {
        "sourceId": "legado-gamma-desensitized",
        "bookId": "/book/2001",
        "chapterTitle": "Chapter 1",
        "content": "Auto-built chapter content.",
        "via": "rule"
      }
    }
  ]
}
```

- [ ] **Step 7.2: Wire the new fixture into the CLI host-replay test harness**

Inspect `tools/reader-cli/tests/host_replay.rs` for the existing fixture-loading pattern (it already loads `legado_desensitized_corpus_suite.json`). Add `analyze_url_build_suite.json` to the same loader list.

- [ ] **Step 7.3: Run the host-replay test**

Run: `cargo test -p reader-cli --test host_replay`
Expected: All steps PASS, including the 4 new auto-build steps.

- [ ] **Step 7.4: Commit**

```bash
git add tests/fixtures/host_replay/analyze_url_build_suite.json tools/reader-cli/tests/host_replay.rs
git commit -m "test(reader-cli): host-replay suite for AnalyzeUrl auto-build path

4 desensitized steps exercising real Legado book source shapes:
- search: POST + gbk charset + body template
- detail: relative URL resolution against bookSourceUrl
- toc: tocUrl auto-build with relative path
- chapter: chapterUrl auto-build with relative path
Caller passes only source.bookSource + keyword/bookId/tocUrl/chapterUrl,
no pre-built request — Core builds the descriptor."
```

---

## Task 8: Conformance test + field-coverage verification

**Files:**
- Modify: `tools/reader-cli/src/conformance.rs` (or its tests)
- Modify: `protocol/fixtures/conformance/commands/valid-book-search.json` (if needed)

- [ ] **Step 8.1: Add a conformance fixture for the auto-build path**

Create `protocol/fixtures/conformance/commands/valid-book-search-auto-build.json`:

```json
{
  "protocolVersion": 1,
  "requestId": 9001,
  "method": "book.search",
  "params": {
    "sourceId": "conformance-auto-build",
    "keyword": "conformance",
    "page": 1,
    "source": {
      "bookSourceName": "Conformance Auto Build",
      "bookSourceUrl": "https://conformance.example.test",
      "searchUrl": "https://conformance.example.test/search?q={{key}}",
      "ruleSearch": "$.books[*]"
    }
  }
}
```

- [ ] **Step 8.2: Run conformance**

Run: `cargo run -p reader-cli -- --conformance`
Expected: Exit code 0; new fixture accepted by the schema validator (the new `keyword`/`page`/`tocUrl`/`chapterUrl` fields are optional, so the schema accepts them).

If the conformance runner rejects the new fixture, update `protocol/reader-command.schema.json` to include the new optional fields.

- [ ] **Step 8.3: Run the full verification suite**

Run:
```
cargo test -p reader-content -p reader-runtime -p reader-contract -p reader-cli
cargo run -p reader-cli -- --conformance
```
Expected: All tests PASS; conformance exits 0.

- [ ] **Step 8.4: Commit**

```bash
git add protocol/fixtures/conformance/commands/valid-book-search-auto-build.json protocol/reader-command.schema.json
git commit -m "test(protocol): conformance fixture for book.search auto-build

Validates that the new keyword/page fields are accepted by the
protocol schema and the conformance runner. Updates the schema to
declare the optional fields added in Tasks 4 and 5."
```

---

## Self-Review

**1. Spec coverage check:**

| Spec requirement | Task(s) |
| --- | --- |
| searchUrl/exploreUrl template expansion ({{key}}/{{page}}/pageMinus/pagePlus) | Task 2 |
| URL DSL parsing (url + JSON options: method/charset/body/headers/encoding) | Task 1 |
| URL-embedded JS execution (makeSearchRequestExecutingURLJS equivalent) | Task 6 |
| Core produces HostHttpRequest descriptor, host executes | Task 3 (assembly) + Task 4/5 (wiring) |
| book.search builds search_request descriptor → emit http.execute → host returns → Core parses | Task 4 |
| book.detail/toc/chapter.content auto-build | Task 5 |
| CLI host replay uses real book source request descriptor validation | Task 7 |
| Field coverage against Legado AnalyzeUrl.kt (method/charset/body/headers/js/retry/redirect/cookie) | Field-coverage table at top + Task 8 conformance |
| Real Legado book source fixtures (desensitized) | Task 7 |
| Core/Host boundary (Core opens no socket) | Maintained — all tasks produce `HostHttpRequest`, never execute HTTP |
| Migration fidelity (port Swift, don't reinvent) | Tasks 1/2/3/6 port `URLDSLParser.swift` + `BookSourceRequestBuilder.swift` |
| Cargo test + conformance verification | Task 8.3 |

**Gaps:** None identified. The plan covers all 5 closure requirements.

**2. Placeholder scan:** Searched for "TBD", "TODO", "implement later", "fill in details", "similar to Task N" — none found. All steps contain complete code or exact commands.

**3. Type consistency:**
- `AnalyzeUrlContext` defined in Task 2, used in Tasks 3/4/5/6 — consistent.
- `AnalyzeUrl::build_request` signature in Task 3, referenced in Tasks 4/5 — consistent.
- `AnalyzeUrl::build_request_with_js` signature in Task 6, references `dyn JsSandbox` from `reader-js` — consistent with the trait shown in the audit.
- `HostHttpRequest` fields (url/method/headers/body/charset/follow_redirects/max_redirects/retry/use_platform_cookie_jar/session) match the existing `reader-contract::remote::HostHttpRequest` definition — verified against `crates/reader-contract/src/remote.rs`.
- `BookSearchParams.keyword`/`page` added in Task 4, used in Task 4 — consistent.
- `BookTocParams.toc_url` / `ChapterContentParams.chapter_url` added in Task 5, used in Task 5 — consistent.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-06-27-analyze-url-request-builder.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints.

**Which approach?**

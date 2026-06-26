//! Integration tests for `reader_content::analyze_url`.
//!
//! These exercise the URL DSL parser ported from Swift `URLDSLParser.swift`
//! and the AnalyzeUrl builder ported from `BookSourceRequestBuilder.swift`.

use reader_content::analyze_url::{
    expand_static_templates, AnalyzeUrl, AnalyzeUrlContext, JsExpressionClassification, UrlDslParser,
};

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
        JsExpressionClassification::RequiresJsSandbox
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

// ===== Task 2: Static template expander + page list =====

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

// ===== Task 3: AnalyzeUrl::build_request — HostHttpRequest assembly =====

#[test]
fn build_request_plain_get_url() {
    let ctx = AnalyzeUrlContext::for_search("mirror", 1);
    let request = AnalyzeUrl::build_request(
        "https://example.test/search?q={{key}}",
        &ctx,
        "https://example.test",
        &Default::default(),
    )
    .expect("plain GET builds");
    assert_eq!(request.url, "https://example.test/search?q=mirror");
    assert_eq!(request.method, "GET");
    assert!(request.body.is_none());
}

#[test]
fn build_request_post_with_body_and_charset() {
    let ctx = AnalyzeUrlContext::for_search("mirror", 1);
    let raw = r#"https://example.test/search, {"method":"POST","body":"k={{key}}","charset":"gbk"}"#;
    let request = AnalyzeUrl::build_request(raw, &ctx, "https://example.test", &Default::default())
        .expect("POST+body builds");
    assert_eq!(request.url, "https://example.test/search");
    assert_eq!(request.method, "POST");
    assert_eq!(request.body.as_deref(), Some("k=mirror"));
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

#[test]
fn build_request_auto_content_type_for_post_body() {
    let ctx = AnalyzeUrlContext::for_search("k", 1);
    let raw = r#"https://example.test/search, {"method":"POST","body":"k=test","charset":"gbk"}"#;
    let request = AnalyzeUrl::build_request(raw, &ctx, "https://example.test", &Default::default())
        .expect("POST builds");
    let headers = request.headers.as_object().expect("headers object");
    assert_eq!(
        headers["Content-Type"].as_str(),
        Some("application/x-www-form-urlencoded; charset=gbk")
    );
}

#[test]
fn build_request_rejects_js_url_in_non_js_build() {
    let ctx = AnalyzeUrlContext::for_search("k", 1);
    let raw = "https://example.test/search@js:result.replace(' ', '+')";
    let result = AnalyzeUrl::build_request(raw, &ctx, "https://example.test", &Default::default());
    assert!(result.is_err());
}

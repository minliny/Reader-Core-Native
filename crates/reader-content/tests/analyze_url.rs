//! Integration tests for `reader_content::analyze_url`.
//!
//! These exercise the URL DSL parser ported from Swift `URLDSLParser.swift`
//! and the AnalyzeUrl builder ported from `BookSourceRequestBuilder.swift`.

use reader_content::analyze_url::{JsExpressionClassification, UrlDslParser};

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

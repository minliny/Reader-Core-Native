//! Integration tests for `reader_content::analyze_url`.
//!
//! These exercise the URL DSL parser ported from Swift `URLDSLParser.swift`
//! and the AnalyzeUrl builder ported from `BookSourceRequestBuilder.swift`.

use reader_content::analyze_url::{
    expand_static_templates, AnalyzeUrl, AnalyzeUrlContext, JsExpressionClassification,
    UrlDslParser,
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
    let raw =
        r#"https://example.test/search, {"method":"POST","body":"k={{key}}","charset":"gbk"}"#;
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

#[test]
fn classify_static_variable_placeholders_are_not_js() {
    // {{source.key}} / {{source.booksourceurl}} / {{cookie.*}} are static
    // variable references, not JS expressions — they must NOT be classified
    // as RequiresJsSandbox (would route to JS path → no evaluator → error).
    for raw in &[
        "{{source.key}}",
        "{{source.bookSourceUrl}}",
        "{{cookie.token}}",
        "search.php?keyword={{key}}",
        "https://host.test/{{source.key}}/path",
    ] {
        let (expr, class) = UrlDslParser::classify_js_expression(raw);
        assert_eq!(
            class,
            JsExpressionClassification::None,
            "static placeholder {raw:?} must not be classified as JS"
        );
        assert!(expr.is_none(), "no JS expression for static {raw:?}");
    }
}

// ===== Task 2: Static template expander + page list =====

#[test]
fn expand_static_templates_replaces_key_and_page() {
    let ctx = AnalyzeUrlContext::for_search("mirror", 2);
    let out = expand_static_templates(
        "https://example.test/search?q={{key}}&p={{page}}&pm={{pageMinus}}&pp={{pagePlus}}",
        &ctx,
    );
    assert_eq!(out, "https://example.test/search?q=mirror&p=2&pm=1&pp=3");
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
    let raw =
        r#"https://example.test/search, {"method":"POST","body":"k={{key}}","charset":"gbk"}"#;
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

// ===========================================================================
// Task 6: build_request_with_js — URL-embedded JS execution
// ===========================================================================

#[test]
fn build_request_with_js_evaluates_at_js_expression() {
    let ctx = AnalyzeUrlContext::for_search("斗破苍穹", 1);
    let raw = "@js:\"https://example.test/search?q=\" + key";
    let request = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |expr, context| {
            // Toy evaluator: concatenate the variable bindings + return the
            // expression result. The real implementation uses a JS sandbox.
            assert_eq!(expr, "\"https://example.test/search?q=\" + key");
            assert_eq!(context["key"], "斗破苍穹");
            assert_eq!(context["page"], 1);
            assert_eq!(context["baseUrl"], "https://example.test");
            Ok(format!("https://example.test/search?q=斗破苍穹"))
        },
    )
    .expect("JS URL builds");
    assert_eq!(request.url, "https://example.test/search?q=斗破苍穹");
    assert_eq!(request.method, "GET");
}

#[test]
fn build_request_with_js_evaluates_js_tag_expression() {
    let ctx = AnalyzeUrlContext::for_search("test", 2);
    let raw = "<js>\"https://js.example.test/p=\" + page</js>";
    let request = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |expr, _context| {
            assert_eq!(expr, "\"https://js.example.test/p=\" + page");
            Ok("https://js.example.test/p=2".to_string())
        },
    )
    .expect("<js> URL builds");
    assert_eq!(request.url, "https://js.example.test/p=2");
}

#[test]
fn build_request_with_js_evaluates_dsl_js_option() {
    let ctx = AnalyzeUrlContext::for_search("test", 1);
    let raw = r#"https://placeholder.test,{"js":"\"https://real.example.test/k=\" + key"}"#;
    let request = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |expr, _context| {
            assert_eq!(expr, "\"https://real.example.test/k=\" + key");
            Ok("https://real.example.test/k=test".to_string())
        },
    )
    .expect("DSL js option builds");
    assert_eq!(request.url, "https://real.example.test/k=test");
}

#[test]
fn build_request_with_js_returns_js_result_as_dsl_with_post_options() {
    let ctx = AnalyzeUrlContext::for_search("test", 1);
    let raw = "@js:buildUrl(key)";
    let request = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |_expr, _context| {
            // The JS sandbox can return a full DSL string with POST options.
            Ok(r#"https://built.example.test/api,{"method":"POST","body":"q=test"}"#.to_string())
        },
    )
    .expect("JS DSL result builds");
    assert_eq!(request.url, "https://built.example.test/api");
    assert_eq!(request.method, "POST");
    assert_eq!(request.body.as_deref(), Some("q=test"));
}

#[test]
fn build_request_with_js_without_js_falls_through_to_non_js_path() {
    let ctx = AnalyzeUrlContext::for_search("test", 1);
    let raw = "https://example.test/search?q={{key}}";
    let request = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |_expr, _context| panic!("js_eval should not be called for non-JS URL"),
    )
    .expect("non-JS URL builds via fall-through");
    assert_eq!(request.url, "https://example.test/search?q=test");
}

#[test]
fn build_request_with_js_surfaces_evaluator_error() {
    let ctx = AnalyzeUrlContext::for_search("test", 1);
    let raw = "@js:throw new Error('boom')";
    let result = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |_expr, _context| Err("boom".to_string()),
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        reader_content::analyze_url::AnalyzeUrlError::JsExecution(msg) => {
            assert_eq!(msg, "boom");
        }
        other => panic!("expected JsExecution, got {other:?}"),
    }
}

// ===========================================================================
// Issue 1: POST body accepts JSON object value (番薯小说 case)
// ===========================================================================

#[test]
fn parse_url_with_json_object_body_serializes_to_string() {
    // 番薯小说-style: body is a JSON object, not a string.
    let raw = r#"https://example.test/search, {"method":"POST","body":{"keyword":"斗破苍穹","page_index":1,"page_size":20}}"#;
    let result = UrlDslParser::parse(raw).expect("URL with JSON object body parses");
    assert_eq!(result.url, "https://example.test/search");
    assert_eq!(result.options.method, "POST");
    assert_eq!(
        result.options.body.as_deref(),
        Some(r#"{"keyword":"斗破苍穹","page_index":1,"page_size":20}"#)
    );
}

#[test]
fn build_request_post_with_json_object_body_serializes_to_string() {
    let ctx = AnalyzeUrlContext::for_search("k", 1);
    let raw = r#"https://example.test/search, {"method":"POST","body":{"keyword":"斗破苍穹","page_index":1}}"#;
    let request = AnalyzeUrl::build_request(raw, &ctx, "https://example.test", &Default::default())
        .expect("POST with JSON object body builds");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.body.as_deref(),
        Some(r#"{"keyword":"斗破苍穹","page_index":1}"#)
    );
}

#[test]
fn parse_url_with_string_body_still_works() {
    // Regression guard: existing string-body behavior unchanged.
    let raw = r#"https://example.test/search, {"method":"POST","body":"key=value&page=1"}"#;
    let result = UrlDslParser::parse(raw).expect("URL with string body parses");
    assert_eq!(result.options.body.as_deref(), Some("key=value&page=1"));
}

// ===========================================================================
// Issue 2: DSL JSON + JS tail concatenation (图书迷子 case)
// ===========================================================================

#[test]
fn parse_url_with_json_options_and_at_js_suffix_strips_js_tail() {
    // 图书迷子-style: URL+JSON DSL followed by @js: tail. Without the fix,
    // the trailing @js: content makes the JSON options invalid.
    let raw = "https://example.test/search,{\"method\":\"POST\",\"body\":\"q=test\"}@js:java.webView(null,baseUrl,null)\nurl=String(result)";
    let result = UrlDslParser::parse(raw).expect("URL+JSON+@js: parses");
    assert_eq!(result.url, "https://example.test/search");
    assert_eq!(result.options.method, "POST");
    assert_eq!(result.options.body.as_deref(), Some("q=test"));
    assert!(result.has_js_expression);
    let js = result.js_expression.as_deref().expect("js expression set");
    assert!(js.contains("java.webView"), "js expression was: {js}");
    assert!(js.contains("url=String(result)"), "js expression was: {js}");
}

#[test]
fn parse_url_with_json_options_and_js_tag_suffix_strips_js_tail() {
    let raw = r#"https://example.test/search,{"method":"POST","body":"q=test"}<js>result.replace(' ', '+')</js>"#;
    let result = UrlDslParser::parse(raw).expect("URL+JSON+<js> parses");
    assert_eq!(result.url, "https://example.test/search");
    assert_eq!(result.options.method, "POST");
    assert_eq!(result.options.body.as_deref(), Some("q=test"));
    assert!(result.has_js_expression);
    assert_eq!(
        result.js_expression.as_deref(),
        Some("result.replace(' ', '+')")
    );
}

#[test]
fn build_request_with_js_handles_dsl_json_with_at_js_tail() {
    // End-to-end: build_request_with_js should parse the JSON options,
    // evaluate the JS tail, and use the JS result as the final URL.
    let ctx = AnalyzeUrlContext::for_search("斗破苍穹", 1);
    let raw = "https://placeholder.test,{\"method\":\"POST\",\"body\":\"q={{key}}\"}@js:buildUrl(key)";
    let request = AnalyzeUrl::build_request_with_js(
        raw,
        &ctx,
        "https://example.test",
        &Default::default(),
        |expr, context| {
            assert_eq!(expr, "buildUrl(key)");
            assert_eq!(context["key"], "斗破苍穹");
            Ok("https://built.example.test/api".to_string())
        },
    )
    .expect("JS tail URL builds");
    assert_eq!(request.url, "https://built.example.test/api");
    assert_eq!(request.method, "GET");
}

#[test]
fn parse_url_with_at_js_inside_body_string_is_not_stripped() {
    // Regression guard: @js: inside a quoted body string must NOT be treated
    // as the JS tail separator.
    let raw = r#"https://example.test/search,{"method":"POST","body":"q=test@js:foo"}"#;
    let result = UrlDslParser::parse(raw).expect("URL with @js: inside body parses");
    assert_eq!(result.url, "https://example.test/search");
    assert_eq!(result.options.body.as_deref(), Some("q=test@js:foo"));
    assert!(!result.has_js_expression);
}

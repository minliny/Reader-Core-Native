//! Integration tests for the LegadoRule multi-engine prefix dispatcher.
//!
//! These tests exercise `RuleEngine::execute_legado_rule`, which mirrors
//! Legado `AnalyzeRule.SourceRule.init` (lines 545-578) by routing a raw rule
//! string to CSS / XPath / JSONPath based on its prefix, before any `##`
//! suffix / `@put` / `@get` / `{{}}` / `<js>` handling (covered in later
//! tasks).

use reader_rule::{NoopVariableScope, RuleEngine, RuleVariableScope};

const HTML: &str = r#"<div class="list"><a href="/a">Alpha</a><a href="/b">Beta</a></div>"#;
const JSON: &str = r#"{"books":[{"name":"X","url":"/x"},{"name":"Y","url":"/y"}]}"#;

#[test]
fn dispatches_css_default_rule() {
    let out = RuleEngine::new()
        .execute_legado_rule(HTML, "div.list&&a@text", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["Alpha", "Beta"]);
}

#[test]
fn dispatches_at_css_prefix() {
    let out = RuleEngine::new()
        .execute_legado_rule(HTML, "@CSS:div.list&&a@text", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["Alpha", "Beta"]);
}

#[test]
fn dispatches_at_at_css_prefix() {
    let out = RuleEngine::new()
        .execute_legado_rule(HTML, "@@div.list&&a@text", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["Alpha", "Beta"]);
}

#[test]
fn dispatches_json_dollar_prefix() {
    let out = RuleEngine::new()
        .execute_legado_rule(JSON, "$.books[*].name", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["X", "Y"]);
}

#[test]
fn dispatches_at_json_prefix() {
    let out = RuleEngine::new()
        .execute_legado_rule(JSON, "@Json:$.books[*].url", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["/x", "/y"]);
}

#[test]
fn dispatches_xpath_slash_prefix() {
    let xml = r#"<root><item>One</item><item>Two</item></root>"#;
    let out = RuleEngine::new()
        .execute_legado_rule(xml, "//item/text()", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["One", "Two"]);
}

#[test]
fn dispatches_at_xpath_prefix() {
    let xml = r#"<root><item>One</item></root>"#;
    let out = RuleEngine::new()
        .execute_legado_rule(xml, "@XPath://item/text()", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(out.values(), &["One"]);
}

#[test]
fn xpath_parses_real_html_with_void_elements_and_unclosed_tags() {
    // rb-xpath-strict-xml-parser: 真实书源响应是 HTML,含 void 元素 (<img>/<br>)
    // 和未闭合标签 (<p>),严格 XML 解析 (sxd_document::parser::parse) 会全部失败。
    // 对齐 Legado AnalyzeByXPath.strToJXDocument: 非 `<?xml` 输入走 HTML 容错解析。
    let html = r#"<!doctype html>
<html><body>
  <div class="book">
    <img src="/cover/1.jpg">
    <h3>Title One</h3>
    <p>para one<br>line two
    <a href="/book/1">link1</a>
  </div>
  <div class="book">
    <img src="/cover/2.jpg">
    <h3>Title Two</h3>
    <a href="/book/2">link2</a>
  </div>
</body></html>"#;

    let titles = RuleEngine::new()
        .execute_legado_rule(
            html,
            "@XPath://div[@class='book']/h3/text()",
            &mut NoopVariableScope,
            None,
        )
        .unwrap();
    assert_eq!(titles.values(), &["Title One", "Title Two"]);

    // void 元素 <img> 的属性也能被 XPath 访问
    let covers = RuleEngine::new()
        .execute_legado_rule(html, "@XPath://img/@src", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(covers.values(), &["/cover/1.jpg", "/cover/2.jpg"]);

    // 未闭合的 <p> 不会让解析失败,XPath 仍能取到 <a> 链接
    let links = RuleEngine::new()
        .execute_legado_rule(html, "@XPath://a/@href", &mut NoopVariableScope, None)
        .unwrap();
    assert_eq!(links.values(), &["/book/1", "/book/2"]);
}

#[test]
fn empty_rule_yields_empty_output() {
    let out = RuleEngine::new()
        .execute_legado_rule(HTML, "   ", &mut NoopVariableScope, None)
        .unwrap();
    assert!(out.is_empty());
}

#[test]
fn noop_variable_scope_stores_nothing() {
    let mut scope = NoopVariableScope;
    scope.put("k".to_string(), "v".to_string());
    assert_eq!(scope.get("k"), None);
    assert!(scope.entries().is_empty());
    // suppress unused warning on the trait import
    let _: &dyn RuleVariableScope = &scope;
}

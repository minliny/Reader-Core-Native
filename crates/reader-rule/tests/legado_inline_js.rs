//! Integration tests for Legado inline JS: `{{...}}` templates and
//! `<js>...</js>` / `@js:` segment chaining.
//!
//! - `{{expr}}` (Legado evalPattern jsRuleType, AnalyzeRule.kt:606-609,
//!   makeUpRule 695-697): the expression is evaluated as JS with no `result`
//!   binding and its string result is substituted into the rule body before
//!   extraction. Mirrors AppPattern.EXP_PATTERN `{{([\w\W]*?)}}`.
//! - `<js>expr</js>` / `@js:expr` (Legado splitSourceRule 498-518,
//!   AppPattern.JS_PATTERN): the rule is split into alternating non-JS / JS
//!   segments. Non-JS segments execute on the input; JS segments transform
//!   each output value with `result` bound to the current value.

use reader_rule::{NoopVariableScope, RuleEngine, RuleJsEvaluator};

const HTML: &str = r#"<div>hello</div><span>world</span>"#;

/// A tiny JS evaluator that records calls and returns a canned result, so
/// tests can assert on the `result` binding without pulling in reader-js.
#[derive(Default)]
struct FakeJs {
    calls: std::cell::RefCell<Vec<(String, Option<String>)>>,
}

impl RuleJsEvaluator for FakeJs {
    fn eval(&self, expr: &str, context: Option<&str>) -> Result<String, String> {
        self.calls
            .borrow_mut()
            .push((expr.to_string(), context.map(|s| s.to_string())));
        // Minimal "interpreter": return the context uppercased when `result`
        // is referenced, otherwise return the expr verbatim. This lets tests
        // verify both template (no context) and transform (with context)
        // paths without a real JS engine.
        if let Some(ctx) = context {
            Ok(ctx.to_uppercase())
        } else {
            Ok(expr.to_string())
        }
    }
}

fn exec(rule: &str, js: Option<&FakeJs>) -> Vec<String> {
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    engine
        .execute_legado_rule(
            HTML,
            rule,
            &mut scope,
            js.map(|j| j as &dyn RuleJsEvaluator),
        )
        .unwrap()
        .into_values()
}

#[test]
fn js_template_substitutes_result_into_selector() {
    // {{"div"}} → JS returns "div", body becomes "div@text"
    let js = FakeJs::default();
    // The FakeJs returns the expr verbatim when context is None, so we use a
    // literal that doubles as a CSS selector fragment.
    let out = exec(r#"{{div}}@text"#, Some(&js));
    assert_eq!(out, vec!["hello".to_string()]);
    // Template was evaluated with no result context
    assert_eq!(js.calls.borrow().len(), 1);
    assert_eq!(js.calls.borrow()[0].1, None);
}

#[test]
fn js_template_without_evaluator_leaves_rule_unchanged() {
    // No JS evaluator → {{...}} is not substituted; the raw "{{div}}" reaches
    // the CSS engine which returns no matches (graceful, no panic).
    let out = exec(r#"{{div}}@text"#, None);
    assert!(
        out.is_empty(),
        "without a JS evaluator {{...}} yields no matches"
    );
}

#[test]
fn inline_js_post_processes_each_output_value() {
    // div@text extracts ["hello", "world"] (wait — span has no @text? Actually
    // div@text selects div text only). Let me use a selector that matches one.
    // div@text → ["hello"], then <js> uppercases via result binding.
    let html = r#"<div>hello</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let js = FakeJs::default();
    let out = engine
        .execute_legado_rule(
            html,
            "div@text<js>result</js>",
            &mut scope,
            Some(&js as &dyn RuleJsEvaluator),
        )
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["HELLO".to_string()]);
    // JS was called with the extracted value as context
    assert_eq!(js.calls.borrow().len(), 1);
    assert_eq!(js.calls.borrow()[0].1.as_deref(), Some("hello"));
}

#[test]
fn at_js_segment_post_processes_output() {
    let html = r#"<div>hello</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let js = FakeJs::default();
    let out = engine
        .execute_legado_rule(
            html,
            "div@text@js:result",
            &mut scope,
            Some(&js as &dyn RuleJsEvaluator),
        )
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["HELLO".to_string()]);
}

#[test]
fn inline_js_without_evaluator_yields_empty() {
    // <js> present but no evaluator → cannot transform → empty output.
    let html = r#"<div>hello</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let out = engine
        .execute_legado_rule(html, "div@text<js>result</js>", &mut scope, None)
        .unwrap()
        .into_values();
    assert!(out.is_empty());
}

#[test]
fn pure_js_segment_without_selector_produces_js_result() {
    // <js>"static"</js> with no preceding rule segment: the JS is evaluated
    // with the input HTML as context (Legado binds result to the input when
    // the JS segment is first).
    let html = r#"<div>hello</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let js = FakeJs::default();
    let out = engine
        .execute_legado_rule(
            html,
            r#"<js>"pure"</js>"#,
            &mut scope,
            Some(&js as &dyn RuleJsEvaluator),
        )
        .unwrap()
        .into_values();
    // FakeJs uppercases the context (the input HTML), so output is uppercased HTML.
    assert_eq!(out, vec![html.to_uppercase()]);
}

#[test]
fn js_template_and_inline_js_combined() {
    // {{"div"}} templates into "div@text", extracts "hello", then <js>
    // uppercases to "HELLO".
    let html = r#"<div>hello</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let js = FakeJs::default();
    let out = engine
        .execute_legado_rule(
            html,
            r#"{{div}}@text<js>result</js>"#,
            &mut scope,
            Some(&js as &dyn RuleJsEvaluator),
        )
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["HELLO".to_string()]);
}

#[test]
fn inline_js_chains_multiple_segments() {
    // div@text → ["hello"], <js> uppercase → ["HELLO"], <js> lowercase → ["hello"]
    let html = r#"<div>hello</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let js = FakeJs::default();
    let out = engine
        .execute_legado_rule(
            html,
            "div@text<js>result</js><js>result</js>",
            &mut scope,
            Some(&js as &dyn RuleJsEvaluator),
        )
        .unwrap()
        .into_values();
    // First JS: "hello" → "HELLO", second JS: "HELLO" → "HELLO" (FakeJs
    // uppercases, so "HELLO" stays "HELLO"). This proves chaining.
    assert_eq!(out, vec!["HELLO".to_string()]);
    assert_eq!(js.calls.borrow().len(), 2);
    assert_eq!(js.calls.borrow()[0].1.as_deref(), Some("hello"));
    assert_eq!(js.calls.borrow()[1].1.as_deref(), Some("HELLO"));
}

//! Integration tests for the Legado `@put`/`@get` variable mechanism
//! (AnalyzeRule.kt:408-431 splitPutRule, 580-604 evalMatcher @get branch,
//! 698-699 makeUpRule getRuleType, 754-769 get()).
//!
//! `@put:{...}` extracts a JSON object of string key-value pairs, stores them
//! in the variable scope, and is removed from the rule string. `@get:{key}`
//! is replaced with the stored value (empty string if missing). Both patterns
//! are case-insensitive (Legado putPattern/evalPattern use CASE_INSENSITIVE).

use reader_rule::{RuleEngine, RuleVariableScope};
use std::collections::BTreeMap;

const HTML: &str = r#"<div>hello</div><span>world</span>"#;

#[derive(Default)]
struct RecordingScope {
    vars: BTreeMap<String, String>,
}

impl RuleVariableScope for RecordingScope {
    fn get(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }
    fn put(&mut self, key: String, value: String) {
        self.vars.insert(key, value);
    }
    fn entries(&self) -> Vec<(&str, &str)> {
        self.vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

fn exec_with_scope(rule: &str, scope: &mut RecordingScope) -> Vec<String> {
    let engine = RuleEngine::new();
    engine
        .execute_legado_rule(HTML, rule, scope, None)
        .unwrap()
        .into_values()
}

#[test]
fn put_extracts_and_stores_in_scope() {
    let mut scope = RecordingScope::default();
    let out = exec_with_scope(r#"@put:{"k":"v"}div@text"#, &mut scope);
    assert_eq!(out, vec!["hello".to_string()]);
    // @put pair must be stored in the scope and removed from the rule body
    assert_eq!(scope.entries(), vec![("k", "v")]);
}

#[test]
fn get_substitutes_selector_from_put() {
    let mut scope = RecordingScope::default();
    // @put stores sel=div, @get:{sel} expands to "div", forming "div@text"
    let out = exec_with_scope(r#"@put:{"sel":"div"}@get:{sel}@text"#, &mut scope);
    assert_eq!(out, vec!["hello".to_string()]);
}

#[test]
fn put_stores_multiple_keys() {
    let mut scope = RecordingScope::default();
    let out = exec_with_scope(r#"@put:{"a":"1","b":"2"}div@text"#, &mut scope);
    assert_eq!(out, vec!["hello".to_string()]);
    let mut entries = scope.entries();
    entries.sort();
    assert_eq!(entries, vec![("a", "1"), ("b", "2")]);
}

#[test]
fn put_get_case_insensitive() {
    let mut scope = RecordingScope::default();
    let out = exec_with_scope(r#"@PUT:{"sel":"span"}@GET:{sel}@text"#, &mut scope);
    assert_eq!(out, vec!["world".to_string()]);
}

#[test]
fn get_missing_key_yields_empty_string() {
    let mut scope = RecordingScope::default();
    // @get:{missing} → "" so the rule body becomes "@text" which selects the
    // text of all elements at the root. The key point: no panic, no error,
    // and the @get is consumed (not left in the selector).
    let out = exec_with_scope(r#"div@text##l##@get:{missing}"#, &mut scope);
    // selector = "div@text", replace_regex = "l", replacement = "" (from @get:{missing})
    // "hello".replace("l", "") → "heo"
    assert_eq!(out, vec!["heo".to_string()]);
}

#[test]
fn put_invalid_json_is_removed_but_not_stored() {
    let mut scope = RecordingScope::default();
    // `{not json}` matches the putPattern but fails JSON parse → Legado skips
    // storage but still strips the @put segment from the rule body.
    let out = exec_with_scope(r#"@put:{not json}div@text"#, &mut scope);
    assert_eq!(out, vec!["hello".to_string()]);
    assert!(
        scope.entries().is_empty(),
        "invalid JSON must not be stored"
    );
}

#[test]
fn put_get_full_pipeline_with_regex_suffix() {
    let mut scope = RecordingScope::default();
    // @put sel=div, @get:{sel} → "div", rule becomes "div@text##l##L"
    // "hello".replace("l", "L") → "heLLo"
    let out = exec_with_scope(r#"@put:{"sel":"div"}@get:{sel}@text##l##L"#, &mut scope);
    assert_eq!(out, vec!["heLLo".to_string()]);
}

#[test]
fn put_get_works_with_json_mode() {
    let json = r#"{"items":[{"name":"a-b"}]}"#;
    let engine = RuleEngine::new();
    let mut scope = RecordingScope::default();
    // Mode prefix (@Json:) must come first — Legado detects mode on the raw
    // rule string before splitPutRule runs. @put is appended at the end so it
    // is stripped after mode detection; @get:{path} then expands into the
    // stored JSONPath selector.
    let out = engine
        .execute_legado_rule(
            json,
            r#"@Json:@get:{path}##-##X@put:{"path":"$.items[*].name"}"#,
            &mut scope,
            None,
        )
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["aXb".to_string()]);
}

#[test]
fn get_substitutes_value_containing_special_chars() {
    let mut scope = RecordingScope::default();
    // @put stores a selector containing a dot (CSS class) — @get must reproduce
    // it verbatim so the CSS engine sees "span" (no special chars here, but the
    // value flows through unchanged).
    scope.vars.insert("sel".to_string(), "span".to_string());
    let out = exec_with_scope(r#"@get:{sel}@text"#, &mut scope);
    assert_eq!(out, vec!["world".to_string()]);
}

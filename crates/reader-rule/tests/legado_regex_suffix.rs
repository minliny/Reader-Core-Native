//! Integration tests for the Legado `##regex##replacement` suffix parser
//! (AnalyzeRule.kt:708-718, replaceRegex 436-460).
//!
//! Legado splits the rule body on `##` *after* mode detection:
//!   `rule##regex##replacement##<flag>`
//! - 1 part  → no regex suffix
//! - 2 parts → `replaceRegex` set, `replacement = ""`, replace-all
//! - 3 parts → `replaceRegex` + `replacement`, replace-all
//! - 4+ parts→ `replaceRegex` + `replacement`, `replaceFirst = true`
//!
//! `replaceFirst` semantics (Legado line 441-452): find the first regex match;
//! if found, return the `replacement` template expanded against that match's
//! capture groups (the rest of the string is discarded); if no match, return
//! the empty string. If the regex fails to compile, return `replacement`
//! verbatim.
//!
//! replace-all semantics (Legado line 453-459): if the regex compiles, replace
//! every match with `replacement`; if it fails to compile, fall back to literal
//! `str.replace(regexStr, replacement)`.

use reader_rule::{NoopVariableScope, RuleEngine};

const HTML: &str = r#"<div>a-b-c</div><div>x-y-z</div>"#;

fn exec_html(rule: &str) -> Vec<String> {
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    engine
        .execute_legado_rule(HTML, rule, &mut scope, None)
        .unwrap()
        .into_values()
}

#[test]
fn regex_suffix_strips_all_matches_when_no_replacement_given() {
    // `##-` with no replacement field → replacement = "" → remove all "-"
    let out = exec_html("div@text##-");
    assert_eq!(out, vec!["abc".to_string(), "xyz".to_string()]);
}

#[test]
fn regex_suffix_replaces_all_matches_with_explicit_replacement() {
    // `##-##X` → replace all "-" with "X"
    let out = exec_html("div@text##-##X");
    assert_eq!(out, vec!["aXbXc".to_string(), "xXyXz".to_string()]);
}

#[test]
fn regex_suffix_replace_first_returns_replacement_on_match() {
    // `##-##X##1` → replaceFirst: first match replaced with "X", rest discarded
    let out = exec_html("div@text##-##X##1");
    assert_eq!(out, vec!["X".to_string(), "X".to_string()]);
}

#[test]
fn regex_suffix_replace_first_no_match_returns_empty() {
    // `##Z##X##1` → no match → ""
    let out = exec_html("div@text##Z##X##1");
    assert_eq!(out, vec!["".to_string(), "".to_string()]);
}

#[test]
fn regex_suffix_no_suffix_passes_through_unchanged() {
    let out = exec_html("div@text");
    assert_eq!(out, vec!["a-b-c".to_string(), "x-y-z".to_string()]);
}

#[test]
fn regex_suffix_invalid_regex_replace_all_falls_back_to_literal() {
    // "(" is an invalid regex → Legado falls back to literal str.replace
    let html = r#"<div>a(b(c</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let out = engine
        .execute_legado_rule(html, "div@text##(##X", &mut scope, None)
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["aXbXc".to_string()]);
}

#[test]
fn regex_suffix_invalid_regex_replace_first_returns_replacement_verbatim() {
    // replaceFirst + invalid regex → Legado returns `replacement` unchanged
    let html = r#"<div>abc</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let out = engine
        .execute_legado_rule(html, "div@text##(##X##1", &mut scope, None)
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["X".to_string()]);
}

#[test]
fn regex_suffix_replace_first_expands_capture_groups() {
    // `##(\w+)\s+(\w+)##$2 $1##1` → first match "hello world" → "world hello"
    let html = r#"<div>hello world</div>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let out = engine
        .execute_legado_rule(html, r"div@text##(\w+)\s+(\w+)##$2 $1##1", &mut scope, None)
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["world hello".to_string()]);
}

#[test]
fn regex_suffix_applies_to_json_mode_output() {
    // `@Json:$.name##-##X` → JSONPath extraction then regex replace
    let json = r#"{"name":"a-b"}"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let out = engine
        .execute_legado_rule(json, "@Json:$.name##-##X", &mut scope, None)
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["aXb".to_string()]);
}

#[test]
fn regex_suffix_applies_to_xpath_mode_output() {
    // `@XPath://div/text()##-##X` → XPath extraction then regex replace.
    // sxd-xpath requires well-formed XML, so wrap in a root element.
    let xml = r#"<root><div>a-b-c</div><div>x-y-z</div></root>"#;
    let engine = RuleEngine::new();
    let mut scope = NoopVariableScope;
    let out = engine
        .execute_legado_rule(xml, "@XPath://div/text()##-##X", &mut scope, None)
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["aXbXc".to_string(), "xXyXz".to_string()]);
}

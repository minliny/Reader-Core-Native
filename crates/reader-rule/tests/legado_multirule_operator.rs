//! Integration tests for the Legado MultiRule combination operators
//! `&&` / `||` / `%%` on CSS / XPath / Regex paths.
//!
//! These tests verify `rb-legado-css-multirule-operator`: the operators are
//! now split at the rule-type dispatcher (`RuleEngine::execute_mode` →
//! `split_legado_combined_rule`), mirroring Legado `RuleAnalyzer.splitRule`
//! (`RuleAnalyzer.kt:165-237`) which every analyzer (`AnalyzeByXPath` /
//! `AnalyzeByJSoup` / `AnalyzeByRegex`) calls regardless of rule type. Before
//! the fix, only the JSONPath evaluator split on these operators; CSS/shorthand
//! paths fed the raw combined selector to `scraper::Selector::parse`, which
//! rejected rules like `class.ser-ret@li||class.j_bookList@li`.
//!
//! Legado combination semantics (AnalyzeByXPath.getElements 52-89):
//! - `||` OR-fallback: branches tried in order; first non-empty result wins.
//! - `&&` AND-merge: every branch executed; results concatenated.
//! - `%%` parallel zip: every branch executed; results interleaved by index.

use reader_rule::{NoopVariableScope, RuleEngine};

const HTML: &str = r#"<div class="list">
    <a href="/a">Alpha</a>
    <a href="/b">Beta</a>
</div>
<div class="empty"></div>
<ul class="items">
    <li>One</li>
    <li>Two</li>
    <li>Three</li>
</ul>"#;

const XML: &str = r#"<root>
    <item>One</item>
    <item>Two</item>
    <item>Three</item>
</root>"#;

fn run(rule: &str) -> Vec<String> {
    RuleEngine::new()
        .execute_legado_rule(HTML, rule, &mut NoopVariableScope, None)
        .unwrap()
        .into_values()
}

fn run_xml(rule: &str) -> Vec<String> {
    RuleEngine::new()
        .execute_legado_rule(XML, rule, &mut NoopVariableScope, None)
        .unwrap()
        .into_values()
}

// ---------------------------------------------------------------------------
// CSS path — `||` OR-fallback
// ---------------------------------------------------------------------------

#[test]
fn css_or_fallback_returns_first_non_empty_branch() {
    // First branch `div.missing@text` matches nothing; second branch
    // `div.list a@text` matches. OR-fallback returns the second branch.
    let out = run("div.missing@text||div.list a@text");
    assert_eq!(out, vec!["Alpha".to_string(), "Beta".to_string()]);
}

#[test]
fn css_or_fallback_returns_first_branch_when_non_empty() {
    // Both branches match; OR-fallback returns the FIRST non-empty branch.
    let out = run("div.list a@text||ul.items li@text");
    assert_eq!(out, vec!["Alpha".to_string(), "Beta".to_string()]);
}

#[test]
fn css_or_fallback_yodu_style_class_shorthand() {
    // Mirrors yodu.org ruleSearch.bookList = "class.ser-ret@li||class.j_bookList@li":
    // first branch matches, second branch is never tried.
    let html = r#"<ul class="ser-ret"><li>Book A</li><li>Book B</li></ul>"#;
    let out = RuleEngine::new()
        .execute_legado_rule(
            html,
            "class.ser-ret@li@text||class.j_bookList@li@text",
            &mut NoopVariableScope,
            None,
        )
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["Book A".to_string(), "Book B".to_string()]);
}

#[test]
fn css_or_fallback_all_branches_empty_returns_empty() {
    let out = run("div.missing@text||span.nope@text");
    assert!(out.is_empty());
}

// ---------------------------------------------------------------------------
// CSS path — `&&` AND-merge
// ---------------------------------------------------------------------------

#[test]
fn css_and_merge_concatenates_all_branch_results() {
    // Each branch runs independently; results concatenated in order.
    let out = run("div.list a@text&&ul.items li@text");
    assert_eq!(
        out,
        vec![
            "Alpha".to_string(),
            "Beta".to_string(),
            "One".to_string(),
            "Two".to_string(),
            "Three".to_string(),
        ]
    );
}

#[test]
fn css_and_merge_yodu_style_kind_field() {
    // Mirrors yodu.org ruleSearch.kind three-branch AND-merge pattern
    // (`class.X.0@text&&tag.span.2@text&&class.Y.1@text`). Uses single-class
    // shorthand to avoid the separate multi-class translation gap.
    let html = r#"<div>
        <span class="genre">网络玄幻</span>
        <span class="author">天蚕土豆</span>
        <span class="status">连载</span>
        <span class="status">完本</span>
    </div>"#;
    let out = RuleEngine::new()
        .execute_legado_rule(
            html,
            "class.genre.0@text&&tag.span.1@text&&class.status.1@text",
            &mut NoopVariableScope,
            None,
        )
        .unwrap()
        .into_values();
    assert_eq!(
        out,
        vec![
            "网络玄幻".to_string(),
            "天蚕土豆".to_string(),
            "完本".to_string(),
        ]
    );
}

#[test]
fn css_and_merge_skips_empty_branches() {
    // Middle branch matches nothing; remaining branches still merge.
    let out = run("div.list a@text&&div.missing@text&&ul.items li@text");
    assert_eq!(
        out,
        vec![
            "Alpha".to_string(),
            "Beta".to_string(),
            "One".to_string(),
            "Two".to_string(),
            "Three".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// CSS path — `%%` parallel zip
// ---------------------------------------------------------------------------

#[test]
fn css_parallel_zip_interleaves_branch_results_by_index() {
    // Branch 1: [Alpha, Beta]; Branch 2: [One, Two, Three].
    // Zip: [Alpha, One, Beta, Two] (Legado stops at first branch's length).
    let out = run("div.list a@text%%ul.items li@text");
    assert_eq!(
        out,
        vec![
            "Alpha".to_string(),
            "One".to_string(),
            "Beta".to_string(),
            "Two".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// First-operator-wins semantics (Legado RuleAnalyzer.elementsType)
// ---------------------------------------------------------------------------

#[test]
fn first_operator_wins_mixing_operators_uses_first_found() {
    // `a@text||ul.items li@text&&div.list a@text`: first operator is `||`,
    // so elementsType = "||". The rule splits by `||` into:
    //   branch 1 = "a@text"
    //   branch 2 = "ul.items li@text&&div.list a@text"  (&& is literal here,
    //     not a split point, because elementsType is already fixed to ||)
    // Branch 1 matches (Alpha, Beta) → OR-fallback returns it immediately.
    let out = run("a@text||ul.items li@text&&div.list a@text");
    assert_eq!(out, vec!["Alpha".to_string(), "Beta".to_string()]);
}

// ---------------------------------------------------------------------------
// Single rule (no operator) — must not regress
// ---------------------------------------------------------------------------

#[test]
fn single_rule_without_operator_unchanged() {
    let out = run("div.list a@text");
    assert_eq!(out, vec!["Alpha".to_string(), "Beta".to_string()]);
}

// ---------------------------------------------------------------------------
// XPath path — `||` / `&&` / `%%`
// ---------------------------------------------------------------------------

#[test]
fn xpath_or_fallback_returns_first_non_empty_branch() {
    // First XPath branch matches nothing; second matches.
    let out = run_xml("//missing/text()||//item/text()");
    assert_eq!(
        out,
        vec!["One".to_string(), "Two".to_string(), "Three".to_string(),]
    );
}

#[test]
fn xpath_and_merge_concatenates_all_branch_results() {
    let out = run_xml("//item[1]/text()&&//item[3]/text()");
    assert_eq!(out, vec!["One".to_string(), "Three".to_string()]);
}

#[test]
fn xpath_parallel_zip_interleaves_branch_results_by_index() {
    // Branch 1: [One, Two, Three]; Branch 2: [One, Two, Three] (same set).
    // Zip: [One, One, Two, Two, Three, Three].
    let out = run_xml("//item/text()%%//item/text()");
    assert_eq!(
        out,
        vec![
            "One".to_string(),
            "One".to_string(),
            "Two".to_string(),
            "Two".to_string(),
            "Three".to_string(),
            "Three".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// Quote/bracket awareness — operators inside quotes/brackets are NOT split
// ---------------------------------------------------------------------------

#[test]
fn operators_inside_css_attribute_selector_are_not_split() {
    // `a[href="x&&y"]` — the `&&` is inside a quoted attribute value, so it
    // must NOT be treated as a combination operator.
    let html = r#"<a href="x&&y">Link</a>"#;
    let out = RuleEngine::new()
        .execute_legado_rule(html, "a[href=\"x&&y\"]@text", &mut NoopVariableScope, None)
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["Link".to_string()]);
}

#[test]
fn operators_inside_xpath_predicate_are_not_split() {
    // `//item[contains(@href, "x||y")]` — the `||` is inside a predicate.
    let xml = r#"<root><item href="x||y">Match</item><item href="z">No</item></root>"#;
    let out = RuleEngine::new()
        .execute_legado_rule(
            xml,
            "//item[contains(@href, \"x||y\")]/text()",
            &mut NoopVariableScope,
            None,
        )
        .unwrap()
        .into_values();
    assert_eq!(out, vec!["Match".to_string()]);
}

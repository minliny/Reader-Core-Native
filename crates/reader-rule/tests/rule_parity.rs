//! Parity tests for rule execution boundaries: fallback, empty results,
//! error expressions, duplicate results, encoding/escaping, and the
//! JSONPath/CSS/JS expression surface that reader-rule owns.

use reader_rule::{CaptureGroup, RuleEngine, RuleError, RuleStep};

const HTML: &str = include_str!("fixtures/catalog.html");
const JSON: &str = include_str!("fixtures/catalog.json");
const XML: &str = include_str!("fixtures/catalog.xml");

// ---------------------------------------------------------------------------
// Fallback
// ---------------------------------------------------------------------------

#[test]
fn fallback_step_seeds_values_when_chain_results_are_empty() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".does-not-exist"),
                RuleStep::fallback(["default-1", "default-2"]),
                RuleStep::regex_replace(r"^", "id:"),
            ],
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["id:default-1".to_string(), "id:default-2".to_string()]
    );
}

#[test]
fn fallback_step_passes_through_non_empty_input_unchanged() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".book-title"),
                RuleStep::fallback(["should-not-appear"]),
            ],
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &[
            "Dune & Foundation".to_string(),
            "Missing Href".to_string()
        ]
    );
}

#[test]
fn fallback_step_alone_returns_configured_values_on_empty_input() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step("", &RuleStep::fallback(["only", "defaults"]))
        .unwrap();

    assert_eq!(
        output.values(),
        &["only".to_string(), "defaults".to_string()]
    );
}

#[test]
fn fallback_step_passes_through_non_empty_single_input() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step("keep-me", &RuleStep::fallback(["ignored"]))
        .unwrap();

    assert_eq!(output.values(), &["keep-me".to_string()]);
}

#[test]
fn fallback_does_not_mask_errors_from_earlier_successful_steps() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".book-title"),
                RuleStep::json_path("$.not_json"),
                RuleStep::fallback(["should-not-reach"]),
            ],
        )
        .unwrap_err();

    match error {
        RuleError::ChainStepFailed { index, source } => {
            assert_eq!(index, 1);
            assert!(matches!(*source, RuleError::JsonParse { .. }));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// JSONPath recursive descent and negative index
// ---------------------------------------------------------------------------

#[test]
fn jsonpath_recursive_descent_collects_fields_at_every_depth() {
    let engine = RuleEngine::new();

    let titles = engine
        .execute_step(JSON, &RuleStep::json_path("$..title"))
        .unwrap();
    assert_eq!(
        titles.values(),
        &[
            "Dune".to_string(),
            "Foundation".to_string(),
            "The Left Hand of Darkness".to_string()
        ]
    );
}

#[test]
fn jsonpath_recursive_descent_wildcard_collects_all_values() {
    let engine = RuleEngine::new();

    let json = r#"{
        "a": { "b": [1, 2] },
        "c": "end"
    }"#;

    let all = engine
        .execute_step(json, &RuleStep::json_path("$..*"))
        .unwrap();

    // Root object's children: a, c
    // a's children: b (array)
    // b's children: 1, 2
    // c is a string — no children
    assert_eq!(
        all.values(),
        &[
            "{\"b\":[1,2]}".to_string(),
            "end".to_string(),
            "[1,2]".to_string(),
            "1".to_string(),
            "2".to_string()
        ]
    );
}

#[test]
fn jsonpath_recursive_descent_with_bracket_index() {
    let engine = RuleEngine::new();

    let firsts = engine
        .execute_step(JSON, &RuleStep::json_path("$..books..[0].title"))
        .unwrap();
    assert_eq!(
        firsts.values(),
        &["Dune".to_string(), "The Left Hand of Darkness".to_string()]
    );
}

#[test]
fn jsonpath_negative_index_resolves_from_end() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b", "c", "d"]}"#;

    let last = engine
        .execute_step(json, &RuleStep::json_path("$.items[-1]"))
        .unwrap();
    assert_eq!(last.values(), &["d".to_string()]);

    let second_last = engine
        .execute_step(json, &RuleStep::json_path("$.items[-2]"))
        .unwrap();
    assert_eq!(second_last.values(), &["c".to_string()]);
}

#[test]
fn jsonpath_negative_index_out_of_bounds_returns_empty() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a"]}"#;

    let missing = engine
        .execute_step(json, &RuleStep::json_path("$.items[-5]"))
        .unwrap();
    assert!(missing.is_empty());
}

#[test]
fn jsonpath_recursive_descent_returns_empty_for_missing_field() {
    let engine = RuleEngine::new();

    let missing = engine
        .execute_step(JSON, &RuleStep::json_path("$..nonexistent"))
        .unwrap();
    assert!(missing.is_empty());
}

// ---------------------------------------------------------------------------
// Empty results
// ---------------------------------------------------------------------------

#[test]
fn css_selector_with_no_matches_returns_empty() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(HTML, &RuleStep::css_text(".nonexistent-class"))
        .unwrap();
    assert!(output.is_empty());
    assert_eq!(output.len(), 0);
}

#[test]
fn css_attr_missing_on_some_elements_skips_them() {
    let engine = RuleEngine::new();

    let hrefs = engine
        .execute_step(HTML, &RuleStep::css_attr("a.book-link", "href"))
        .unwrap();
    // Only the first <a> has href; the second is skipped, not emitted as empty.
    assert_eq!(hrefs.values(), &["/book/1".to_string()]);
}

#[test]
fn jsonpath_root_only_path_returns_entire_document_as_json_string() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(r#"{"a":1}"#, &RuleStep::json_path("$"))
        .unwrap();
    assert_eq!(output.values(), &["{\"a\":1}".to_string()]);
}

#[test]
fn regex_extract_with_no_matches_returns_empty() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(
            "no digits here",
            &RuleStep::regex_capture(r"\d+", CaptureGroup::WholeMatch),
        )
        .unwrap();
    assert!(output.is_empty());
}

#[test]
fn xpath_with_no_matching_nodes_returns_empty() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(
            XML,
            &RuleStep::xpath_with_namespaces(
                "//r:missing/text()",
                [("r", "urn:reader:test")],
            ),
        )
        .unwrap();
    assert!(output.is_empty());
}

// ---------------------------------------------------------------------------
// Error expressions
// ---------------------------------------------------------------------------

#[test]
fn invalid_regex_pattern_produces_regex_syntax_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step(
            "input",
            &RuleStep::regex_capture("[", CaptureGroup::WholeMatch),
        )
        .unwrap_err();

    assert!(matches!(error, RuleError::RegexSyntax { .. }));
}

#[test]
fn invalid_json_input_produces_json_parse_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step("not json", &RuleStep::json_path("$.a"))
        .unwrap_err();

    assert!(matches!(error, RuleError::JsonParse { .. }));
}

#[test]
fn invalid_jsonpath_syntax_produces_jsonpath_syntax_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step(r#"{"a":1}"#, &RuleStep::json_path("bad-path"))
        .unwrap_err();

    assert!(matches!(error, RuleError::JsonPathSyntax { .. }));
}

#[test]
fn invalid_css_selector_produces_css_selector_syntax_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step("<div></div>", &RuleStep::css_text("<<<"))
        .unwrap_err();

    assert!(matches!(error, RuleError::CssSelectorSyntax { .. }));
}

#[test]
fn invalid_xpath_expression_produces_xpath_syntax_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step(XML, &RuleStep::xpath("/////"))
        .unwrap_err();

    assert!(matches!(error, RuleError::XPathSyntax { .. }));
}

#[test]
fn xpath_on_non_xml_input_produces_input_parse_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step("not xml at all", &RuleStep::xpath("//book"))
        .unwrap_err();

    assert!(matches!(error, RuleError::XPathInputParse { .. }));
}

#[test]
fn regex_capture_group_index_out_of_range_produces_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step(
            "title:Dune",
            &RuleStep::regex_capture(r"title:(\w+)", CaptureGroup::index(5)),
        )
        .unwrap_err();

    assert!(matches!(error, RuleError::RegexCaptureGroupMissing { .. }));
}

#[test]
fn regex_capture_named_group_missing_produces_error() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_step(
            "title:Dune",
            &RuleStep::regex_capture(r"title:(\w+)", CaptureGroup::name("missing")),
        )
        .unwrap_err();

    assert!(matches!(error, RuleError::RegexCaptureGroupMissing { .. }));
}

// ---------------------------------------------------------------------------
// Duplicate results
// ---------------------------------------------------------------------------

#[test]
fn css_selector_preserves_duplicate_text_values() {
    let engine = RuleEngine::new();

    let html = r#"
        <ul>
            <li>same</li>
            <li>same</li>
            <li>same</li>
        </ul>
    "#;

    let output = engine.execute_step(html, &RuleStep::css_text("li")).unwrap();
    assert_eq!(
        output.values(),
        &[
            "same".to_string(),
            "same".to_string(),
            "same".to_string()
        ]
    );
}

#[test]
fn jsonpath_wildcard_preserves_duplicate_values() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["dup", "dup", "unique"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[*]"))
        .unwrap();
    assert_eq!(
        output.values(),
        &["dup".to_string(), "dup".to_string(), "unique".to_string()]
    );
}

#[test]
fn regex_extract_preserves_duplicate_matches() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(
            "id:1 id:1 id:2",
            &RuleStep::regex_capture(r"id:(\d+)", CaptureGroup::index(1)),
        )
        .unwrap();
    assert_eq!(
        output.values(),
        &["1".to_string(), "1".to_string(), "2".to_string()]
    );
}

#[test]
fn chained_rules_preserve_duplicates_through_all_steps() {
    let engine = RuleEngine::new();

    let html = r#"
        <ul>
            <li class="item">val-1</li>
            <li class="item">val-1</li>
            <li class="item">val-2</li>
        </ul>
    "#;

    let output = engine
        .execute_chain(
            html,
            &[
                RuleStep::css_text("li.item"),
                RuleStep::regex_capture(r"val-(\d+)", CaptureGroup::index(1)),
            ],
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["1".to_string(), "1".to_string(), "2".to_string()]
    );
}

// ---------------------------------------------------------------------------
// Encoding / escaping
// ---------------------------------------------------------------------------

#[test]
fn css_text_decodes_html_entities() {
    let engine = RuleEngine::new();

    let html = r#"<p>caf&eacute; &amp; &nbsp;tea &lt;water&gt;</p>"#;

    let output = engine.execute_step(html, &RuleStep::css_text("p")).unwrap();
    assert_eq!(output.values(), &["café & tea <water>".to_string()]);
}

#[test]
fn css_attr_preserves_url_encoding() {
    let engine = RuleEngine::new();

    let html = r#"<a href="/search?q=caf%C3%A9&amp;page=1">link</a>"#;

    let output = engine
        .execute_step(html, &RuleStep::css_attr("a", "href"))
        .unwrap();
    // &amp; is decoded to & by the HTML parser, but %C3%A9 stays encoded.
    assert_eq!(
        output.values(),
        &["/search?q=caf%C3%A9&page=1".to_string()]
    );
}

#[test]
fn jsonpath_returns_json_escaped_strings_for_nested_structures() {
    let engine = RuleEngine::new();

    let json = r#"{"nested": {"a": 1, "b": [2, 3]}}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.nested"))
        .unwrap();
    assert_eq!(output.values(), &["{\"a\":1,\"b\":[2,3]}".to_string()]);
}

#[test]
fn jsonpath_handles_unicode_and_escaped_field_names() {
    let engine = RuleEngine::new();

    let json = r#"{"书名": "沙丘", "tags": ["科幻", "经典"]}"#;

    let title = engine
        .execute_step(json, &RuleStep::json_path("$['书名']"))
        .unwrap();
    assert_eq!(title.values(), &["沙丘".to_string()]);

    let tags = engine
        .execute_step(json, &RuleStep::json_path("$.tags[*]"))
        .unwrap();
    assert_eq!(
        tags.values(),
        &["科幻".to_string(), "经典".to_string()]
    );
}

#[test]
fn regex_handles_unicode_patterns_and_input() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(
            "书名:沙une 书名:沙丘",
            &RuleStep::regex_capture(r"书名:(\w+)", CaptureGroup::index(1)),
        )
        .unwrap();
    assert_eq!(
        output.values(),
        &["沙une".to_string(), "沙丘".to_string()]
    );
}

#[test]
fn regex_replace_supports_dollar_sign_escaping() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step(
            "price: 10",
            &RuleStep::regex_replace(r"price:", "$$"),
        )
        .unwrap();
    // $$ is a literal $ in regex replacement.
    assert_eq!(output.values(), &["$ 10".to_string()]);
}

#[test]
fn css_text_normalizes_whitespace_and_nbsp() {
    let engine = RuleEngine::new();

    let html = r#"<p>  line&nbsp;one   <br>  line&nbsp;two  </p>"#;

    let output = engine.execute_step(html, &RuleStep::css_text("p")).unwrap();
    // nbsp is treated as whitespace by normalize_text; multiple spaces collapse.
    assert_eq!(output.values(), &["line one line two".to_string()]);
}

#[test]
fn xpath_handles_attributes_with_special_characters() {
    let engine = RuleEngine::new();

    let xml = r#"<root><item tag="a&amp;b" /></root>"#;

    let output = engine
        .execute_step(xml, &RuleStep::xpath("//item/@tag"))
        .unwrap();
    assert_eq!(output.values(), &["a&b".to_string()]);
}

// ---------------------------------------------------------------------------
// JSONPath bracket and quoted key edge cases
// ---------------------------------------------------------------------------

#[test]
fn jsonpath_supports_chained_bracket_access() {
    let engine = RuleEngine::new();

    let json = r#"{"matrix": [[1, 2], [3, 4]]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.matrix[1][0]"))
        .unwrap();
    assert_eq!(output.values(), &["3".to_string()]);
}

#[test]
fn jsonpath_quoted_field_with_dots() {
    let engine = RuleEngine::new();

    let json = r#"{"a.b": {"c": "deep"}}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$['a.b'].c"))
        .unwrap();
    assert_eq!(output.values(), &["deep".to_string()]);
}

#[test]
fn jsonpath_wildcard_on_object_returns_all_values() {
    let engine = RuleEngine::new();

    let json = r#"{"a": 1, "b": 2, "c": 3}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.*"))
        .unwrap();
    assert_eq!(
        output.values(),
        &["1".to_string(), "2".to_string(), "3".to_string()]
    );
}

#[test]
fn jsonpath_index_beyond_array_length_returns_empty() {
    let engine = RuleEngine::new();

    let json = r#"{"items": [1, 2, 3]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[10]"))
        .unwrap();
    assert!(output.is_empty());
}

#[test]
fn jsonpath_scalar_values_convert_to_text() {
    let engine = RuleEngine::new();

    let json = r#"{"b": true, "n": null, "f": 3.14, "i": 42}"#;

    let b = engine.execute_step(json, &RuleStep::json_path("$.b")).unwrap();
    assert_eq!(b.values(), &["true".to_string()]);

    let n = engine.execute_step(json, &RuleStep::json_path("$.n")).unwrap();
    assert_eq!(n.values(), &["null".to_string()]);

    let f = engine.execute_step(json, &RuleStep::json_path("$.f")).unwrap();
    assert_eq!(f.values(), &["3.14".to_string()]);

    let i = engine.execute_step(json, &RuleStep::json_path("$.i")).unwrap();
    assert_eq!(i.values(), &["42".to_string()]);
}

// ---------------------------------------------------------------------------
// Chain edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_chain_returns_input_as_single_result() {
    let engine = RuleEngine::new();

    let output = engine.execute_chain("raw-input", &[]).unwrap();
    assert_eq!(output.values(), &["raw-input".to_string()]);
}

#[test]
fn chain_with_all_steps_returning_empty_results_in_empty_output() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".missing"),
                RuleStep::regex_capture(r"\d+", CaptureGroup::WholeMatch),
            ],
        )
        .unwrap();
    assert!(output.is_empty());
}

#[test]
fn fallback_in_middle_of_chain_recovers_from_empty_intermediate_step() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".book-title"),
                RuleStep::css_text(".missing-selector"),
                RuleStep::fallback(["recovered"]),
                RuleStep::regex_replace(r"^", "v:"),
            ],
        )
        .unwrap();

    assert_eq!(output.values(), &["v:recovered".to_string()]);
}

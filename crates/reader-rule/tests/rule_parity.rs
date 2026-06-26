//! Parity tests for rule execution boundaries: fallback, empty results,
//! error expressions, duplicate results, encoding/escaping, and the
//! JSONPath/CSS/JS expression surface that reader-rule owns.

use reader_rule::{CaptureGroup, LegadoRuleContext, RuleEngine, RuleError, RuleStep};

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
        &["Dune & Foundation".to_string(), "Missing Href".to_string()]
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
            &RuleStep::xpath_with_namespaces("//r:missing/text()", [("r", "urn:reader:test")]),
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

    let output = engine
        .execute_step(html, &RuleStep::css_text("li"))
        .unwrap();
    assert_eq!(
        output.values(),
        &["same".to_string(), "same".to_string(), "same".to_string()]
    );
}

#[test]
fn css_result_set_pseudos_filter_in_rule_order_like_old_core() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="rank-list">
            <a class="pt-rank-detail" href="/book/1">Rank 1</a>
            <a class="pt-rank-detail" href="/book/2">Rank 2</a>
            <a class="pt-rank-detail" href="/book/3">Rank 3</a>
            <a class="pt-rank-detail" href="/book/4">Rank 4</a>
            <a class="pt-rank-detail" href="/book/5">Rank 5</a>
            <a class="pt-rank-detail" href="/book/6">Rank 6</a>
        </section>
    "#;

    let first_four = engine
        .execute_step(html, &RuleStep::css_text(".pt-rank-detail:lt(4)"))
        .unwrap();
    let drop_last = engine
        .execute_step(html, &RuleStep::css_attr(".pt-rank-detail:lt(-1)", "href"))
        .unwrap();
    let chained_eq = engine
        .execute_step(html, &RuleStep::css_text(".pt-rank-detail:gt(1):eq(2)"))
        .unwrap();

    assert_eq!(
        first_four.values(),
        &[
            "Rank 1".to_string(),
            "Rank 2".to_string(),
            "Rank 3".to_string(),
            "Rank 4".to_string()
        ]
    );
    assert_eq!(
        drop_last.values(),
        &[
            "/book/1".to_string(),
            "/book/2".to_string(),
            "/book/3".to_string(),
            "/book/4".to_string(),
            "/book/5".to_string()
        ]
    );
    assert_eq!(chained_eq.values(), &["Rank 5".to_string()]);
}

#[test]
fn css_comma_selector_applies_result_pseudos_per_group_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <main>
            <p>1</p>
            <p>2</p>
            <p>3</p>
            <div>4</div>
            <div>5</div>
        </main>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_text("p:eq(0),div:eq(1)"))
        .unwrap();

    assert_eq!(output.values(), &["1".to_string(), "5".to_string()]);
}

#[test]
fn css_comma_selector_applies_first_last_per_group_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <main>
            <p>1</p>
            <p>2</p>
            <div>3</div>
            <div>4</div>
        </main>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_text("p:first,div:last"))
        .unwrap();

    assert_eq!(output.values(), &["1".to_string(), "4".to_string()]);
}

#[test]
fn css_selector_result_filter_middle_segment_filters_parent_before_child_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="a">
                <a href="/a0">A0</a>
                <a href="/a1">A1</a>
            </article>
            <article data-id="b"><a href="/b0">B0</a></article>
            <article data-id="c"><a href="/c0">C0</a></article>
        </section>
    "#;

    let second_article_link = engine
        .execute_step(html, &RuleStep::css_attr(".cards>article:eq(1)>a", "href"))
        .unwrap();

    assert_eq!(second_article_link.values(), &["/b0".to_string()]);

    let first_two_article_links = engine
        .execute_step(html, &RuleStep::css_attr(".cards>article:lt(2)>a", "href"))
        .unwrap();

    assert_eq!(
        first_two_article_links.values(),
        &["/a0".to_string(), "/a1".to_string(), "/b0".to_string()]
    );

    let nested_html = r#"
        <section class="cards">
            <article data-id="a">
                <div><a href="/a0x">A0X</a><a href="/a0y">A0Y</a></div>
                <div><a href="/a1">A1</a></div>
            </article>
            <article data-id="b">
                <div><a href="/b0">B0</a></div>
                <div><a href="/b1">B1</a></div>
            </article>
        </section>
    "#;

    let nested_second_div_link = engine
        .execute_step(
            nested_html,
            &RuleStep::css_attr(".cards>article:eq(0)>div:eq(1)>a", "href"),
        )
        .unwrap();

    assert_eq!(nested_second_div_link.values(), &["/a1".to_string()]);
}

#[test]
fn css_selector_contains_filters_by_text() {
    let engine = RuleEngine::new();

    let html = r#"
        <ul>
            <li>Dune Messiah</li>
            <li>Foundation</li>
            <li>Children of Dune</li>
        </ul>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_text("li:contains('Dune')"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune Messiah".to_string(), "Children of Dune".to_string()]
    );
}

#[test]
fn css_selector_contains_is_case_insensitive() {
    let engine = RuleEngine::new();

    let html = r#"
        <ul>
            <li>Dune Messiah</li>
            <li>Foundation</li>
            <li>Children of Dune</li>
        </ul>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_text("li:contains('dune')"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune Messiah".to_string(), "Children of Dune".to_string()]
    );
}

#[test]
fn css_selector_contains_own_ignores_descendant_text() {
    let engine = RuleEngine::new();

    let html = r#"
        <section>
            <p>Dune <span>Appendix</span></p>
            <p><span>Dune</span> Appendix</p>
            <p>Foundation</p>
        </section>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_text("p:containsOwn('Dune')"))
        .unwrap();

    assert_eq!(output.values(), &["Dune Appendix".to_string()]);

    let stacked_html = r#"
        <section class="chapters">
            <a href="/free">Chapter 1</a>
            <a href="/vip">Chapter 2 VIP</a>
            <a href="/about">About</a>
        </section>
    "#;

    let public_chapters = engine
        .execute_step(
            stacked_html,
            &RuleStep::css_attr(
                ".chapters>a:containsOwn(Chapter):not(:containsOwn(VIP))",
                "href",
            ),
        )
        .unwrap();

    assert_eq!(public_chapters.values(), &["/free".to_string()]);
}

#[test]
fn css_own_text_extraction_returns_direct_text_like_legado() {
    let engine = RuleEngine::new();

    let html = r#"
        <section>
            <p><span>nested</span> Direct <b>child</b> Tail</p>
        </section>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_attr("p", "ownText"))
        .unwrap();

    assert_eq!(output.values(), &["Direct Tail".to_string()]);
}

#[test]
fn css_text_nodes_extraction_returns_plain_text_like_old_core() {
    let engine = RuleEngine::new();

    let html = r#"
        <div class="content"><p>Nested chapter text</p></div>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_attr(".content", "textNodes"))
        .unwrap();

    assert_eq!(output.values(), &["Nested chapter text".to_string()]);
}

#[test]
fn css_html_extraction_returns_inner_html_like_old_core() {
    let engine = RuleEngine::new();

    let html = r#"
        <div id="nr1"><blockquote><p>正文</p></blockquote></div>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_attr("#nr1", "html"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["<blockquote><p>正文</p></blockquote>".to_string()]
    );
}

#[test]
fn css_selector_contains_own_middle_segment_filters_parent_before_child_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <dl class="info">
            <dd>AUTHOR：<a href="/alice">Alice</a></dd>
            <dd><span>AUTHOR：</span><a href="/nested">Nested</a></dd>
        </dl>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr("dd:containsOwn(author：)>a", "href"),
        )
        .unwrap();

    assert_eq!(output.values(), &["/alice".to_string()]);

    let nested_html = r#"
        <section class="cards">
            <article data-id="a">
                Featured
                <div>Target <a href="/a">A</a></div>
                <div>Other <a href="/a-other">Other</a></div>
            </article>
            <article data-id="b">
                Plain
                <div>Target <a href="/b">B</a></div>
            </article>
            <article data-id="c">
                Featured
                <div>Other <a href="/c">C</a></div>
            </article>
        </section>
    "#;

    let nested_output = engine
        .execute_step(
            nested_html,
            &RuleStep::css_attr(
                ".cards>article:containsOwn(Featured)>div:containsOwn(Target)>a",
                "href",
            ),
        )
        .unwrap();

    assert_eq!(nested_output.values(), &["/a".to_string()]);
}

#[test]
fn css_selector_not_contains_own_middle_segment_excludes_direct_text_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <dl class="chapters">
            <dd>VIP<a href="/vip">Locked</a></dd>
            <dd><span>VIP</span><a href="/nested-vip">Nested label</a></dd>
            <dd>Free<a href="/free">Free</a></dd>
        </dl>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".chapters>dd:not(:containsOwn(VIP))>a", "href"),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["/nested-vip".to_string(), "/free".to_string()]
    );
}

#[test]
fn css_selector_matches_filters_by_regex_text_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <ul>
            <li>Dune Messiah</li>
            <li>Foundation</li>
            <li>Children of Dune</li>
        </ul>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_text("li:matches((?i)^dune)"))
        .unwrap();

    assert_eq!(output.values(), &["Dune Messiah".to_string()]);
}

#[test]
fn css_selector_not_matches_own_excludes_direct_text_regex_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <div class="chapters">
            <a>正文<span>最新</span></a>
            <a>最新<span>正文</span></a>
            <a>旧章</a>
        </div>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_text(".chapters>a:not(:matchesOwn(最新))"),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["正文最新".to_string(), "旧章".to_string()]
    );
}

#[test]
fn css_selector_contains_whole_text_preserves_whitespace_and_case_like_old_core() {
    let engine = RuleEngine::new();

    let html = "<section class=\"whole\"><p data-id=\"first\"> jsoup\n The <i>HTML</i> Parser</p><p data-id=\"second\">jsoup The <i>HTML</i> Parser</p></section>";

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".whole>p:containsWholeText( jsoup\n The HTML Parser)",
                "data-id",
            ),
        )
        .unwrap();
    let case_sensitive = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".whole>p:containsWholeText( jsoup\n The html Parser)",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(output.values(), &["first".to_string()]);
    assert!(case_sensitive.is_empty());
}

#[test]
fn css_selector_whole_own_text_uses_direct_non_normalized_text_like_old_core() {
    let engine = RuleEngine::new();

    let html = "<section><p data-id=\"own\">Prefix\n<span>child</span>\tSuffix</p><p data-id=\"desc\"><span>Prefix\n\tSuffix</span></p></section>";

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr("p:containsWholeOwnText(Prefix\n\tSuffix)", "data-id"),
        )
        .unwrap();
    let descendant_only = engine
        .execute_step(
            html,
            &RuleStep::css_attr("p:containsWholeOwnText(child)", "data-id"),
        )
        .unwrap();

    assert_eq!(output.values(), &["own".to_string()]);
    assert!(descendant_only.is_empty());
}

#[test]
fn css_selector_matches_whole_text_regex_uses_non_normalized_text_like_old_core() {
    let engine = RuleEngine::new();

    let html = "<section><div data-id=\"ssn\">AA\n  123-45-6789\nZZ</div><div data-id=\"flat\">AA 123-45-6789 ZZ</div><p data-id=\"own\">Line\n<span>child</span>\tTail</p><p data-id=\"desc\"><span>Line\n\tTail</span></p></section>";

    let whole_text = engine
        .execute_step(
            html,
            &RuleStep::css_attr("div:matchesWholeText(AA\\n\\s+123-45-6789\\nZZ)", "data-id"),
        )
        .unwrap();
    let whole_own_text = engine
        .execute_step(
            html,
            &RuleStep::css_attr("p:matchesWholeOwnText(^Line\\n\\tTail$)", "data-id"),
        )
        .unwrap();
    let descendant_only = engine
        .execute_step(
            html,
            &RuleStep::css_attr("p:matchesWholeOwnText(child)", "data-id"),
        )
        .unwrap();

    assert_eq!(whole_text.values(), &["ssn".to_string()]);
    assert_eq!(whole_own_text.values(), &["own".to_string()]);
    assert!(descendant_only.is_empty());
}

#[test]
fn css_selector_whole_text_filters_compose_with_has_and_not_like_old_core() {
    let engine = RuleEngine::new();

    let html = "<section class=\"cards\"><article data-id=\"a\"><p>Line\n  Alpha<span>!</span></p></article><article data-id=\"b\"><p>Line Alpha</p></article></section>";

    let has_output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:has(> p:containsWholeText(Line\n  Alpha!))",
                "data-id",
            ),
        )
        .unwrap();
    let not_output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:not(:containsWholeText(Line\n  Alpha!))",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(has_output.values(), &["a".to_string()]);
    assert_eq!(not_output.values(), &["b".to_string()]);
}

#[test]
fn css_selector_parent_matches_elements_with_children_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <div class="links">
            <a href="/empty"></a>
            <a href="/blank-text"> </a>
            <a href="/child"><span></span></a>
            <a href="/text">Text</a>
            <a href="/second-empty"></a>
        </div>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_attr(".links > a:parent", "href"))
        .unwrap();

    assert_eq!(
        output.values(),
        &[
            "/blank-text".to_string(),
            "/child".to_string(),
            "/text".to_string()
        ]
    );
}

#[test]
fn css_selector_not_parent_matches_elements_without_children_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <div class="links">
            <a href="/empty"></a>
            <a href="/blank-text"> </a>
            <a href="/child"><span></span></a>
            <a href="/text">Text</a>
            <a href="/second-empty"></a>
        </div>
    "#;

    let output = engine
        .execute_step(html, &RuleStep::css_attr(".links > a:not(:parent)", "href"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["/empty".to_string(), "/second-empty".to_string()]
    );
}

#[test]
fn css_selector_parent_middle_segment_filters_parent_before_child_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="empty"></article>
            <article data-id="text"><a href="/text">Text</a></article>
            <article data-id="child"><a href="/child"><span>Child</span></a></article>
        </section>
    "#;

    let child_links = engine
        .execute_step(html, &RuleStep::css_attr(".cards>article:parent>a", "href"))
        .unwrap();

    assert_eq!(
        child_links.values(),
        &["/text".to_string(), "/child".to_string()]
    );

    let not_parent_child_links = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:parent)>a", "href"),
        )
        .unwrap();

    assert!(not_parent_child_links.is_empty());

    let nested_html = r#"
        <section class="cards">
            <article data-id="a">
                <div><a href="/a">A</a></div>
            </article>
            <article data-id="b">
                <div></div>
            </article>
            <article data-id="c"></article>
        </section>
    "#;

    let nested_parent_links = engine
        .execute_step(
            nested_html,
            &RuleStep::css_attr(".cards>article:parent>div:parent>a", "href"),
        )
        .unwrap();

    assert_eq!(nested_parent_links.values(), &["/a".to_string()]);
}

#[test]
fn css_selector_has_parent_matches_child_parent_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="empty"><a href="/empty"></a></article>
            <article data-id="text"><a href="/text">Text</a></article>
            <article data-id="none"><span>None</span></article>
        </section>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards > article:has(> a:parent) > a:parent", "href"),
        )
        .unwrap();

    assert_eq!(output.values(), &["/text".to_string()]);

    let articles = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards > article:has(> a:parent)", "data-id"),
        )
        .unwrap();

    assert_eq!(articles.values(), &["text".to_string()]);
}

#[test]
fn css_selector_has_generic_selector_matches_descendant_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="a"><a href="/a">A</a></article>
            <article data-id="b"><p><a href="/nested">Nested</a></p></article>
            <article data-id="c"><span>None</span></article>
            <article data-id="d"><div><a href="/deep">Deep</a></div></article>
            <article data-id="e"><div><a href="/nested-deep"><span>Deep</span></a></div></article>
        </section>
    "#;

    let descendants = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(a)", "data-id"),
        )
        .unwrap();
    assert_eq!(
        descendants.values(),
        &[
            "a".to_string(),
            "b".to_string(),
            "d".to_string(),
            "e".to_string()
        ]
    );

    let direct_children = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(> a)", "data-id"),
        )
        .unwrap();
    assert_eq!(direct_children.values(), &["a".to_string()]);

    let nested_has = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(div:has(a))", "data-id"),
        )
        .unwrap();
    assert_eq!(nested_has.values(), &["d".to_string(), "e".to_string()]);

    let nested_has_text_filter = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:has(div:has(> a:containsOwn(Deep)))",
                "data-id",
            ),
        )
        .unwrap();
    assert_eq!(nested_has_text_filter.values(), &["d".to_string()]);

    let without_links = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:has(a))", "data-id"),
        )
        .unwrap();
    assert_eq!(without_links.values(), &["c".to_string()]);

    let without_nested_has = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:has(div:has(a)))", "data-id"),
        )
        .unwrap();
    assert_eq!(
        without_nested_has.values(),
        &["a".to_string(), "b".to_string(), "c".to_string()]
    );

    let without_nested_has_text_filter = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:not(:has(div:has(> a:containsOwn(Deep))))",
                "data-id",
            ),
        )
        .unwrap();
    assert_eq!(
        without_nested_has_text_filter.values(),
        &[
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "e".to_string()
        ]
    );
}

#[test]
fn css_selector_has_contains_own_matches_direct_child_own_text_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="a"><p>Flag <span>child</span></p></article>
            <article data-id="b"><p><span>Flag</span></p></article>
            <article data-id="c"><p>Plain</p></article>
        </section>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(> p:containsOwn(Flag))", "data-id"),
        )
        .unwrap();

    assert_eq!(output.values(), &["a".to_string()]);

    let without_direct_flag = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:has(> p:containsOwn(Flag)))", "data-id"),
        )
        .unwrap();

    assert_eq!(
        without_direct_flag.values(),
        &["b".to_string(), "c".to_string()]
    );
}

#[test]
fn css_selector_has_middle_segment_filters_parent_before_child_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="flag"><p>Flag</p><a href="/flag">Flag</a></article>
            <article data-id="nested"><p><span>Flag</span></p><a href="/nested">Nested</a></article>
            <article data-id="plain"><p>Plain</p><a href="/plain">Plain</a></article>
        </section>
    "#;

    let direct_own_text = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(> p:containsOwn(Flag))>a", "href"),
        )
        .unwrap();

    assert_eq!(direct_own_text.values(), &["/flag".to_string()]);

    let without_direct_own_text = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:has(> p:containsOwn(Flag)))>a", "href"),
        )
        .unwrap();

    assert_eq!(
        without_direct_own_text.values(),
        &["/nested".to_string(), "/plain".to_string()]
    );

    let nested_html = r#"
        <section class="cards">
            <article data-id="a">
                <header>Featured</header>
                <div><span class="target">Target</span><a href="/a">A</a></div>
                <div><a href="/a-other">Other</a></div>
            </article>
            <article data-id="b">
                <div><span class="target">Target</span><a href="/b">B</a></div>
            </article>
            <article data-id="c">
                <header>Featured</header>
                <div><a href="/c">C</a></div>
            </article>
        </section>
    "#;

    let nested_has = engine
        .execute_step(
            nested_html,
            &RuleStep::css_attr(
                ".cards>article:has(> header)>div:has(> span.target)>a",
                "href",
            ),
        )
        .unwrap();

    assert_eq!(nested_has.values(), &["/a".to_string()]);
}

#[test]
fn css_selector_has_not_contains_own_matches_inner_negation_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="free"><p>Free</p></article>
            <article data-id="vip"><p>VIP</p></article>
            <article data-id="nested-vip"><p><span>VIP</span></p></article>
            <article data-id="mixed"><p>VIP</p><p>Free</p></article>
        </section>
    "#;

    let has_non_vip_child = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(> p:not(:containsOwn(VIP)))", "data-id"),
        )
        .unwrap();

    assert_eq!(
        has_non_vip_child.values(),
        &[
            "free".to_string(),
            "nested-vip".to_string(),
            "mixed".to_string()
        ]
    );

    let without_vip_child = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:has(> p:containsOwn(VIP)))", "data-id"),
        )
        .unwrap();

    assert_eq!(
        without_vip_child.values(),
        &["free".to_string(), "nested-vip".to_string()]
    );
}

#[test]
fn css_selector_has_result_filter_applies_before_text_filter_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="first-vip"><p>VIP</p><p>Free</p></article>
            <article data-id="second-vip"><p>Free</p><p>VIP</p></article>
            <article data-id="nested-vip"><p><span>VIP</span></p><p>Free</p></article>
            <article data-id="first-free"><p>Free</p></article>
        </section>
    "#;

    let first_child_vip = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:has(> p:eq(0):containsOwn(VIP))", "data-id"),
        )
        .unwrap();

    assert_eq!(first_child_vip.values(), &["first-vip".to_string()]);

    let without_first_child_vip = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:not(:has(> p:eq(0):containsOwn(VIP)))",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(
        without_first_child_vip.values(),
        &[
            "second-vip".to_string(),
            "nested-vip".to_string(),
            "first-free".to_string()
        ]
    );
}

#[test]
fn css_selector_contains_data_matches_script_data_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section>
            <script data-id="script">BookData = { chapter: 1 };</script>
            <p data-id="visible">BookData visible text</p>
        </section>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr("script:containsData(bookdata)", "data-id"),
        )
        .unwrap();
    let visible_text_output = engine
        .execute_step(
            html,
            &RuleStep::css_attr("p:containsData(BookData)", "data-id"),
        )
        .unwrap();

    assert_eq!(output.values(), &["script".to_string()]);
    assert!(visible_text_output.is_empty());

    let stacked_html = r#"
        <section>
            <script data-id="free">BookData = { chapter: 1 };</script>
            <script data-id="vip">BookData = { chapter: 2, tag: "VIP" };</script>
            <p data-id="visible">BookData VIP visible text</p>
        </section>
    "#;

    let public_data = engine
        .execute_step(
            stacked_html,
            &RuleStep::css_attr(
                "script:containsData(BookData):not(:containsData(VIP))",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(public_data.values(), &["free".to_string()]);
}

#[test]
fn css_selector_not_contains_data_excludes_script_data_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="article">
                <script>BookData = { chapter: 1 };</script>
            </article>
            <article data-id="plain">
                <p>Plain text</p>
            </article>
        </section>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:containsData(BookData))", "data-id"),
        )
        .unwrap();

    assert_eq!(output.values(), &["plain".to_string()]);
}

#[test]
fn css_selector_contains_data_middle_segment_filters_parent_before_child_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="script">
                <script>BookData = { chapter: 1 };</script>
                <a href="/script">Script</a>
            </article>
            <article data-id="visible">
                <p>BookData visible text</p>
                <a href="/visible">Visible</a>
            </article>
            <article data-id="plain">
                <p>Plain text</p>
                <a href="/plain">Plain</a>
            </article>
        </section>
    "#;

    let data_parent_links = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:containsData(BookData)>a", "href"),
        )
        .unwrap();

    assert_eq!(data_parent_links.values(), &["/script".to_string()]);

    let without_data_parent_links = engine
        .execute_step(
            html,
            &RuleStep::css_attr(".cards>article:not(:containsData(BookData))>a", "href"),
        )
        .unwrap();

    assert_eq!(
        without_data_parent_links.values(),
        &["/visible".to_string(), "/plain".to_string()]
    );

    let nested_html = r#"
        <section class="cards">
            <article data-id="a">
                <script>ArticleData = true;</script>
                <div>
                    <script>TargetData = true;</script>
                    <a href="/a">A</a>
                </div>
                <div><a href="/a-other">Other</a></div>
            </article>
            <article data-id="b">
                <div>
                    <script>TargetData = true;</script>
                    <a href="/b">B</a>
                </div>
            </article>
            <article data-id="c">
                <script>ArticleData = true;</script>
                <div><a href="/c">C</a></div>
            </article>
        </section>
    "#;

    let nested_data = engine
        .execute_step(
            nested_html,
            &RuleStep::css_attr(
                ".cards>article:containsData(ArticleData)>div:containsData(TargetData)>a",
                "href",
            ),
        )
        .unwrap();

    assert_eq!(nested_data.values(), &["/a".to_string()]);
}

#[test]
fn css_selector_has_contains_data_matches_descendant_data_like_jsoup() {
    let engine = RuleEngine::new();

    let html = r#"
        <section class="cards">
            <article data-id="direct">
                <script>BookData = { chapter: 1 };</script>
            </article>
            <article data-id="nested">
                <div><script>BookData = { chapter: 2 };</script></div>
            </article>
            <article data-id="plain">
                <p>Plain text</p>
            </article>
        </section>
    "#;

    let output = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:has(script:containsData(BookData))",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["direct".to_string(), "nested".to_string()]
    );

    let direct_child = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:has(> script:containsData(BookData))",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(direct_child.values(), &["direct".to_string()]);

    let without_direct_child = engine
        .execute_step(
            html,
            &RuleStep::css_attr(
                ".cards>article:not(:has(> script:containsData(BookData)))",
                "data-id",
            ),
        )
        .unwrap();

    assert_eq!(
        without_direct_child.values(),
        &["nested".to_string(), "plain".to_string()]
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
    assert_eq!(output.values(), &["/search?q=caf%C3%A9&page=1".to_string()]);
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
    assert_eq!(tags.values(), &["科幻".to_string(), "经典".to_string()]);
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
    assert_eq!(output.values(), &["沙une".to_string(), "沙丘".to_string()]);
}

#[test]
fn regex_replace_supports_dollar_sign_escaping() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_step("price: 10", &RuleStep::regex_replace(r"price:", "$$"))
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

    let b = engine
        .execute_step(json, &RuleStep::json_path("$.b"))
        .unwrap();
    assert_eq!(b.values(), &["true".to_string()]);

    let n = engine
        .execute_step(json, &RuleStep::json_path("$.n"))
        .unwrap();
    assert_eq!(n.values(), &["null".to_string()]);

    let f = engine
        .execute_step(json, &RuleStep::json_path("$.f"))
        .unwrap();
    assert_eq!(f.values(), &["3.14".to_string()]);

    let i = engine
        .execute_step(json, &RuleStep::json_path("$.i"))
        .unwrap();
    assert_eq!(i.values(), &["42".to_string()]);
}

#[test]
fn jsonpath_top_level_or_uses_first_non_empty_branch_like_legado() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Foundation" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.missing[*]||$.books[*].title"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );
}

#[test]
fn jsonpath_top_level_and_merges_non_empty_branches_like_legado() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": { "title": "Dune", "author": "Frank Herbert" },
        "titles": ["Dune", "Foundation"],
        "authors": ["Frank Herbert", "Isaac Asimov"]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.book.title&&$.book.author"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune".to_string(), "Frank Herbert".to_string()]
    );
}

#[test]
fn jsonpath_top_level_percent_zips_branches_like_legado() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "author": "Frank Herbert" },
            { "title": "Foundation", "author": "Isaac Asimov" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[*].title%%$.books[*].author"),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &[
            "Dune".to_string(),
            "Frank Herbert".to_string(),
            "Foundation".to_string(),
            "Isaac Asimov".to_string()
        ]
    );
}

#[test]
fn jsonpath_embedded_rule_template_matches_legado_get_string_list() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": { "title": "Dune", "author": "Frank Herbert" }
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("《{$.book.title}》 by {$.book.author}"),
        )
        .unwrap();

    assert_eq!(output.values(), &["《Dune》 by Frank Herbert".to_string()]);

    let combined_inside_template = engine
        .execute_step(
            json,
            &RuleStep::json_path("pair: {$.book.title&&$.book.author}"),
        )
        .unwrap();

    assert_eq!(
        combined_inside_template.values(),
        &["pair: Dune\nFrank Herbert".to_string()]
    );

    let missing_only = engine
        .execute_step(json, &RuleStep::json_path("missing {$.book.subtitle}"))
        .unwrap();
    assert!(missing_only.is_empty());

    let mixed_missing = engine
        .execute_step(
            json,
            &RuleStep::json_path("title: {$.book.title}; missing: {$.book.subtitle}"),
        )
        .unwrap();

    assert_eq!(
        mixed_missing.values(),
        &["title: Dune; missing: {$.book.subtitle}".to_string()]
    );

    let zip_inside_template = engine
        .execute_step(
            json,
            &RuleStep::json_path("zip: {$.titles[*]%%$.authors[*]}"),
        )
        .unwrap();
    assert!(zip_inside_template.is_empty());

    let mixed_zip = engine
        .execute_step(
            json,
            &RuleStep::json_path("title: {$.book.title}; zip: {$.titles[*]%%$.authors[*]}"),
        )
        .unwrap();

    assert_eq!(
        mixed_zip.values(),
        &["title: Dune; zip: {$.titles[*]%%$.authors[*]}".to_string()]
    );
}

#[test]
fn jsonpath_hash_regex_replacement_matches_legado_source_rule() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune 小说",
            "created": "2026-06-25T21:14:57+00:00",
            "cover": "/book/12345/index.html"
        }
    }"#;

    let cleaned_title = engine
        .execute_step(json, &RuleStep::json_path("$.book.title##小说"))
        .unwrap();
    assert_eq!(cleaned_title.values(), &["Dune ".to_string()]);

    let template = engine
        .execute_step(
            json,
            &RuleStep::json_path("created: {$.book.created##T|\\+.*## }"),
        )
        .unwrap();
    assert_eq!(
        template.values(),
        &["created: 2026-06-25 21:14:57 ".to_string()]
    );

    let cover_url = engine
        .execute_step(
            json,
            &RuleStep::json_path(
                "$.book.cover##.+\\D((\\d+)\\d{3})\\D##/files/article/image/$2/$1/$1s.jpg###",
            ),
        )
        .unwrap();
    assert_eq!(
        cover_url.values(),
        &["/files/article/image/12/12345/12345s.jpg".to_string()]
    );
}

#[test]
fn jsonpath_double_brace_template_matches_legado_source_rule() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune 小说",
            "created": "2026-06-25T21:14:57+00:00"
        }
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path(
                "《{{$.book.title##小说}}》 updated {{$.book.created##T|\\+.*## }}",
            ),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["《Dune 》 updated 2026-06-25 21:14:57 ".to_string()]
    );
}

#[test]
fn legado_json_prefix_routes_to_jsonpath_like_legado() {
    let engine = RuleEngine::new();

    let json = r#"{
        "cover": "/covers/dune.jpg",
        "maxChapterId": 42,
        "body": "Chapter body",
        "cpContent": "Fallback body"
    }"#;

    let cover = engine.execute_legado_css(json, "@JSon:$.cover").unwrap();
    assert_eq!(cover.values(), &["/covers/dune.jpg".to_string()]);

    let fallback = engine
        .execute_legado_css(json, "@json:$.missing||$.cover")
        .unwrap();
    assert_eq!(fallback.values(), &["/covers/dune.jpg".to_string()]);

    let template = engine
        .execute_legado_css(json, "@Json:第{$.maxChapterId}章")
        .unwrap();
    assert_eq!(template.values(), &["第42章".to_string()]);

    let merged = engine
        .execute_legado_css(json, "@JSON:$..body&&$..cpContent")
        .unwrap();
    assert_eq!(
        merged.values(),
        &["Chapter body".to_string(), "Fallback body".to_string()]
    );
}

#[test]
fn legado_bare_jsonpath_rule_routes_to_jsonpath_like_legado() {
    let engine = RuleEngine::new();

    let json = r#"{
        "cover": "/covers/dune.jpg",
        "books": [
            { "title": "Dune" },
            { "title": "Foundation" }
        ]
    }"#;

    let cover = engine.execute_legado_css(json, "$.cover").unwrap();
    assert_eq!(cover.values(), &["/covers/dune.jpg".to_string()]);

    let indexed = engine
        .execute_legado_css(r#"[{"title":"Dune"}]"#, "$[0].title")
        .unwrap();
    assert_eq!(indexed.values(), &["Dune".to_string()]);

    let recursive = engine.execute_legado_css(json, "$..title").unwrap();
    assert_eq!(
        recursive.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );
}

#[test]
fn legado_xpath_prefix_routes_to_xpath_like_legado() {
    let engine = RuleEngine::new();

    let xml = r#"
        <player>
            <div id="jp_container_1">
                <ul id="jp-lines">
                    <li data-code="abc-123">Line</li>
                </ul>
            </div>
        </player>
    "#;

    let code = engine
        .execute_legado_css(
            xml,
            "@XPath://div[@id=\"jp_container_1\"][1]/ul[@id=\"jp-lines\"]/li[1]/@data-code",
        )
        .unwrap();
    assert_eq!(code.values(), &["abc-123".to_string()]);

    let text = engine
        .execute_legado_css(xml, "@xpath://li/text()")
        .unwrap();
    assert_eq!(text.values(), &["Line".to_string()]);
}

#[test]
fn legado_bare_slash_rule_routes_to_xpath_like_legado() {
    let engine = RuleEngine::new();

    let xml = r#"
        <catalog>
            <book href="/books/1">
                <title>Dune</title>
            </book>
        </catalog>
    "#;

    let href = engine.execute_legado_css(xml, "//book[1]/@href").unwrap();
    assert_eq!(href.values(), &["/books/1".to_string()]);

    let title = engine
        .execute_legado_css(xml, "/catalog/book/title/text()")
        .unwrap();
    assert_eq!(title.values(), &["Dune".to_string()]);
}

#[test]
fn legado_xpath_combinations_match_legado_rule_split() {
    let engine = RuleEngine::new();

    let xml = r#"
        <catalog>
            <book href="/books/1">
                <title>Dune</title>
            </book>
            <book href="/books/2">
                <title>Foundation</title>
            </book>
        </catalog>
    "#;

    let fallback = engine
        .execute_legado_css(xml, "@XPath://missing||//book/title/text()")
        .unwrap();
    assert_eq!(
        fallback.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let merged = engine
        .execute_legado_css(xml, "@XPath://book/title/text()&&//book/@href")
        .unwrap();
    assert_eq!(
        merged.values(),
        &[
            "Dune".to_string(),
            "Foundation".to_string(),
            "/books/1".to_string(),
            "/books/2".to_string()
        ]
    );

    let zipped = engine
        .execute_legado_css(xml, "@XPath://book/title/text()%%//book/@href")
        .unwrap();
    assert_eq!(
        zipped.values(),
        &[
            "Dune".to_string(),
            "/books/1".to_string(),
            "Foundation".to_string(),
            "/books/2".to_string()
        ]
    );
}

#[test]
fn legado_xpath_hash_regex_replacement_matches_source_rule() {
    let engine = RuleEngine::new();

    let xml = r#"
        <catalog>
            <book href="/books/1">
                <title>卷一·Dune</title>
            </book>
            <book href="/books/2">
                <title>卷二·Foundation</title>
            </book>
        </catalog>
    "#;

    let titles = engine
        .execute_legado_css(xml, "@XPath://book/title/text()##卷[一二]·##")
        .unwrap();
    assert_eq!(
        titles.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let href_id = engine
        .execute_legado_css(xml, "@XPath://book/@href##.+/(\\d+)##id=$1###")
        .unwrap();
    assert_eq!(href_id.values(), &["id=1".to_string(), "id=2".to_string()]);
}

#[test]
fn legado_xpath_double_brace_template_matches_source_rule() {
    let engine = RuleEngine::new();

    let xml = r#"
        <metadata>
            <data name="Title">Dune</data>
            <data name="Author" value="Frank Herbert" />
            <data name="BookUrl" value="https://example.test/book/42" />
        </metadata>
    "#;

    let intro = engine
        .execute_legado_css(
            xml,
            "📖 书名：{{//data[@name='Title']/text()}}\n✏️ 作者：{{//data[@name='Author']/@value}}",
        )
        .unwrap();
    assert_eq!(
        intro.values(),
        &["📖 书名：Dune\n✏️ 作者：Frank Herbert".to_string()]
    );

    let book_id = engine
        .execute_legado_css(
            xml,
            "id={{//data[@name='BookUrl']/@value##.*/(\\d+)##$1###}}",
        )
        .unwrap();
    assert_eq!(book_id.values(), &["id=42".to_string()]);
}

#[test]
fn legado_put_get_context_matches_source_rule_variables() {
    let engine = RuleEngine::new();
    let mut context = LegadoRuleContext::new();

    let book_json = r#"{
        "id": "book-42",
        "name": "Dune",
        "author": "Frank Herbert"
    }"#;

    let name = engine
        .execute_legado_css_with_context(
            book_json,
            "$.name@put:{id:id,name:name,author:author}",
            &mut context,
        )
        .unwrap();
    assert_eq!(name.values(), &["Dune".to_string()]);
    assert_eq!(context.get_variable("id"), Some("book-42"));
    assert_eq!(context.get_variable("name"), Some("Dune"));
    assert_eq!(context.get_variable("author"), Some("Frank Herbert"));
    context.put_variable("toc", "/books/book-42/toc");

    let chapter_json = r#"{ "chapter_id": "chapter-7" }"#;
    let url = engine
        .execute_legado_css_with_context(
            chapter_json,
            "https://api.example.test/books/@get:{id}/chapters/{{$.chapter_id}}",
            &mut context,
        )
        .unwrap();
    assert_eq!(
        url.values(),
        &["https://api.example.test/books/book-42/chapters/chapter-7".to_string()]
    );

    let author = engine
        .execute_legado_css_with_context(book_json, "@get:{author}", &mut context)
        .unwrap();
    assert_eq!(author.values(), &["Frank Herbert".to_string()]);

    let toc_url = engine
        .execute_legado_css_with_context(
            book_json,
            "@get:{toc}##$##,{'webView': true}",
            &mut context,
        )
        .unwrap();
    assert_eq!(
        toc_url.values(),
        &["/books/book-42/toc,{'webView': true}".to_string()]
    );
}

#[test]
fn jsonpath_filter_by_string_field_keeps_matching_array_items() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel" },
            { "title": "Foundation", "category": "novel" },
            { "title": "Intro", "category": "metadata" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.category == 'novel')].title"),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );
}

#[test]
fn jsonpath_filter_matches_regex_literal_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "dune appendix" },
            { "title": "Foundation" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.title =~ /^dune/i)].title"),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune".to_string(), "dune appendix".to_string()]
    );
}

#[test]
fn jsonpath_filter_supports_in_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel" },
            { "title": "Notes", "category": "essay" },
            { "title": "Index", "category": "metadata" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.category in ['novel','essay'])].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_nin_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel" },
            { "title": "Notes", "category": "essay" },
            { "title": "Index", "category": "metadata" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.category nin ['metadata'])].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_anyof_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "tags": ["novel", "space"] },
            { "title": "Notes", "tags": ["essay"] },
            { "title": "Index", "tags": ["metadata"] }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.tags anyof ['space','classic'])].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string()]);
}

#[test]
fn jsonpath_filter_supports_noneof_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "tags": ["novel", "space"] },
            { "title": "Notes", "tags": ["essay"] },
            { "title": "Index", "tags": ["metadata"] }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.tags noneof ['metadata'])].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_subsetof_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "tags": ["novel", "space"] },
            { "title": "Notes", "tags": ["essay"] },
            { "title": "Index", "tags": ["metadata"] }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.tags subsetof ['novel','space','classic'])].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string()]);
}

#[test]
fn jsonpath_filter_supports_size_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "tags": ["novel", "space"] },
            { "title": "Notes", "tags": ["essay"] },
            { "title": "Index", "tags": [] }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.tags size 2)].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string()]);
}

#[test]
fn jsonpath_filter_supports_empty_operator_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "tags": ["novel", "space"] },
            { "title": "Notes", "tags": ["essay"] },
            { "title": "Index", "tags": [] }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books[?(@.tags empty)].title"))
        .unwrap();

    assert_eq!(output.values(), &["Index".to_string()]);
}

#[test]
fn jsonpath_supports_length_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" },
            { "title": "Index" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books.length()"))
        .unwrap();

    assert_eq!(output.values(), &["3".to_string()]);
}

#[test]
fn jsonpath_supports_string_length_function_like_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "title": "Dune"
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.title.length()"))
        .unwrap();

    assert_eq!(output.values(), &["4".to_string()]);
}

#[test]
fn jsonpath_supports_length_path_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" },
            { "title": "Index" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.length($.books)"))
        .unwrap();

    assert_eq!(output.values(), &["3".to_string()]);
}

#[test]
fn jsonpath_supports_object_length_function_like_jayway_and_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune",
            "author": "Frank Herbert"
        }
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.book.length()"))
        .unwrap();

    assert_eq!(output.values(), &["2".to_string()]);
}

#[test]
fn jsonpath_supports_size_alias_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune",
            "author": "Frank Herbert"
        }
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.book.size()"))
        .unwrap();

    assert_eq!(output.values(), &["2".to_string()]);
}

#[test]
fn jsonpath_supports_size_path_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune",
            "author": "Frank Herbert"
        }
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.size($.book)"))
        .unwrap();

    assert_eq!(output.values(), &["2".to_string()]);
}

#[test]
fn jsonpath_supports_first_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books.first()"))
        .unwrap();

    assert_eq!(output.values(), &[r#"{"title":"Dune"}"#.to_string()]);
}

#[test]
fn jsonpath_supports_path_after_first_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books.first().title"))
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string()]);
}

#[test]
fn jsonpath_supports_last_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books.last()"))
        .unwrap();

    assert_eq!(output.values(), &[r#"{"title":"Notes"}"#.to_string()]);
}

#[test]
fn jsonpath_supports_index_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" },
            { "title": "Index" }
        ]
    }"#;

    let second = engine
        .execute_step(json, &RuleStep::json_path("$.books.index(1)"))
        .unwrap();
    let last = engine
        .execute_step(json, &RuleStep::json_path("$.books.index(-1)"))
        .unwrap();

    assert_eq!(second.values(), &[r#"{"title":"Notes"}"#.to_string()]);
    assert_eq!(last.values(), &[r#"{"title":"Index"}"#.to_string()]);
}

#[test]
fn jsonpath_supports_index_path_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune" },
            { "title": "Notes" },
            { "title": "Index" }
        ],
        "selected": 1
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books.index($.selected)"))
        .unwrap();

    assert_eq!(output.values(), &[r#"{"title":"Notes"}"#.to_string()]);
}

#[test]
fn jsonpath_supports_sum_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [1.5, 2.25]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.sum()"))
        .unwrap();

    assert_eq!(output.values(), &["3.75".to_string()]);
}

#[test]
fn jsonpath_supports_sum_numeric_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [1.5, 2.25]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.sum(3)"))
        .unwrap();

    assert_eq!(output.values(), &["6.75".to_string()]);
}

#[test]
fn jsonpath_supports_min_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [2.25, 1.5, 3.75]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.min()"))
        .unwrap();

    assert_eq!(output.values(), &["1.5".to_string()]);
}

#[test]
fn jsonpath_supports_min_numeric_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [2.25, 3.75]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.min(1.5)"))
        .unwrap();

    assert_eq!(output.values(), &["1.5".to_string()]);
}

#[test]
fn jsonpath_supports_max_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [2.25, 1.5, 3.75]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.max()"))
        .unwrap();

    assert_eq!(output.values(), &["3.75".to_string()]);
}

#[test]
fn jsonpath_supports_max_numeric_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [2.25, 1.5]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.max(3.75)"))
        .unwrap();

    assert_eq!(output.values(), &["3.75".to_string()]);
}

#[test]
fn jsonpath_supports_avg_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [1.5, 2.25]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.avg()"))
        .unwrap();

    assert_eq!(output.values(), &["1.875".to_string()]);
}

#[test]
fn jsonpath_supports_avg_numeric_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [1.5, 2.25]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.avg(3)"))
        .unwrap();

    assert_eq!(output.values(), &["2.25".to_string()]);
}

#[test]
fn jsonpath_supports_stddev_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [1.5, 2.25]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.stddev()"))
        .unwrap();

    assert_eq!(output.values(), &["0.375".to_string()]);
}

#[test]
fn jsonpath_supports_stddev_numeric_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "scores": [1]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.scores.stddev(3)"))
        .unwrap();

    assert_eq!(output.values(), &["1.0".to_string()]);
}

#[test]
fn jsonpath_supports_keys_function_like_jayway_and_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune",
            "author": "Frank Herbert"
        }
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.book.keys()"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["author".to_string(), "title".to_string()]
    );
}

#[test]
fn jsonpath_supports_values_function_like_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "book": {
            "title": "Dune",
            "author": "Frank Herbert"
        }
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.book.values()"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["Frank Herbert".to_string(), "Dune".to_string()]
    );
}

#[test]
fn jsonpath_supports_concat_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "parts": ["Read", "er", 7]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.parts.concat('-Core')"))
        .unwrap();

    assert_eq!(output.values(), &["Reader-Core".to_string()]);
}

#[test]
fn jsonpath_supports_concat_multiple_parameters_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "parts": ["Read", "er", 7]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.parts.concat('-','Core')"))
        .unwrap();

    assert_eq!(output.values(), &["Reader-Core".to_string()]);
}

#[test]
fn jsonpath_supports_concat_root_path_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "parts": ["Read", "er"],
        "suffix": "Core"
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.parts.concat($.suffix)"))
        .unwrap();

    assert_eq!(output.values(), &["ReaderCore".to_string()]);
}

#[test]
fn jsonpath_supports_append_function_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": ["a", "b"]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items.append('c')"))
        .unwrap();

    assert_eq!(output.values(), &[r#"["a","b","c"]"#.to_string()]);
}

#[test]
fn jsonpath_supports_append_multiple_parameters_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": ["a", "b"]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items.append('c','d')"))
        .unwrap();

    assert_eq!(output.values(), &[r#"["a","b","c","d"]"#.to_string()]);
}

#[test]
fn jsonpath_supports_append_numeric_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": ["a", "b"]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items.append(3)"))
        .unwrap();

    assert_eq!(output.values(), &[r#"["a","b",3]"#.to_string()]);
}

#[test]
fn jsonpath_supports_append_json_object_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": ["a"]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.items.append({\"x\":1,\"y\":2})"),
        )
        .unwrap();

    assert_eq!(output.values(), &[r#"["a",{"x":1,"y":2}]"#.to_string()]);
}

#[test]
fn jsonpath_supports_append_root_path_parameter_like_jayway() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": ["a"],
        "extra": { "x": 1 }
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items.append($.extra)"))
        .unwrap();

    assert_eq!(output.values(), &[r#"["a",{"x":1}]"#.to_string()]);
}

#[test]
fn jsonpath_filter_compares_numeric_fields() {
    let engine = RuleEngine::new();

    let json = r#"{
        "chapters": [
            { "title": "Intro", "order": 1 },
            { "title": "Chapter 1", "order": 2 },
            { "title": "Chapter 2", "order": 3 }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.chapters[?(@.order >= 2)].title"),
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["Chapter 1".to_string(), "Chapter 2".to_string()]
    );
}

#[test]
fn jsonpath_filter_supports_bracket_quoted_field_names() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category.name": "novel" },
            { "title": "Intro", "category.name": "metadata" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@['category.name'] == 'novel')].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string()]);
}

#[test]
fn jsonpath_filter_compares_against_current_item_field() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": [
            { "name": "low", "score": 2, "min": 5 },
            { "name": "pass", "score": 5, "min": 5 },
            { "name": "high", "score": 8, "min": 5 }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.items[?(@.score >= @.min)].name"),
        )
        .unwrap();

    assert_eq!(output.values(), &["pass".to_string(), "high".to_string()]);
}

#[test]
fn jsonpath_filter_compares_current_scalar_item_like_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "tags": ["novel", "essay", "metadata"]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.tags[?(@ == 'novel')]"))
        .unwrap();

    assert_eq!(output.values(), &["novel".to_string()]);
}

#[test]
fn jsonpath_filter_keeps_items_where_field_exists() {
    let engine = RuleEngine::new();

    let json = r#"{
        "links": [
            { "title": "Keep", "href": "/book/1" },
            { "title": "Drop" },
            { "title": "Also Keep", "href": "" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.links[?(@.href)].title"))
        .unwrap();

    assert_eq!(
        output.values(),
        &["Keep".to_string(), "Also Keep".to_string()]
    );
}

#[test]
fn jsonpath_filter_supports_and_conditions() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel", "enabled": true },
            { "title": "Draft", "category": "novel", "enabled": false },
            { "title": "Index", "category": "metadata", "enabled": true }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(@.category == 'novel' && @.enabled == true)].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string()]);
}

#[test]
fn jsonpath_filter_supports_or_conditions() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel" },
            { "title": "Notes", "category": "essay" },
            { "title": "Index", "category": "metadata" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path(
                "$.books[?(@.category == 'novel' || @.category == 'essay')].title",
            ),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_textual_or_conditions_like_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel" },
            { "title": "Notes", "category": "essay" },
            { "title": "Index", "category": "metadata" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path(
                "$.books[?(@.category == 'novel' or @.category == 'essay')].title",
            ),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_grouped_boolean_conditions() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel", "enabled": true },
            { "title": "Draft", "category": "novel", "enabled": false },
            { "title": "Notes", "category": "essay", "enabled": true },
            { "title": "Index", "category": "metadata", "enabled": true }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path(
                "$.books[?((@.category == 'novel' || @.category == 'essay') && @.enabled == true)].title",
            ),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_not_conditions() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "category": "novel" },
            { "title": "Index", "category": "metadata" },
            { "title": "Notes", "category": "essay" }
        ]
    }"#;

    let output = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.books[?(!(@.category == 'metadata'))].title"),
        )
        .unwrap();

    assert_eq!(output.values(), &["Dune".to_string(), "Notes".to_string()]);
}

#[test]
fn jsonpath_filter_supports_not_existence_function_like_old_core() {
    let engine = RuleEngine::new();

    let json = r#"{
        "books": [
            { "title": "Dune", "cover": "dune.jpg" },
            { "title": "Notes" },
            { "title": "Index", "cover": "index.jpg" }
        ]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.books[?(not(@.cover))].title"))
        .unwrap();

    assert_eq!(output.values(), &["Notes".to_string()]);
}

#[test]
fn jsonpath_union_extracts_multiple_array_indices() {
    let engine = RuleEngine::new();

    let json = r#"{
        "items": ["zero", "one", "two", "three"]
    }"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[0,2]"))
        .unwrap();

    assert_eq!(output.values(), &["zero".to_string(), "two".to_string()]);
}

// ---------------------------------------------------------------------------
// JSONPath slice expressions
// ---------------------------------------------------------------------------

#[test]
fn jsonpath_slice_extracts_inclusive_start_exclusive_end() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b", "c", "d", "e"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[1:3]"))
        .unwrap();
    assert_eq!(output.values(), &["b".to_string(), "c".to_string()]);
}

#[test]
fn jsonpath_slice_open_ended_end_goes_to_last() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b", "c", "d", "e"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[2:]"))
        .unwrap();
    assert_eq!(
        output.values(),
        &["c".to_string(), "d".to_string(), "e".to_string()]
    );
}

#[test]
fn jsonpath_slice_open_ended_start_begins_at_zero() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b", "c", "d", "e"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[:2]"))
        .unwrap();
    assert_eq!(output.values(), &["a".to_string(), "b".to_string()]);
}

#[test]
fn jsonpath_slice_negative_start_counts_from_end() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b", "c", "d", "e"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[-2:]"))
        .unwrap();
    assert_eq!(output.values(), &["d".to_string(), "e".to_string()]);
}

#[test]
fn jsonpath_slice_positive_step_skips_elements() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b", "c", "d", "e"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[::2]"))
        .unwrap();
    assert_eq!(
        output.values(),
        &["a".to_string(), "c".to_string(), "e".to_string()]
    );
}

#[test]
fn jsonpath_slice_out_of_range_returns_empty() {
    let engine = RuleEngine::new();

    let json = r#"{"items": ["a", "b"]}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[5:10]"))
        .unwrap();
    assert!(output.is_empty());
}

#[test]
fn jsonpath_slice_on_non_array_returns_empty() {
    let engine = RuleEngine::new();

    let json = r#"{"items": "not-an-array"}"#;

    let output = engine
        .execute_step(json, &RuleStep::json_path("$.items[0:2]"))
        .unwrap();
    assert!(output.is_empty());
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

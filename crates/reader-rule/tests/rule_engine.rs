use reader_rule::{CaptureGroup, RuleEngine, RuleError, RuleStep};

#[test]
fn regex_capture_and_replace() {
    let engine = RuleEngine::new();

    let captured = engine
        .execute_step(
            "Book: Dune #42",
            &RuleStep::regex_capture(r"Book: ([^#]+) #(\d+)", CaptureGroup::index(1)),
        )
        .unwrap();
    assert_eq!(captured.values(), &["Dune".to_string()]);

    let replaced = engine
        .execute_step(
            "chapter-001 chapter-002",
            &RuleStep::regex_replace(r"chapter-(\d+)", "c$1"),
        )
        .unwrap();
    assert_eq!(replaced.values(), &["c001 c002".to_string()]);
}

#[test]
fn jsonpath_single_multi_and_missing_values() {
    let engine = RuleEngine::new();
    let json = r#"{
        "book": { "title": "Dune" },
        "items": [
            { "name": "Dune" },
            { "name": "Foundation" }
        ]
    }"#;

    let title = engine
        .execute_step(json, &RuleStep::json_path("$.book.title"))
        .unwrap();
    assert_eq!(title.values(), &["Dune".to_string()]);

    let names = engine
        .execute_step(json, &RuleStep::json_path("$.items[*].name"))
        .unwrap();
    assert_eq!(
        names.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let missing = engine
        .execute_step(json, &RuleStep::json_path("$.book.author.name"))
        .unwrap();
    assert!(missing.is_empty());
}

// ===========================================================================
// Issue 4 (batch v4 json_parse_error): Legado JSONPath variants
// ===========================================================================

#[test]
fn jsonpath_supports_bracket_after_dot_wildcard() {
    // Legado variant `$.[*]` — bracket directly after the dot. Should behave
    // the same as `$[*]`. From corpus-batch-v4 src-084: `bookList: "$.[*]"`.
    // The root must be an array for `$.[*]` to iterate elements.
    let engine = RuleEngine::new();
    let json = r#"[
        { "name": "Dune" },
        { "name": "Foundation" }
    ]"#;

    let names = engine
        .execute_step(json, &RuleStep::json_path("$.[*].name"))
        .expect("$.[*].name should parse");
    assert_eq!(
        names.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );
}

#[test]
fn jsonpath_supports_bracket_after_dot_index() {
    // `$.[0]` and `$.[1]` should behave like `$[0]` and `$[1]`.
    let engine = RuleEngine::new();
    let json = r#"[{"name":"first"},{"name":"second"}]"#;

    let first = engine
        .execute_step(json, &RuleStep::json_path("$.[0].name"))
        .expect("$.[0].name should parse");
    assert_eq!(first.values(), &["first".to_string()]);

    let second = engine
        .execute_step(json, &RuleStep::json_path("$.[1].name"))
        .expect("$.[1].name should parse");
    assert_eq!(second.values(), &["second".to_string()]);
}

#[test]
fn jsonpath_supports_legado_template_with_literal_suffix() {
    // Legado template syntax `{$.field}literal` — evaluate `$.field` and
    // append the literal suffix to each result. From corpus-batch-v4 src-034:
    // `kind: "$.chapters_update_time&&$.c_class_name&&{$.crazy_rating}分"`.
    let engine = RuleEngine::new();
    let json = r#"{"crazy_rating": "9.5"}"#;

    let rating = engine
        .execute_step(json, &RuleStep::json_path("{$.crazy_rating}分"))
        .expect("{$.crazy_rating}分 should parse");
    assert_eq!(rating.values(), &["9.5分".to_string()]);
}

#[test]
fn jsonpath_legado_template_without_suffix_returns_value() {
    // `{$.field}` (no literal suffix) should just return the field value.
    let engine = RuleEngine::new();
    let json = r#"{"rating": 8.5}"#;

    let rating = engine
        .execute_step(json, &RuleStep::json_path("{$.rating}"))
        .expect("{$.rating} should parse");
    assert_eq!(rating.values(), &["8.5".to_string()]);
}

#[test]
fn jsonpath_legado_template_in_and_combined_rule() {
    // The full Legado `kind` rule from src-034 combines three branches with
    // `&&`; the third branch is the `{$.crazy_rating}分` template.
    let engine = RuleEngine::new();
    let json = r#"{
        "chapters_update_time": "2024-01-01",
        "c_class_name": "玄幻",
        "crazy_rating": "9.5"
    }"#;

    let combined = engine
        .execute_step(
            json,
            &RuleStep::json_path("$.chapters_update_time&&$.c_class_name&&{$.crazy_rating}分"),
        )
        .expect("combined rule should parse");
    assert_eq!(
        combined.values(),
        &[
            "2024-01-01".to_string(),
            "玄幻".to_string(),
            "9.5分".to_string()
        ]
    );
}

#[test]
fn css_selector_text_and_attribute_extraction() {
    let engine = RuleEngine::new();
    let html = r#"
        <main>
            <a class="book" href="/book/1"><span>Dune</span></a>
            <a class="book" href="/book/2">Foundation</a>
        </main>
    "#;

    let text = engine
        .execute_step(html, &RuleStep::css_text("a.book"))
        .unwrap();
    assert_eq!(
        text.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let hrefs = engine
        .execute_step(html, &RuleStep::css_attr("a.book", "href"))
        .unwrap();
    assert_eq!(
        hrefs.values(),
        &["/book/1".to_string(), "/book/2".to_string()]
    );
}

#[test]
fn legado_css_text_nodes_extracts_text_recursively() {
    // Regression for rb-legado-textnodes-extraction: `@textNodes` was
    // misinterpreted as an HTML attribute lookup and returned empty output,
    // breaking content extraction for sources like 快眼看书
    // (`ruleContent.content = "id.chaptercontent@textNodes"`).
    let engine = RuleEngine::new();
    let html = r#"
        <div id="chaptercontent">
            <div id="center_tip"><b>最新网址：www.example.com</b></div>
            <p>"斗之力，三段！"</p>
            <p>望着测验魔石碑上面闪亮得甚至有些刺眼的五个大字。</p>
        </div>
    "#;

    let out = engine
        .execute_legado_css(html, "id.chaptercontent@textNodes")
        .unwrap();
    let values = out.values();
    assert_eq!(values.len(), 1, "textNodes should produce one text value");
    let content = &values[0];
    assert!(
        content.contains("斗之力，三段！"),
        "textNodes should include nested text, got: {content:?}"
    );
    assert!(
        content.contains("望着测验魔石碑"),
        "textNodes should include all nested paragraphs, got: {content:?}"
    );
    assert!(
        content.contains("最新网址"),
        "textNodes should include text from nested <div>/<b>, got: {content:?}"
    );
}

#[test]
fn legado_css_own_text_excludes_children() {
    // `@ownText` returns only the element's direct text, excluding text from
    // child elements. Mirrors Jsoup `Node.ownText()`.
    let engine = RuleEngine::new();
    let html = r#"
        <div id="chapter">
            Direct text
            <span>child text</span>
        </div>
    "#;

    let out = engine
        .execute_legado_css(html, "id.chapter@ownText")
        .unwrap();
    let values = out.values();
    assert_eq!(values.len(), 1, "ownText should produce one text value");
    let content = &values[0];
    assert!(
        content.contains("Direct text"),
        "ownText should include direct text, got: {content:?}"
    );
    assert!(
        !content.contains("child text"),
        "ownText should exclude child element text, got: {content:?}"
    );
}

#[test]
fn xpath_extracts_node_and_scalar_values() {
    let engine = RuleEngine::new();
    let xml = r#"
        <root>
            <book id="1"><title>Dune</title></book>
            <book id="2"><title>Foundation</title></book>
        </root>
    "#;

    let titles = engine
        .execute_step(xml, &RuleStep::xpath("//book/title/text()"))
        .unwrap();
    assert_eq!(
        titles.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let title = engine
        .execute_step(xml, &RuleStep::xpath("string(//book[@id='2']/title)"))
        .unwrap();
    assert_eq!(title.values(), &["Foundation".to_string()]);
}

#[test]
fn chained_rules_execute_in_order_and_preserve_multi_results() {
    let engine = RuleEngine::new();
    let html = r#"
        <ul>
            <li class="item">book-100</li>
            <li class="item">book-200</li>
        </ul>
    "#;

    let output = engine
        .execute_chain(
            html,
            &[
                RuleStep::css_text("li.item"),
                RuleStep::regex_capture(r"book-(\d+)", CaptureGroup::index(1)),
                RuleStep::regex_replace(r"^", "id:"),
            ],
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["id:100".to_string(), "id:200".to_string()]
    );
}

#[test]
fn empty_results_short_circuit_later_steps() {
    let engine = RuleEngine::new();
    let html = "<main><span>no match</span></main>";

    let output = engine
        .execute_chain(
            html,
            &[
                RuleStep::css_text(".missing"),
                RuleStep::regex_capture("[", CaptureGroup::WholeMatch),
            ],
        )
        .unwrap();

    assert!(output.is_empty());
}

#[test]
fn chained_rule_errors_include_the_failing_step_index() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_chain(
            "<main><span>book-100</span></main>",
            &[
                RuleStep::css_text("span"),
                RuleStep::regex_capture("[", CaptureGroup::WholeMatch),
            ],
        )
        .unwrap_err();

    match error {
        RuleError::ChainStepFailed { index, source } => {
            assert_eq!(index, 1);
            assert!(matches!(*source, RuleError::RegexSyntax { .. }));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

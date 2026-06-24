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

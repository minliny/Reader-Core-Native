use reader_rule::{CaptureGroup, RuleEngine, RuleError, RuleStep};

const HTML: &str = include_str!("fixtures/catalog.html");
const JSON: &str = include_str!("fixtures/catalog.json");
const XML: &str = include_str!("fixtures/catalog.xml");

#[test]
fn css_handles_multi_selectors_missing_attrs_and_text_normalization() {
    let engine = RuleEngine::new();

    let titles = engine
        .execute_step(HTML, &RuleStep::css_text(".book-title, .featured .title"))
        .unwrap();
    assert_eq!(
        titles.values(),
        &[
            "Dune & Foundation".to_string(),
            "Missing Href".to_string(),
            "The Left Hand of Darkness".to_string()
        ]
    );

    let hrefs = engine
        .execute_step(HTML, &RuleStep::css_attr("a.book-link", "href"))
        .unwrap();
    assert_eq!(hrefs.values(), &["/book/1".to_string()]);
}

#[test]
fn regex_extracts_multiple_named_and_numbered_captures() {
    let engine = RuleEngine::new();
    let input = "title:Dune id:42; title:Foundation id:7";
    let pattern = r"title:(?P<title>[A-Za-z]+) id:(\d+)";

    let all_groups = engine
        .execute_step(input, &RuleStep::regex_captures(pattern))
        .unwrap();
    assert_eq!(
        all_groups.values(),
        &[
            "Dune".to_string(),
            "42".to_string(),
            "Foundation".to_string(),
            "7".to_string()
        ]
    );

    let names = engine
        .execute_step(
            input,
            &RuleStep::regex_capture(pattern, CaptureGroup::name("title")),
        )
        .unwrap();
    assert_eq!(
        names.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let ids = engine
        .execute_step(
            input,
            &RuleStep::regex_capture(pattern, CaptureGroup::index(2)),
        )
        .unwrap();
    assert_eq!(ids.values(), &["42".to_string(), "7".to_string()]);
}

#[test]
fn regex_replace_supports_existing_refs_and_errors_on_missing_refs() {
    let engine = RuleEngine::new();

    let replaced = engine
        .execute_step(
            "title:Dune id:42",
            &RuleStep::regex_replace(r"title:(?P<title>[A-Za-z]+) id:(\d+)", "${title}#$2"),
        )
        .unwrap();
    assert_eq!(replaced.values(), &["Dune#42".to_string()]);

    let error = engine
        .execute_step(
            "title:Dune id:42",
            &RuleStep::regex_replace(r"title:(?P<title>[A-Za-z]+) id:(\d+)", "$missing"),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        RuleError::RegexReplacementCaptureMissing { group, .. } if group == "missing"
    ));
}

#[test]
fn jsonpath_handles_nested_arrays_wildcards_quoted_keys_and_missing_fields() {
    let engine = RuleEngine::new();

    let source = engine
        .execute_step(JSON, &RuleStep::json_path("$['meta']['source.name']"))
        .unwrap();
    assert_eq!(source.values(), &["fixture-source".to_string()]);

    let titles = engine
        .execute_step(JSON, &RuleStep::json_path("$.sections[*].books[*].title"))
        .unwrap();
    assert_eq!(
        titles.values(),
        &[
            "Dune".to_string(),
            "Foundation".to_string(),
            "The Left Hand of Darkness".to_string()
        ]
    );

    let tags = engine
        .execute_step(JSON, &RuleStep::json_path("$.sections[*].books[*].tags[*]"))
        .unwrap();
    assert_eq!(
        tags.values(),
        &[
            "sci-fi".to_string(),
            "classic".to_string(),
            "sci-fi".to_string()
        ]
    );

    let missing = engine
        .execute_step(JSON, &RuleStep::json_path("$.sections[*].books[*].isbn"))
        .unwrap();
    assert!(missing.is_empty());
}

#[test]
fn xpath_supports_registered_namespaces_and_errors_without_them() {
    let engine = RuleEngine::new();

    let titles = engine
        .execute_step(
            XML,
            &RuleStep::xpath_with_namespaces("//r:book/r:title/text()", [("r", "urn:reader:test")]),
        )
        .unwrap();
    assert_eq!(
        titles.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let error = engine
        .execute_step(XML, &RuleStep::xpath("//r:book/r:title/text()"))
        .unwrap_err();
    assert!(matches!(error, RuleError::XPathEvaluation { .. }));
}

#[test]
fn chained_rules_expand_multi_input_to_multi_output() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".series"),
                RuleStep::regex_capture(r"\d+", CaptureGroup::WholeMatch),
            ],
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &[
            "10".to_string(),
            "11".to_string(),
            "20".to_string(),
            "21".to_string()
        ]
    );
}

#[test]
fn chained_rules_short_circuit_empty_results_before_invalid_later_steps() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".does-not-exist"),
                RuleStep::regex_replace("[", "$missing"),
            ],
        )
        .unwrap();

    assert!(output.is_empty());
}

#[test]
fn chained_rules_preserve_error_step_index_after_successful_step() {
    let engine = RuleEngine::new();

    let error = engine
        .execute_chain(
            HTML,
            &[
                RuleStep::css_text(".book-title"),
                RuleStep::json_path("$.not_json"),
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

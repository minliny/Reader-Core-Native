use reader_rule::{LegadoCssExtraction, LegadoCssRule, LegadoCssStep, RuleEngine};

const HTML: &str = r#"
    <main>
        <section class="summary">
            <a class="book" href="/book/1"><span>Dune</span></a>
            <a class="book" href="/book/2">Foundation</a>
        </section>
        <article class="chapter">
            <h1>The Chapter</h1>
            <p><b>First</b> paragraph.</p>
        </article>
        <div class="list">
            <div class="item">
                <div class="name"><a href="/first">First</a></div>
            </div>
            <div class="item">
                <div class="name"><a href="/second">Second</a></div>
            </div>
        </div>
    </main>
"#;

#[test]
fn legado_css_parser_exposes_pipeline_ast() {
    let rule = LegadoCssRule::parse("div.list&&div.item;div.name&&a@text").unwrap();

    assert_eq!(
        rule.steps(),
        &[
            LegadoCssStep::Select("div.list".to_string()),
            LegadoCssStep::Select("div.item".to_string()),
            LegadoCssStep::Select("div.name".to_string()),
            LegadoCssStep::Extract {
                selector: Some("a".to_string()),
                extraction: LegadoCssExtraction::Text,
            },
        ]
    );

    let href = LegadoCssRule::parse("a&&@href").unwrap();
    assert_eq!(
        href.steps(),
        &[
            LegadoCssStep::Select("a".to_string()),
            LegadoCssStep::Extract {
                selector: None,
                extraction: LegadoCssExtraction::Attr("href".to_string()),
            },
        ]
    );
}

#[test]
fn legado_css_extracts_text_html_and_href() {
    let engine = RuleEngine::new();

    let text = engine.execute_legado_css(HTML, "a.book@text").unwrap();
    assert_eq!(
        text.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let html = engine
        .execute_legado_css(HTML, "article.chapter@html")
        .unwrap();
    // Legado `@html` uses `element.toString()` = OUTER HTML (element tag +
    // content), mirroring `AnalyzeByJSoup.kt` `getResultList`/`getString`.
    assert_eq!(
        html.values(),
        &["<article class=\"chapter\">\n            <h1>The Chapter</h1>\n            <p><b>First</b> paragraph.</p>\n        </article>".to_string()]
    );

    let href = engine.execute_legado_css(HTML, "a.book@href").unwrap();
    assert_eq!(
        href.values(),
        &["/book/1".to_string(), "/book/2".to_string()]
    );
}

#[test]
fn legado_css_supports_current_attr_and_nested_pipelines() {
    let engine = RuleEngine::new();

    let hrefs = engine.execute_legado_css(HTML, "a&&@href").unwrap();
    assert_eq!(
        hrefs.values(),
        &[
            "/book/1".to_string(),
            "/book/2".to_string(),
            "/first".to_string(),
            "/second".to_string(),
        ]
    );

    let items = engine
        .execute_legado_css(HTML, "div.list&&div.item")
        .unwrap();
    assert_eq!(items.values(), &["First".to_string(), "Second".to_string()]);

    let names = engine
        .execute_legado_css(HTML, "div.list&&div.item;div.name&&a@text")
        .unwrap();
    assert_eq!(names.values(), &["First".to_string(), "Second".to_string()]);
}

#[test]
fn legado_css_empty_missing_and_no_match_fail_closed() {
    let engine = RuleEngine::new();

    assert!(engine.execute_legado_css(HTML, "").unwrap().is_empty());
    assert!(engine
        .execute_optional_legado_css(HTML, None)
        .unwrap()
        .is_empty());
    assert!(engine
        .execute_optional_legado_css(HTML, Some("a.missing@href"))
        .unwrap()
        .is_empty());
}

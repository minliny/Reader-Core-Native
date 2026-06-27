//! Regression tests for release blocker rb-legado-css-shorthand-selector.
//!
//! Legado 用 `class.X` / `tag.X` / `id.X` 作为 CSS 简写(对应 Legado
//! `AnalyzeByJSoup.kt` ElementsSingle 的 `beforeRule.split(".")` 分支):
//!   - `class.X` -> `getElementsByClass(X)` = CSS `.X`
//!   - `tag.X`   -> `getElementsByTag(X)`   = CSS `X`
//!   - `id.X`    -> `Evaluator.Id(X)`       = CSS `#X`
//!
//! 真实 Legado 书源(如 tests/fixtures/remote_source/legado_sudugu_vertical.json)
//! 大量使用 `@` 作为管道分隔符,例如 `tag.h3@tag.a@text` 表示:
//!   select `tag.h3` -> select `tag.a` -> extract `text`。
//!
//! 修复前 compile_legado_css_selector 把 `class.item` 直接喂给
//! `scraper::Selector::parse`,匹配到 `<class class="item">` 元素(错误),
//! 导致真实书源搜索/详情/目录全返回空。

use reader_rule::{LegadoCssExtraction, LegadoCssRule, LegadoCssStep, RuleEngine};

const HTML: &str = r#"
<html>
<body>
  <div id="list">
    <div class="item">
      <h3><a href="/first">First</a></h3>
      <p class="kind"><span>Adventure</span></p>
    </div>
    <div class="item">
      <h3><a href="/second">Second</a></h3>
      <p class="kind"><span>Romance</span></p>
    </div>
  </div>
  <div class="container">
    <div class="des bb">
      <p>Intro text</p>
    </div>
  </div>
</body>
</html>
"#;

#[test]
fn class_shorthand_translates_to_css_class_selector() {
    let engine = RuleEngine::new();

    // class.item -> .item,应选中两个 div.item,默认 text 抽取。
    // element_text 在 h3/p 等 block 边界插入空格,与 Legado jsoup text() 一致。
    let items = engine.execute_legado_css(HTML, "class.item").unwrap();
    assert_eq!(
        items.values(),
        &["First Adventure".to_string(), "Second Romance".to_string()]
    );
}

#[test]
fn id_shorthand_translates_to_css_id_selector() {
    let engine = RuleEngine::new();

    // id.list -> #list,默认 text 抽取。
    let list_text = engine.execute_legado_css(HTML, "id.list").unwrap();
    assert!(list_text.values().iter().any(|v| v.contains("First")));
    assert!(list_text.values().iter().any(|v| v.contains("Second")));
}

#[test]
fn tag_shorthand_translates_to_css_type_selector() {
    let engine = RuleEngine::new();

    // tag.h3 -> h3,默认 text 抽取。
    let h3_text = engine.execute_legado_css(HTML, "tag.h3").unwrap();
    assert_eq!(
        h3_text.values(),
        &["First".to_string(), "Second".to_string()]
    );
}

#[test]
fn at_pipeline_with_shorthand_selectors_and_text_extraction() {
    let engine = RuleEngine::new();

    // tag.h3@tag.a@text -> select h3 -> select a -> extract text
    // 真实 Legado 书源 search.name 规则: "tag.h3@tag.a@text"
    let names = engine
        .execute_legado_css(HTML, "tag.h3@tag.a@text")
        .unwrap();
    assert_eq!(names.values(), &["First".to_string(), "Second".to_string()]);
}

#[test]
fn at_pipeline_with_shorthand_selectors_and_attr_extraction() {
    let engine = RuleEngine::new();

    // tag.h3@tag.a@href -> select h3 -> select a -> extract href
    // 真实 Legado 书源 search.bookUrl 规则: "tag.h3@tag.a@href"
    let hrefs = engine
        .execute_legado_css(HTML, "tag.h3@tag.a@href")
        .unwrap();
    assert_eq!(
        hrefs.values(),
        &["/first".to_string(), "/second".to_string()]
    );
}

#[test]
fn mixed_shorthand_and_plain_css_in_at_pipeline() {
    let engine = RuleEngine::new();

    // class.item@tag.a@href -> select .item -> select a -> extract href
    let hrefs = engine
        .execute_legado_css(HTML, "class.item@tag.a@href")
        .unwrap();
    assert_eq!(
        hrefs.values(),
        &["/first".to_string(), "/second".to_string()]
    );
}

#[test]
fn parser_translates_at_pipeline_into_select_then_extract_ast() {
    // tag.h3@tag.a@text -> [Select("tag.h3"), Extract { selector: Some("tag.a"), Text }]
    let rule = LegadoCssRule::parse("tag.h3@tag.a@text").unwrap();
    assert_eq!(
        rule.steps(),
        &[
            LegadoCssStep::Select("tag.h3".to_string()),
            LegadoCssStep::Extract {
                selector: Some("tag.a".to_string()),
                extraction: LegadoCssExtraction::Text,
            },
        ]
    );
}

#[test]
fn parser_keeps_single_at_extraction_unchanged() {
    // a.book@text -> [Extract { selector: Some("a.book"), Text }]
    // (向后兼容:单 @ 仍是 selector@extraction)
    let rule = LegadoCssRule::parse("a.book@text").unwrap();
    assert_eq!(
        rule.steps(),
        &[LegadoCssStep::Extract {
            selector: Some("a.book".to_string()),
            extraction: LegadoCssExtraction::Text,
        }]
    );
}

#[test]
fn parser_keeps_leading_at_attr_extraction_unchanged() {
    // @href -> [Extract { selector: None, Attr("href") }]
    let rule = LegadoCssRule::parse("@href").unwrap();
    assert_eq!(
        rule.steps(),
        &[LegadoCssStep::Extract {
            selector: None,
            extraction: LegadoCssExtraction::Attr("href".to_string()),
        }]
    );
}

#[test]
fn shorthand_does_not_swallow_plain_css_selectors() {
    let engine = RuleEngine::new();

    // 普通 CSS 选择器仍应正常工作(不误翻译)。
    let plain = engine.execute_legado_css(HTML, "div.item@text").unwrap();
    assert_eq!(
        plain.values(),
        &["First Adventure".to_string(), "Second Romance".to_string()]
    );

    // a@href 仍是单 selector+attr 抽取。
    let hrefs = engine.execute_legado_css(HTML, "a@href").unwrap();
    assert_eq!(
        hrefs.values(),
        &["/first".to_string(), "/second".to_string()]
    );
}

const INDEX_HTML: &str = r#"
<html>
<body>
  <div class="info">
    <p class="kind"><span>Adventure</span></p>
    <p class="author"><a href="/author">AuthorName</a></p>
    <p class="status">Ongoing</p>
  </div>
  <ul>
    <li><a href="/c1">Chapter 1</a></li>
    <li><a href="/c2">Chapter 2</a></li>
    <li><a href="/c3">Chapter 3</a></li>
  </ul>
</body>
</html>
"#;

#[test]
fn legado_dot_index_selects_nth_element() {
    let engine = RuleEngine::new();

    // tag.p.1 -> select all <p>, take index 1 (second <p>)
    // 真实 Legado 书源 search.author 规则: "tag.p.1@tag.a@text##作者："
    let author = engine
        .execute_legado_css(INDEX_HTML, "tag.p.1@tag.a@text")
        .unwrap();
    assert_eq!(author.values(), &["AuthorName".to_string()]);

    // tag.p.0 -> first <p>
    let kind = engine
        .execute_legado_css(INDEX_HTML, "tag.p.0@tag.span@text")
        .unwrap();
    assert_eq!(kind.values(), &["Adventure".to_string()]);

    // tag.li.0 -> first <li>
    let first_chapter = engine
        .execute_legado_css(INDEX_HTML, "tag.li.0@tag.a@text")
        .unwrap();
    assert_eq!(first_chapter.values(), &["Chapter 1".to_string()]);
}

#[test]
fn legado_dot_index_supports_negative_indices() {
    let engine = RuleEngine::new();

    // tag.p.-1 -> last <p>
    let status = engine
        .execute_legado_css(INDEX_HTML, "tag.p.-1@text")
        .unwrap();
    assert_eq!(status.values(), &["Ongoing".to_string()]);

    // tag.li.-1 -> last <li>
    let last_chapter = engine
        .execute_legado_css(INDEX_HTML, "tag.li.-1@tag.a@text")
        .unwrap();
    assert_eq!(last_chapter.values(), &["Chapter 3".to_string()]);
}

#[test]
fn legado_dot_index_out_of_range_returns_empty() {
    let engine = RuleEngine::new();

    // tag.p.10 -> index 10 out of range (only 3 <p> elements)
    let missing = engine
        .execute_legado_css(INDEX_HTML, "tag.p.10@text")
        .unwrap();
    assert!(missing.values().is_empty());
}

#[test]
fn legado_dot_index_does_not_swallow_css_class_names() {
    let engine = RuleEngine::new();

    // div.item should NOT have ".item" parsed as an index (it's a CSS class).
    // 验证索引解析不会误吞非数字的 class 名。
    let items = engine.execute_legado_css(HTML, "div.item@text").unwrap();
    assert_eq!(
        items.values(),
        &["First Adventure".to_string(), "Second Romance".to_string()]
    );
}

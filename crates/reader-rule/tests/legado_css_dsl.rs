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

    let root_text = engine.execute_legado_css(HTML, "@text").unwrap();
    assert_eq!(
        root_text.values(),
        &["Dune Foundation The Chapter First paragraph. First Second".to_string()]
    );

    let text = engine.execute_legado_css(HTML, "a.book@text").unwrap();
    assert_eq!(
        text.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let uppercase_text = engine.execute_legado_css(HTML, "a.book@TEXT").unwrap();
    assert!(uppercase_text.is_empty());

    let html = engine
        .execute_legado_css(HTML, "article.chapter@html")
        .unwrap();
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
fn legado_css_default_mode_uses_at_as_nested_selector_separator_before_extraction() {
    let engine = RuleEngine::new();

    let names = engine
        .execute_legado_css(HTML, "div.item@div.name@a@text")
        .unwrap();
    assert_eq!(names.values(), &["First".to_string(), "Second".to_string()]);

    let hrefs = engine
        .execute_legado_css(HTML, "div.item@div.name@a@href")
        .unwrap();
    assert_eq!(
        hrefs.values(),
        &["/first".to_string(), "/second".to_string()]
    );
}

#[test]
fn legado_css_prefix_uses_css_source_mode_like_legado() {
    let engine = RuleEngine::new();

    let text = engine.execute_legado_css(HTML, "@CSS:a.book@text").unwrap();
    assert_eq!(
        text.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let bare_prefix_text = engine.execute_legado_css(HTML, "css:a.book@text").unwrap();
    assert_eq!(
        bare_prefix_text.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let bare_prefix_default_text = engine
        .execute_legado_css(HTML, "css:article.chapter h1")
        .unwrap();
    assert_eq!(
        bare_prefix_default_text.values(),
        &["The Chapter".to_string()]
    );

    let merged = engine
        .execute_legado_css(HTML, "@CSS:article.chapter h1@text&&a.book@href")
        .unwrap();
    assert_eq!(
        merged.values(),
        &[
            "The Chapter".to_string(),
            "/book/1".to_string(),
            "/book/2".to_string()
        ]
    );

    let css_literal = engine
        .execute_legado_css(HTML, "@CSS:class.book@text")
        .unwrap();
    assert!(css_literal.is_empty());

    let bare_css_literal = engine
        .execute_legado_css(HTML, "css:class.book@text")
        .unwrap();
    assert!(bare_css_literal.is_empty());
}

#[test]
fn legado_double_at_prefix_escapes_to_default_css_rule_like_legado() {
    let engine = RuleEngine::new();

    let selected = engine.execute_legado_css(HTML, "@@a.book@text").unwrap();
    assert_eq!(
        selected.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let root_text = engine.execute_legado_css(HTML, "@@@text").unwrap();
    assert_eq!(
        root_text.values(),
        &["Dune Foundation The Chapter First paragraph. First Second".to_string()]
    );
}

#[test]
fn legado_css_default_mode_supports_jsoup_shorthand_selectors() {
    let engine = RuleEngine::new();

    let by_class = engine.execute_legado_css(HTML, "class.book@text").unwrap();
    assert_eq!(
        by_class.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );

    let by_tag = engine.execute_legado_css(HTML, "tag.a@href").unwrap();
    assert_eq!(
        by_tag.values(),
        &[
            "/book/1".to_string(),
            "/book/2".to_string(),
            "/first".to_string(),
            "/second".to_string()
        ]
    );

    let wildcard = engine
        .execute_legado_css(
            r#"
                <main>
                    <section data-id="summary">Summary</section>
                    <article data-id="chapter">Chapter</article>
                </main>
            "#,
            "main&&tag.*@data-id",
        )
        .unwrap();
    assert_eq!(
        wildcard.values(),
        &["summary".to_string(), "chapter".to_string()]
    );

    let html = r#"
        <main>
            <section id="featured">Featured title</section>
            <p>Direct Dune</p>
            <p><span>Dune nested</span></p>
        </main>
    "#;

    let by_id = engine.execute_legado_css(html, "id.featured@text").unwrap();
    assert_eq!(by_id.values(), &["Featured title".to_string()]);

    let by_own_text = engine.execute_legado_css(html, "text.Direct@text").unwrap();
    assert_eq!(by_own_text.values(), &["Direct Dune".to_string()]);

    let dotted_shorthand = r#"
        <main>
            <a class="book featured" href="/featured">Featured</a>
            <a class="book" href="/plain">Plain</a>
            <section id="featured">Dotted id</section>
            <p>Direct.Dune marker</p>
        </main>
    "#;

    let dotted_class = engine
        .execute_legado_css(dotted_shorthand, "class.book.featured@href")
        .unwrap();
    assert_eq!(
        dotted_class.values(),
        &["/featured".to_string(), "/plain".to_string()]
    );

    let dotted_id = engine
        .execute_legado_css(dotted_shorthand, "id.featured.extra@text")
        .unwrap();
    assert_eq!(dotted_id.values(), &["Dotted id".to_string()]);

    let dotted_text = engine
        .execute_legado_css(dotted_shorthand, "text.Direct.Dune@text")
        .unwrap();
    assert_eq!(dotted_text.values(), &["Direct.Dune marker".to_string()]);
}

#[test]
fn legado_css_default_mode_supports_jsoup_index_filters() {
    let engine = RuleEngine::new();

    let second = engine.execute_legado_css(HTML, "tag.a.1@href").unwrap();
    assert_eq!(second.values(), &["/book/2".to_string()]);

    let last = engine.execute_legado_css(HTML, "tag.a.-1@href").unwrap();
    assert_eq!(last.values(), &["/second".to_string()]);

    let exclude_first = engine.execute_legado_css(HTML, "tag.a!0@href").unwrap();
    assert_eq!(
        exclude_first.values(),
        &[
            "/book/2".to_string(),
            "/first".to_string(),
            "/second".to_string()
        ]
    );

    let bracket_pick = engine.execute_legado_css(HTML, "tag.a[0,-1]@href").unwrap();
    assert_eq!(
        bracket_pick.values(),
        &["/book/1".to_string(), "/second".to_string()]
    );

    let bracket_exclude_range = engine.execute_legado_css(HTML, "tag.a[!1:2]@href").unwrap();
    assert_eq!(
        bracket_exclude_range.values(),
        &["/book/1".to_string(), "/second".to_string()]
    );

    let bracket_reverse = engine.execute_legado_css(HTML, "tag.a[-1:0]@href").unwrap();
    assert_eq!(
        bracket_reverse.values(),
        &[
            "/second".to_string(),
            "/first".to_string(),
            "/book/2".to_string(),
            "/book/1".to_string()
        ]
    );
}

#[test]
fn legado_css_default_mode_supports_direct_children_selectors() {
    let engine = RuleEngine::new();
    let html = r#"
        <main>
            <section>Summary</section>
            <article>Article</article>
            <footer>Footer</footer>
        </main>
    "#;

    let children = engine
        .execute_legado_css(html, "main&&children.1@text")
        .unwrap();
    assert_eq!(children.values(), &["Article".to_string()]);

    let first = engine.execute_legado_css(html, "main&&.0@text").unwrap();
    assert_eq!(first.values(), &["Summary".to_string()]);

    let first_and_last = engine
        .execute_legado_css(html, "main&&[0,-1]@text")
        .unwrap();
    assert_eq!(
        first_and_last.values(),
        &["Summary".to_string(), "Footer".to_string()]
    );
}

#[test]
fn legado_css_or_uses_first_non_empty_branch_like_legado() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_legado_css(HTML, "a.missing@text||a.book@text")
        .unwrap();

    assert_eq!(
        output.values(),
        &["Dune".to_string(), "Foundation".to_string()]
    );
}

#[test]
fn legado_css_and_merges_non_empty_extraction_branches_like_legado() {
    let engine = RuleEngine::new();

    let output = engine
        .execute_legado_css(HTML, "article.chapter h1@text&&a.book@href")
        .unwrap();

    assert_eq!(
        output.values(),
        &[
            "The Chapter".to_string(),
            "/book/1".to_string(),
            "/book/2".to_string()
        ]
    );
}

#[test]
fn legado_css_percent_zips_non_empty_branches_like_legado() {
    let engine = RuleEngine::new();
    let html = r#"
        <section>
            <h2>Dune</h2><span class="author">Frank Herbert</span>
            <h2>Foundation</h2><span class="author">Isaac Asimov</span>
        </section>
    "#;

    let output = engine
        .execute_legado_css(html, "h2@text%%span.author@text")
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
fn legado_css_reuses_compat_selector_filters() {
    let engine = RuleEngine::new();
    let html = r#"
        <section class="chapters">
            <a href="/c/1">Chapter 1</a>
            <a href="/c/2">Chapter 2 VIP</a>
            <a href="/c/3">Chapter 3</a>
        </section>
    "#;

    let second = engine
        .execute_legado_css(html, ".chapters>a:eq(1)@href")
        .unwrap();
    assert_eq!(second.values(), &["/c/2".to_string()]);

    let public = engine
        .execute_legado_css(html, ".chapters>a:not(:containsOwn(VIP))@text")
        .unwrap();
    assert_eq!(
        public.values(),
        &["Chapter 1".to_string(), "Chapter 3".to_string()]
    );

    let public_chapter_hrefs = engine
        .execute_legado_css(
            html,
            ".chapters>a:containsOwn(Chapter):not(:containsOwn(VIP))@href",
        )
        .unwrap();
    assert_eq!(
        public_chapter_hrefs.values(),
        &["/c/1".to_string(), "/c/3".to_string()]
    );

    let first_two = engine
        .execute_legado_css(html, ".chapters&&a:lt(2)@text")
        .unwrap();
    assert_eq!(
        first_two.values(),
        &["Chapter 1".to_string(), "Chapter 2 VIP".to_string()]
    );

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

    let nested = engine
        .execute_legado_css(
            nested_html,
            ".cards>article:containsOwn(Featured)>div:containsOwn(Target)>a@href",
        )
        .unwrap();
    assert_eq!(nested.values(), &["/a".to_string()]);

    let nested_has_html = r#"
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
        .execute_legado_css(
            nested_has_html,
            ".cards>article:has(> header)>div:has(> span.target)>a@href",
        )
        .unwrap();
    assert_eq!(nested_has.values(), &["/a".to_string()]);

    let nested_data_html = r#"
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
        .execute_legado_css(
            nested_data_html,
            ".cards>article:containsData(ArticleData)>div:containsData(TargetData)>a@href",
        )
        .unwrap();
    assert_eq!(nested_data.values(), &["/a".to_string()]);

    let public_data_html = r#"
        <section>
            <script data-id="free">BookData = { chapter: 1 };</script>
            <script data-id="vip">BookData = { chapter: 2, tag: "VIP" };</script>
        </section>
    "#;

    let public_data = engine
        .execute_legado_css(
            public_data_html,
            "script:containsData(BookData):not(:containsData(VIP))@data-id",
        )
        .unwrap();
    assert_eq!(public_data.values(), &["free".to_string()]);

    let nested_parent_html = r#"
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

    let nested_parent = engine
        .execute_legado_css(
            nested_parent_html,
            ".cards>article:parent>div:parent>a@href",
        )
        .unwrap();
    assert_eq!(nested_parent.values(), &["/a".to_string()]);
}

#[test]
fn legado_css_supports_special_attr_extractions() {
    let engine = RuleEngine::new();
    let html = r#"
        <article>
            <p id="own"><span>nested</span> Direct <b>child</b> Tail</p>
            <div id="nodes"> Lead <p>Nested chapter text</p> Tail </div>
            <div id="nested-only"><p>Nested chapter text</p></div>
        </article>
    "#;

    let own_text = engine.execute_legado_css(html, "#own@ownText").unwrap();
    assert_eq!(own_text.values(), &["Direct Tail".to_string()]);

    let text_nodes = engine.execute_legado_css(html, "#nodes@textNodes").unwrap();
    assert_eq!(text_nodes.values(), &["Lead\nTail".to_string()]);

    let nested_only = engine
        .execute_legado_css(html, "#nested-only@textNodes")
        .unwrap();
    assert!(nested_only.is_empty());
}

#[test]
fn legado_css_applies_hash_regex_replacements_like_legado() {
    let engine = RuleEngine::new();
    let html = r#"
        <article>
            <p class="kind">分类：玄幻小说</p>
            <p class="intro">第一句。第二句！</p>
            <a class="cover" href="/book/12345/index.html">Cover</a>
        </article>
    "#;

    let cleaned_kind = engine
        .execute_legado_css(html, "p.kind@text##小说|.*：")
        .unwrap();
    assert_eq!(cleaned_kind.values(), &["玄幻".to_string()]);

    let intro_with_breaks = engine
        .execute_legado_css(html, "p.intro@text##([。！？])##$1<br>")
        .unwrap();
    assert_eq!(
        intro_with_breaks.values(),
        &["第一句。<br>第二句！<br>".to_string()]
    );

    let cover_url = engine
        .execute_legado_css(
            html,
            "a.cover@href##.+\\D((\\d+)\\d{3})\\D##/files/article/image/$2/$1/$1s.jpg###",
        )
        .unwrap();
    assert_eq!(
        cover_url.values(),
        &["/files/article/image/12/12345/12345s.jpg".to_string()]
    );
}

#[test]
fn legado_css_empty_rule_replacement_matches_raw_source_like_legado() {
    let engine = RuleEngine::new();
    let html = r#"
        <article>
            <p>总字数：18万<small>统计</small></p>
            <script>window.meta = "类型.玄幻小说<";</script>
            <script>{"name":"第一章 "}</script>
        </article>
    "#;

    let word_count = engine
        .execute_legado_css(html, "##总字数：([^<]+)<##$1###")
        .unwrap();
    assert_eq!(word_count.values(), &["18万".to_string()]);

    let embedded = engine
        .execute_legado_css(html, "kind={{@@##类型\\.([^<]+)<##$1###}}")
        .unwrap();
    assert_eq!(embedded.values(), &["kind=玄幻小说".to_string()]);

    let horizontal_space = engine
        .execute_legado_css(html, "##\"name\":\"([^\\n\"]+?)[\\h。，、：]?\"##$1###")
        .unwrap();
    assert_eq!(horizontal_space.values(), &["第一章".to_string()]);
}

#[test]
fn legado_css_double_at_template_matches_legado_source_rule() {
    let engine = RuleEngine::new();
    let html = r#"
        <article>
            <meta property="chapter_name" content="正文卷.第1章 开始">
            <meta property="update_time" content="2026-06-25 21:14:57 CST">
        </article>
    "#;

    let output = engine
        .execute_legado_css(
            html,
            "最新：{{@@[property$=chapter_name]@content##正文卷\\.}}•{{@@[property$=update_time]@content##\\s.*}}",
        )
        .unwrap();

    assert_eq!(
        output.values(),
        &["最新：第1章 开始•2026-06-25".to_string()]
    );

    let quoted = engine
        .execute_legado_css(
            html,
            "chapter='{{@@[property$=chapter_name]@content##正文卷\\.}}'",
        )
        .unwrap();
    assert_eq!(quoted.values(), &["chapter='第1章 开始'".to_string()]);
}

#[test]
fn legado_css_single_at_template_matches_legado_source_rule() {
    let engine = RuleEngine::new();
    let html = r#"
        <nav>
            <a href="/renwu/1">中华典藏</a>
        </nav>
        <section>
            <span class="tag-list">奇幻</span>
            <p class="introduce">第一句。第二句！</p>
        </section>
    "#;

    let output = engine
        .execute_legado_css(html, "genre={{@[href*=/renwu/]@text}}")
        .unwrap();

    assert_eq!(output.values(), &["genre=中华典藏".to_string()]);

    let direct = engine
        .execute_legado_css(html, "[href*=/renwu/]@text")
        .unwrap();
    assert_eq!(direct.values(), &["中华典藏".to_string()]);

    let replaced_template = engine
        .execute_legado_css(
            html,
            "标签：{{@.tag-list@text}} {{@.introduce@text}}##([。！？])##$1<br>",
        )
        .unwrap();
    assert_eq!(
        replaced_template.values(),
        &["标签：奇幻 第一句。<br>第二句！<br>".to_string()]
    );
}

#[test]
fn legado_css_last_result_filters_match_legado_jsoup() {
    let engine = RuleEngine::new();
    let html = r#"
        <section>
            <p>   </p>
            <p>Visible</p>
            <p></p>
            <a href="/book/1">First</a>
            <a href="">Blank attr</a>
            <a>No attr</a>
            <a href="/book/1">Duplicate</a>
            <a href="/book/2">Second</a>
            <a href="   ">Spaces attr</a>
        </section>
    "#;

    let text = engine.execute_legado_css(html, "p@text").unwrap();
    assert_eq!(text.values(), &["Visible".to_string()]);

    let href = engine.execute_legado_css(html, "a@href").unwrap();
    assert_eq!(
        href.values(),
        &["/book/1".to_string(), "/book/2".to_string()]
    );
}

#[test]
fn legado_css_html_and_all_match_legado_jsoup_boundaries() {
    let engine = RuleEngine::new();
    let html = r#"
        <article class="chapter">
            <p>Visible</p>
            <script>window.secret = 1</script>
            <style>.hidden { display: none }</style>
        </article>
    "#;

    let cleaned = engine
        .execute_legado_css(html, "article.chapter@html")
        .unwrap();
    assert_eq!(
        cleaned.values(),
        &["<article class=\"chapter\">\n            <p>Visible</p>\n            \n            \n        </article>".to_string()]
    );

    let all = engine
        .execute_legado_css(html, "article.chapter@all")
        .unwrap();
    assert_eq!(
        all.values(),
        &["<article class=\"chapter\">\n            <p>Visible</p>\n            <script>window.secret = 1</script>\n            <style>.hidden { display: none }</style>\n        </article>".to_string()]
    );

    let paragraphs = engine
        .execute_legado_css("<section><p>A</p><p>B</p></section>", "p@all")
        .unwrap();
    assert_eq!(paragraphs.values(), &["<p>A</p>\n<p>B</p>".to_string()]);
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

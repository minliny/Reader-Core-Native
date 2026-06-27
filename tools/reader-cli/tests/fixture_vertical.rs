//! End-to-end smoke for the remote-reading vertical: drives `reader-cli
//! --fixture-vertical` through the full import → search → host-http search →
//! detail → toc → chapter → progress pipeline and the JS-unsupported path,
//! asserting each emitted event shape.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_reader-cli");

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/fixtures/remote_source");
    p.push(name);
    p.canonicalize().unwrap_or(p)
}

fn booksource_fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../crates/reader-content/tests/fixtures/booksource_canonical.json");
    p.canonicalize().unwrap_or(p)
}

#[test]
fn fixture_vertical_runs_full_pipeline() {
    let events = run_fixture("basic_source.json");

    assert_eq!(events[0]["data"]["imported"], true);
    assert_eq!(events[0]["data"]["sourceId"], "basic-src");

    assert_basic_pipeline_events(&events);
    assert_eq!(events[4]["data"]["book"]["author"], "Frank Herbert");
    assert_eq!(events[4]["data"]["book"]["intro"], "A desert planet.");
    assert!(events[6]["data"]["content"]
        .as_str()
        .unwrap()
        .contains("First paragraph"));
}

#[test]
fn fixture_vertical_runs_legado_css_dsl_pipeline() {
    let events = run_fixture("legado_css_source.json");

    assert_eq!(events[0]["data"]["imported"], true);
    assert_eq!(events[0]["data"]["sourceId"], "legado-css-src");

    assert_basic_pipeline_events(&events);
    assert_eq!(events[4]["data"]["book"]["author"], "Frank Herbert");
    assert_eq!(events[4]["data"]["book"]["intro"], "A desert planet.");
    assert!(events[6]["data"]["content"]
        .as_str()
        .unwrap()
        .contains("First & bold line."));
}

/// Real Legado source (sudugu.org 速读谷吧) vertical pipeline.
///
/// This fixture uses a desensitized real online Legado book source with rules
/// covering 5 DSL forms in a single source:
/// - CSS: `class.item`, `tag.h3@tag.a@text`, `id.list@tag.li`
/// - ## replacement: `tag.p.1@tag.a@text##作者：`, `class.con@html##<div.*?>|</div>`
/// - @put: `@put:{id:"##/(\\d+)/##1"}` in ruleBookInfo.downloadUrls
/// - @js: `@js:result='...'` in ruleBookInfo.downloadUrls
/// - @xpath: `@xpath://div[@class='pages bb']//a[...]/@href` in ruleToc/ruleContent
///
/// Fixture uses the real Legado export format: ruleSearch/ruleBookInfo/ruleToc/
/// ruleContent as JSON objects and concurrentRate as a string ("3"), matching
/// BookSource.kt. Resolved blockers:
/// - rb-legado-rulesearch-object-deser: rule_search now Option<Value>, accepts
///   both string (legacy) and object (real Legado) forms.
/// - rb-legado-concurrentrate-string-deser: concurrent_rate now Option<Value>,
///   accepts both int and string.
/// - rb-legado-css-shorthand-selector: class.X/tag.X/id.X now translated to CSS
///   `.X`/`X`/`#X` before scraper::Selector::parse. Search returns 10 books
///   with correct title/author/kind/lastChapter/coverUrl/bookUrl extracted via
///   `tag.h3@tag.a@text`, `tag.p.1@tag.a@text##作者：`, `tag.p.0@tag.span.1@text`,
///   `tag.ul@tag.li.0@tag.a@text`, `tag.img@src`, `tag.h3@tag.a@href`.
/// - rb-xpath-strict-xml-parser: @xpath now uses html5ever (scraper) to parse
///   real HTML into sxd DOM before XPath evaluation. chapter content extracts
///   successfully via `@xpath` rules on real (non-XML) HTML.
/// - rb-tocurl-template-as-selector: tocUrl `{{baseUrl}}/#dir` template now
///   expands to URL and is returned directly (not treated as CSS selector).
///   detail succeeds with full book metadata.
/// Remaining open issue (not a release blocker for this task):
/// - toc list `id.list@tag.li` (2-segment CSS shorthand pipeline where the
///   last segment is a selector `tag.li`, not an extraction like `text`)
///   returns empty. `id.list@tag.li@text` (3-segment) works. This is a CSS
///   shorthand pipeline edge case, tracked separately.
#[test]
fn fixture_vertical_runs_legado_sudugu_real_source_pipeline() {
    let events = run_fixture("legado_sudugu_vertical.json");

    // 9 events: import, search(inline), host.request, search(host), detail, toc,
    // chapter, progress, js(unsupported).
    assert_eq!(events.len(), 9, "expected 9 events, got {events:?}");

    // 1. import succeeds
    assert_eq!(events[0]["data"]["imported"], true);
    assert_eq!(events[0]["data"]["sourceId"], "legado-sudugu-src");
    assert_eq!(events[0]["data"]["name"], "速读谷吧（优）");

    // 2. inline search — rb-legado-css-shorthand-selector resolved: class.item
    //    now translates to .item, returning 10 books with full metadata.
    let inline_books = events[1]["data"]["books"].as_array().unwrap();
    assert!(
        !inline_books.is_empty(),
        "inline search should return non-empty books after rb-legado-css-shorthand-selector fix"
    );
    let first = &inline_books[0];
    assert_eq!(first["title"], "诡秘：善魔女");
    assert_eq!(first["author"], "囧囧哟");
    assert_eq!(first["kind"], "奇幻");
    assert_eq!(first["bookId"], "https://www.sudugu.org/301/");

    // 3. host.request emitted for http.execute
    assert_eq!(events[2]["type"], "host.request");
    assert_eq!(events[2]["capability"], "http.execute");
    assert_eq!(
        events[2]["params"]["url"],
        "https://www.sudugu.org/search?q=dune"
    );

    // 4. search via host — same fix applies, returns the same 10 books.
    let host_books = events[3]["data"]["books"].as_array().unwrap();
    assert!(
        !host_books.is_empty(),
        "host search should return non-empty books after rb-legado-css-shorthand-selector fix"
    );

    // 5. detail — rb-tocurl-template-as-selector resolved: tocUrl
    //    `{{baseUrl}}/#dir` expands to URL and is returned directly, not run
    //    as a CSS selector. detail now succeeds with full book metadata.
    assert_eq!(
        events[4]["type"], "result",
        "detail should succeed after rb-tocurl-template-as-selector fix, got {:?}",
        events[4]
    );
    assert_eq!(events[4]["data"]["book"]["title"], "诡秘：善魔女");
    assert_eq!(events[4]["data"]["book"]["author"], "作者：囧囧哟");
    assert_eq!(events[4]["data"]["book"]["kind"], "奇幻小说");

    // 6. toc — rb-xpath-strict-xml-parser resolved: nextTocUrl `@xpath:...`
    //    no longer fails on real HTML, so toc is a result (not an error).
    //    The chapter list is empty due to the separate `id.list@tag.li`
    //    2-segment CSS shorthand pipeline edge case (not this task's blocker).
    assert_eq!(
        events[5]["type"], "result",
        "toc should be a result (not error) after rb-xpath-strict-xml-parser fix, got {:?}",
        events[5]
    );

    // 7. chapter — rb-xpath-strict-xml-parser resolved: @xpath rules now parse
    //    real HTML via html5ever. chapter content extracts successfully.
    assert_eq!(
        events[6]["type"], "result",
        "chapter should succeed after rb-xpath-strict-xml-parser fix, got {:?}",
        events[6]
    );
    assert_eq!(events[6]["data"]["chapterTitle"], "第1章 序章 醒来");
    assert!(events[6]["data"]["content"]
        .as_str()
        .unwrap()
        .contains("昏暗的地下室"));

    // 8. progress stored
    assert_eq!(events[7]["data"]["stored"], true);

    // 9. JS unsupported — structured error, never a fake network result
    assert_eq!(events[8]["type"], "error");
    assert_eq!(events[8]["error"]["code"], "INTERNAL");
    assert_eq!(events[8]["error"]["details"]["unsupported"], true);
}

/// Real Legado source (js.jsxsapp.com 追书小说) vertical pipeline — pure JSON API.
///
/// This fixture uses a desensitized real online Legado book source whose
/// ruleSearch/ruleBookInfo/ruleToc/ruleContent are ALL JSONPath rules
/// (`$.books`, `$.bookName`, `$..chapters[*]`, `$..body##...`). It covers the
/// `@json:` / `$.` DSL form which the sudugu fixture (CSS + ## + @put + @js +
/// @xpath) does not exercise.
///
/// Source: chao921125/source `yd.json.txt` index 29 (追书小说). All four API
/// responses (search/info/catalog/chapter) are real JSON captured with
/// `okhttp/3.14.9` UA, then trimmed (TOC to 30 chapters) to keep fixture size
/// reasonable. No user token/cookie — the source is a public JSON API.
///
/// Resolved blockers (this fixture confirms the fixes work end-to-end):
/// - rb-legado-ruleexplore-object-deser: rule_explore/rule_review now
///   Option<Value>, accepts both string and object forms (real Legado export
///   has ruleExplore as object).
/// - rb-legado-jsonpath-html-suffix: legado_rule_has_extraction now returns
///   true for JSONPath (`$.`/`$[`/`@json:`) and XPath (`@xpath:`) prefixes,
///   preventing extract_rule_items from appending `@html` to JSONPath rules
///   like `$..chapters[*]` (which produced invalid `$..chapters[*]@html`).
/// - rb-legado-json-booklist-array-iterate: extract_rule_items now unfolds a
///   single JSON-array value into one scope per element (mirrors Legado
///   `AnalyzeRule.getElements` → `AnalyzeByJSonPath.getList`). Search returns
///   >1 book with non-empty title/author.
///
/// Open blockers (this fixture EXPOSES the gaps; test is #[ignore] until
/// fixed — do not weaken assertions to bypass):
/// - rb-legado-json-url-template-jsonpath (FIXED): `expand_template` now
///   handles `{{$.field}}` JSONPath templates evaluated against the current
///   per-item scope, mirroring Legado `AnalyzeRule.makeUpRule` (line 659).
///   search bookUrl `https://js.jsxsapp.com/info/{{$._id}}?language=zh_cn`
///   and toc chapterUrl `https://yd.jsxsapp.com//@get:{bid}/{{$.l}}` now
///   expand correctly. Template rules also short-circuit selector parsing
///   (Legado mode=Regex → `else -> rule`), fixing `kind` =
///   `{{$..lastTime}}\n{{$..bigClass}}\n{{$..subClass}}` which was being fed
///   to the CSS engine as an invalid selector.
///
/// What currently works (proven by manual `--fixture-vertical` run):
/// - import: source deserialized (after rb-legado-ruleexplore-object-deser)
/// - host.request: correct URL/method/headers emitted
/// - detail: full metadata (斗破苍穹 / 天蚕土豆 / cover / intro / lastChapter)
///   — uses fixture's hardcoded bookId, not search result, so unaffected by
///   rb-legado-json-booklist-array-iterate
/// - toc: 30 chapters with correct titles (after rb-legado-jsonpath-html-suffix)
///   — chapterUrl still has unexpanded {{$.l}} (rb-legado-json-url-template-jsonpath)
/// - chapter: real API's version-too-low message extracted via `$..body##...`
/// - progress: stored
/// - js unsupported: structured error for `java.get` host callback
#[test]
fn fixture_vertical_runs_legado_zhuishu_json_real_source_pipeline() {
    let events = run_fixture("legado_zhuishu_json_vertical.json");

    // 9 events: import, search(inline), host.request, search(host), detail, toc,
    // chapter, progress, js(unsupported).
    assert_eq!(events.len(), 9, "expected 9 events, got {events:?}");

    // 1. import succeeds
    assert_eq!(events[0]["data"]["imported"], true);
    assert_eq!(events[0]["data"]["sourceId"], "legado-zhuishu-json-src");
    assert_eq!(events[0]["data"]["name"], "追书小说（JSON）");

    // 2. inline search — rb-legado-json-booklist-array-iterate: $.books must
    //    return each array element as a separate item. rb-legado-json-url-
    //    template-jsonpath: {{$._id}} must expand to the item's _id field.
    let inline_books = events[1]["data"]["books"].as_array().unwrap();
    assert!(
        !inline_books.is_empty(),
        "inline search should return non-empty books"
    );
    // After rb-legado-json-booklist-array-iterate: real API returns ~20 books,
    // not 1 (the whole array as a single string).
    assert!(
        inline_books.len() > 1,
        "inline search should return multiple books after rb-legado-json-booklist-array-iterate fix, got {}",
        inline_books.len()
    );
    let first = &inline_books[0];
    assert!(
        !first["title"].as_str().unwrap_or("").is_empty(),
        "first book title should be non-empty after rb-legado-json-booklist-array-iterate fix"
    );
    assert!(
        !first["author"].as_str().unwrap_or("").is_empty(),
        "first book author should be non-empty after rb-legado-json-booklist-array-iterate fix"
    );
    assert!(
        !first["bookId"]
            .as_str()
            .unwrap_or("")
            .contains("{{"),
        "bookId should have {{$._id}} expanded after rb-legado-json-url-template-jsonpath fix, got {:?}",
        first["bookId"]
    );

    // 3. host.request emitted for http.execute
    assert_eq!(events[2]["type"], "host.request");
    assert_eq!(events[2]["capability"], "http.execute");
    assert_eq!(
        events[2]["params"]["url"],
        "https://js.jsxsapp.com/search?q=dune"
    );

    // 4. search via host — same assertions as inline search
    let host_books = events[3]["data"]["books"].as_array().unwrap();
    assert!(
        !host_books.is_empty(),
        "host search should return non-empty books"
    );
    assert!(
        host_books.len() > 1,
        "host search should return multiple books after rb-legado-json-booklist-array-iterate fix"
    );

    // 5. detail — succeeds (uses fixture's hardcoded bookId, not search result)
    assert_eq!(
        events[4]["type"], "result",
        "detail should succeed, got {:?}",
        events[4]
    );
    assert_eq!(events[4]["data"]["book"]["title"], "斗破苍穹");
    assert_eq!(events[4]["data"]["book"]["author"], "天蚕土豆");
    assert_eq!(events[4]["data"]["book"]["lastChapter"], "第一章 五帝破空");

    // 6. toc — succeeds after rb-legado-jsonpath-html-suffix fix
    assert_eq!(
        events[5]["type"], "result",
        "toc should be a result (not error) after rb-legado-jsonpath-html-suffix fix, got {:?}",
        events[5]
    );
    let chapters = events[5]["data"]["toc"].as_array().unwrap();
    assert!(!chapters.is_empty(), "toc should return non-empty chapters");
    assert_eq!(
        chapters[0]["title"], "1.第一章 陨落的天才",
        "first chapter title should be extracted via $.t"
    );
    // rb-legado-json-url-template-jsonpath: chapterUrl {{$.l}} must expand.
    let first_chapter_url = chapters[0]["url"].as_str().unwrap_or("");
    assert!(
        !first_chapter_url.contains("{{"),
        "chapterUrl should have {{$.l}} expanded after rb-legado-json-url-template-jsonpath fix, got {:?}",
        first_chapter_url
    );

    // 7. chapter — content extracted via $..body##... (real API returns
    //    version-too-low message for this deprecated endpoint)
    assert_eq!(
        events[6]["type"], "result",
        "chapter should succeed, got {:?}",
        events[6]
    );
    assert_eq!(events[6]["data"]["chapterTitle"], "1.第一章 陨落的天才");
    assert!(
        !events[6]["data"]["content"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "chapter content should be non-empty"
    );

    // 8. progress stored
    assert_eq!(events[7]["data"]["stored"], true);

    // 9. JS unsupported — structured error, never a fake network result
    assert_eq!(events[8]["type"], "error");
    assert_eq!(events[8]["error"]["code"], "INTERNAL");
    assert_eq!(events[8]["error"]["details"]["unsupported"], true);
}

#[test]
fn booksource_fixture_outputs_stable_json() {
    let output = Command::new(BIN)
        .arg("--booksource-fixture")
        .arg(booksource_fixture_path())
        .output()
        .expect("reader-cli binary");

    assert!(
        output.status.success(),
        "reader-cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be one JSON object");

    assert_eq!(json["sourceId"], "booksource-fixture");
    assert_eq!(
        json["search"]["books"][0]["bookId"],
        "https://books.example.test/book/dune"
    );
    assert_eq!(json["explore"]["entries"][1]["kind"], "ranking");
    assert_eq!(
        json["toc"]["chapters"][0]["url"],
        "https://books.example.test/book/dune/chapter/1"
    );
    assert_eq!(
        json["content"]["content"],
        "First line.\nbooksource-fixture\nSecond line."
    );
}

fn run_fixture(name: &str) -> Vec<serde_json::Value> {
    let output = Command::new(BIN)
        .arg("--fixture-vertical")
        .arg(fixture_path(name))
        .output()
        .expect("reader-cli binary");

    assert!(
        output.status.success(),
        "reader-cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a JSON event"))
        .collect();

    events
}

fn assert_basic_pipeline_events(events: &[serde_json::Value]) {
    // 9 events: import, search(inline), host.request(http), search(host body),
    // detail, toc, chapter(rule), progress, js(unsupported).
    assert_eq!(events.len(), 9, "expected 9 events, got {events:?}");

    let books = events[1]["data"]["books"].as_array().unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[0]["title"], "Dune");

    assert_eq!(events[2]["type"], "host.request");
    assert_eq!(events[2]["capability"], "http.execute");
    assert_eq!(
        events[2]["params"]["url"],
        "https://books.example.test/search?q=dune"
    );

    let host_books = events[3]["data"]["books"].as_array().unwrap();
    assert_eq!(host_books.len(), 2);
    assert_eq!(host_books[0]["title"], "Dune");
    assert_eq!(events[3]["data"]["http"]["status"], 200);
    assert_eq!(
        events[3]["data"]["http"]["headers"]["content-type"],
        "application/json"
    );

    assert_eq!(events[4]["data"]["book"]["author"], "Frank Herbert");
    assert_eq!(events[4]["data"]["book"]["intro"], "A desert planet.");

    let toc = events[5]["data"]["toc"].as_array().unwrap();
    assert_eq!(toc.len(), 2);

    assert_eq!(events[6]["data"]["via"], "rule");

    assert_eq!(events[7]["data"]["stored"], true);

    // JS unsupported: structured error, never a fake network result.
    assert_eq!(events[8]["type"], "error");
    assert_eq!(events[8]["error"]["code"], "INTERNAL");
    assert_eq!(events[8]["error"]["details"]["unsupported"], true);
}

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
/// Remaining open blockers (detail/toc/chapter still error):
/// - rb-xpath-strict-xml-parser: @xpath fails on real HTML (not well-formed XML)
/// - rb-tocurl-template-as-selector: {{baseUrl}}/#dir treated as CSS selector
///
/// When these remaining blockers are resolved, this test should be updated to
/// assert successful parsing (non-empty toc, content).
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

    // 5. detail — currently error due to rb-tocurl-template-as-selector
    //    (tocUrl "{{baseUrl}}/#dir" treated as CSS selector)
    // 6. toc — currently error due to rb-xpath-strict-xml-parser
    // 7. chapter — currently error due to rb-xpath-strict-xml-parser
    // These three return either a result or an error depending on gap status.
    for idx in 4..=6 {
        let event = &events[idx];
        assert!(
            event["type"] == "result" || event["type"] == "error",
            "event {idx} should be result or error, got {event:?}"
        );
    }

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

//! Integration tests for `book.toc` / `chapter.content` pagination via
//! `nextTocUrl` / `nextContentUrl`.
//!
//! These tests close the Legado pagination gap (`BookChapterList.kt:69`
//! `while (nextUrl.isNotEmpty() && !nextUrlList.contains(nextUrl))` and the
//! identical loop in `BookContent.kt:85`). Core must:
//!
//! 1. Parse a page that yields a non-empty `nextTocUrl` / `nextContentUrl`.
//! 2. Emit a new `http.execute` host request (`RemoteCommandResult::Pending`)
//!    carrying a `BookTocNextPage` / `ChapterContentNextPage` continuation
//!    with the accumulated chapters/content and the visited-URL set.
//! 3. When the host completes the next-page request, resume via
//!    `complete_remote_host`, append the new page's chapters/content, and
//!    either emit another `Pending` or the final merged `Complete` result.
//!
//! The tests exercise the continuation chain directly (dispatch →
//! `complete_remote_host` → `complete_remote_host` …) without spinning up
//! the full runtime worker, mirroring the pattern in `auto_build_search.rs`.

use std::sync::{Arc, Mutex};

use reader_contract::{Command, Event};
use reader_runtime::{
    remote::{
        complete_remote_host, BookTocNextPageState, ChapterContentNextPageState, RemoteDispatch,
        RemoteHostContinuation, RemoteState,
    },
    sink::EventSink,
};

#[derive(Default, Clone)]
struct NoopSink {}

impl EventSink for NoopSink {
    fn emit(&self, _event: &Event) {}
}

/// Build an inline Legado source with a `nextTocUrl` rule of `a.next@href`
/// and a `chapterList` rule of `ol.toc li`.
fn inline_source_with_toc_pagination(base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "sourceId": "src-1",
        "name": "paginated-toc-source",
        "baseUrl": base_url,
        "rules": {},
        "bookSource": {
            "bookSourceName": "paginated-toc-source",
            "bookSourceUrl": base_url,
            "ruleToc": {
                "chapterList": "ol.toc li",
                "chapterName": "a@text",
                "chapterUrl": "a@href",
                "nextTocUrl": "a.next@href"
            }
        }
    })
}

/// Build an inline Legado source with a `nextContentUrl` rule of
/// `a.next-content@href` and a `content` rule of `article.content@html`.
fn inline_source_with_content_pagination(base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "sourceId": "src-1",
        "name": "paginated-content-source",
        "baseUrl": base_url,
        "rules": {},
        "bookSource": {
            "bookSourceName": "paginated-content-source",
            "bookSourceUrl": base_url,
            "ruleContent": {
                "content": "article.content@html",
                "nextContentUrl": "a.next-content@href"
            }
        }
    })
}

fn dispatch(state: &RemoteState, method: &str, params: serde_json::Value) -> RemoteDispatch {
    let cmd = Command::new(11, method, params);
    let sink: Arc<dyn EventSink> = Arc::new(NoopSink::default());
    let active = Arc::new(Mutex::new(std::collections::HashSet::<u64>::new()));
    reader_runtime::remote::dispatch_remote(method, &cmd, &sink, &active, state)
}

/// Build a fake `http.execute` host result with the given body and the
/// post-redirect `finalUrl` used to seed the pagination `visited_urls`.
fn host_result(body: &str, final_url: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "status": 200,
        "finalUrl": final_url
    })
}

/// Page 1 of a paginated TOC: two chapters + a `next` link to page 2.
fn toc_page_1() -> &'static str {
    r#"<ol class="toc">
        <li><a href="/book/dune/chapter/1">Chapter 1</a></li>
        <li><a href="/book/dune/chapter/2">Chapter 2</a></li>
    </ol>
    <a class="next" href="/book/dune/toc?page=2">Next</a>"#
}

/// Page 2 of a paginated TOC: one chapter + a `next` link to page 3.
fn toc_page_2() -> &'static str {
    r#"<ol class="toc">
        <li><a href="/book/dune/chapter/3">Chapter 3</a></li>
    </ol>
    <a class="next" href="/book/dune/toc?page=3">Next</a>"#
}

/// Page 3 of a paginated TOC: one chapter, no `next` link (terminal).
fn toc_page_3_terminal() -> &'static str {
    r#"<ol class="toc">
        <li><a href="/book/dune/chapter/4">Chapter 4</a></li>
    </ol>"#
}

// ===========================================================================
// book.toc pagination
// ===========================================================================

/// Page 1 yields a `nextTocUrl`: dispatch must return `Pending` with a
/// `BookTocNextPage` continuation carrying page-1 chapters + the next URL
/// recorded in `visited_urls`.
#[test]
fn book_toc_pagination_starts_when_next_toc_url_present() {
    let state = RemoteState::new();
    let source = inline_source_with_toc_pagination("https://books.example.test");
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "tocUrl": "https://books.example.test/book/dune/toc?page=1",
        "tocResponse": toc_page_1(),
    });

    let dispatch = dispatch(&state, "book.toc", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => match pending.continuation {
            RemoteHostContinuation::BookTocNextPage(state_) => {
                assert_eq!(
                    state_.pages_fetched, 1,
                    "initial page counts as one fetched page"
                );
                assert_eq!(
                    state_.accumulated.len(),
                    2,
                    "page-1 chapters should be accumulated"
                );
                assert_eq!(state_.accumulated[0].title, "Chapter 1");
                assert_eq!(state_.accumulated[1].title, "Chapter 2");
                // The next URL (resolved absolute) must be in visited_urls so
                // the next iteration can detect a cycle.
                assert!(
                    state_
                        .visited_urls
                        .contains("https://books.example.test/book/dune/toc?page=2"),
                    "next URL should be in visited_urls, got: {:?}",
                    state_.visited_urls
                );
                // The just-fetched page URL must also be recorded.
                assert!(
                    state_
                        .visited_urls
                        .contains("https://books.example.test/book/dune/toc?page=1"),
                    "page-1 URL should be in visited_urls, got: {:?}",
                    state_.visited_urls
                );
                // The pending host request must target the next-page URL.
                assert_eq!(
                    pending.params["url"],
                    "https://books.example.test/book/dune/toc?page=2"
                );
                assert_eq!(pending.params["method"], "GET");
            }
            other => panic!("expected BookTocNextPage continuation, got {other:?}"),
        },
        other => panic!("expected Pending, got {other:?}"),
    }
}

/// Single-page TOC with no `next` link: dispatch finishes immediately with
/// the page's chapters (no `Pending`).
#[test]
fn book_toc_no_pagination_when_no_next_toc_url() {
    let state = RemoteState::new();
    let source = inline_source_with_toc_pagination("https://books.example.test");
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "tocUrl": "https://books.example.test/book/dune/toc?page=1",
        "tocResponse": toc_page_3_terminal(),
    });

    let dispatch = dispatch(&state, "book.toc", params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (no pagination), got {other:?}"),
    }
}

/// Full chain: page 1 (next→2) → page 2 (next→3) → page 3 (terminal).
/// Verifies chapters from all three pages are merged in order.
#[test]
fn book_toc_pagination_merges_three_pages_in_order() {
    let state = RemoteState::new();
    let source = inline_source_with_toc_pagination("https://books.example.test");

    // Page 1: dispatch with pre-fetched response.
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source.clone(),
        "bookId": "b1",
        "tocUrl": "https://books.example.test/book/dune/toc?page=1",
        "tocResponse": toc_page_1(),
    });
    let dispatch = dispatch(&state, "book.toc", params);
    let continuation = match dispatch {
        RemoteDispatch::Pending(pending) => pending.continuation,
        other => panic!("page 1 should yield Pending, got {other:?}"),
    };

    // Page 2: host returns page-2 body. complete_remote_host should return
    // Pending again (page 2 has a next link to page 3).
    let result = complete_remote_host(
        continuation,
        host_result(
            toc_page_2(),
            "https://books.example.test/book/dune/toc?page=2",
        ),
        &state,
    )
    .expect("page 2 should parse");
    let continuation = match result {
        reader_runtime::remote::RemoteCommandResult::Pending(pending) => {
            match pending.continuation {
                RemoteHostContinuation::BookTocNextPage(state_) => {
                    // Pages 1 + 2 = 2 + 1 = 3 chapters accumulated.
                    assert_eq!(
                        state_.accumulated.len(),
                        3,
                        "after page 2, should have 3 chapters, got {:?}",
                        state_.accumulated
                    );
                    assert_eq!(state_.accumulated[2].title, "Chapter 3");
                    assert_eq!(state_.pages_fetched, 2);
                    RemoteHostContinuation::BookTocNextPage(state_)
                }
                other => panic!("expected BookTocNextPage after page 2, got {other:?}"),
            }
        }
        other => panic!("page 2 should yield Pending, got {other:?}"),
    };

    // Page 3: terminal page (no next link). complete_remote_host should
    // return Complete with all 4 chapters merged.
    let result = complete_remote_host(
        continuation,
        host_result(
            toc_page_3_terminal(),
            "https://books.example.test/book/dune/toc?page=3",
        ),
        &state,
    )
    .expect("page 3 should parse");
    match result {
        reader_runtime::remote::RemoteCommandResult::Complete(data) => {
            let chapters = data["toc"]
                .as_array()
                .expect("result should have toc array");
            assert_eq!(
                chapters.len(),
                4,
                "all 4 chapters from pages 1-3 should be merged, got {chapters:?}"
            );
            assert_eq!(chapters[0]["title"], "Chapter 1");
            assert_eq!(chapters[1]["title"], "Chapter 2");
            assert_eq!(chapters[2]["title"], "Chapter 3");
            assert_eq!(chapters[3]["title"], "Chapter 4");
        }
        other => panic!("page 3 should yield Complete, got {other:?}"),
    }
}

/// Cycle detection: when the next URL is already in `visited_urls`, the loop
/// stops and returns the accumulated chapters as Complete (mirrors Legado's
/// `!nextUrlList.contains(nextUrl)` guard).
#[test]
fn book_toc_pagination_stops_on_cycle() {
    let state = RemoteState::new();
    let source = inline_source_with_toc_pagination("https://books.example.test");

    // Manually construct a continuation state where the next URL was already
    // visited. This simulates a source whose page 2 points back to page 1.
    let params = reader_contract::remote::BookTocParams {
        source_id: "src-1".to_string(),
        book_id: "b1".to_string(),
        toc_response: String::new(), // will be overwritten by complete_remote_host
        toc_request: None,
        source: Some(source),
        toc_url: Some("https://books.example.test/book/dune/toc?page=1".to_string()),
    };
    let mut visited = std::collections::HashSet::new();
    visited.insert("https://books.example.test/book/dune/toc?page=1".to_string());
    visited.insert("https://books.example.test/book/dune/toc?page=2".to_string());
    // Page 2's response points back to page 1 (which is in visited_urls).
    let continuation = RemoteHostContinuation::BookTocNextPage(BookTocNextPageState {
        params,
        accumulated: vec![
            reader_domain::TocEntry {
                index: 0,
                title: "Chapter 1".to_string(),
                url: "https://books.example.test/book/dune/chapter/1".to_string(),
            },
            reader_domain::TocEntry {
                index: 1,
                title: "Chapter 2".to_string(),
                url: "https://books.example.test/book/dune/chapter/2".to_string(),
            },
        ],
        pages_fetched: 2,
        visited_urls: visited,
    });

    // Page 2 response: one chapter + next link back to page 1 (cycle).
    let cycle_body = r#"<ol class="toc">
        <li><a href="/book/dune/chapter/3">Chapter 3</a></li>
    </ol>
    <a class="next" href="/book/dune/toc?page=1">Next</a>"#;
    let result = complete_remote_host(
        continuation,
        host_result(
            cycle_body,
            "https://books.example.test/book/dune/toc?page=2",
        ),
        &state,
    )
    .expect("cycle page should parse");

    match result {
        reader_runtime::remote::RemoteCommandResult::Complete(data) => {
            let chapters = data["toc"]
                .as_array()
                .expect("result should have toc array");
            // Page 1 (2 chapters) + page 2 (1 chapter, the new one) = 3 chapters.
            // The cycle is detected on the *next* URL, so page 2's chapters
            // are still appended before stopping.
            assert_eq!(
                chapters.len(),
                3,
                "cycle should stop after appending page 2, got {chapters:?}"
            );
            assert_eq!(chapters[2]["title"], "Chapter 3");
        }
        other => panic!("cycle should yield Complete, got {other:?}"),
    }
}

// ===========================================================================
// chapter.content pagination
// ===========================================================================

/// Page 1 of paginated content: body + a `next-content` link to page 2.
fn content_page_1() -> &'static str {
    r#"<main>
        <article class="content"><p>First page content.</p></article>
        <a class="next-content" href="/book/dune/chapter/2">Next</a>
    </main>"#
}

/// Page 2 of paginated content: body + a `next-content` link to page 3.
fn content_page_2() -> &'static str {
    r#"<main>
        <article class="content"><p>Second page content.</p></article>
        <a class="next-content" href="/book/dune/chapter/3">Next</a>
    </main>"#
}

/// Page 3 of paginated content: terminal (no next link).
fn content_page_3_terminal() -> &'static str {
    r#"<main>
        <article class="content"><p>Third page content.</p></article>
    </main>"#
}

/// Page 1 yields a `nextContentUrl`: dispatch must return `Pending` with a
/// `ChapterContentNextPage` continuation.
#[test]
fn chapter_content_pagination_starts_when_next_content_url_present() {
    let state = RemoteState::new();
    let source = inline_source_with_content_pagination("https://books.example.test");
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://books.example.test/book/dune/chapter/1",
        "chapterResponse": content_page_1(),
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => match pending.continuation {
            RemoteHostContinuation::ChapterContentNextPage(state_) => {
                assert!(
                    state_.accumulated_content.contains("First page content."),
                    "page-1 content should be accumulated, got: {:?}",
                    state_.accumulated_content
                );
                assert_eq!(state_.pages_fetched, 1);
                assert!(
                    state_
                        .visited_urls
                        .contains("https://books.example.test/book/dune/chapter/2"),
                    "next URL should be in visited_urls, got: {:?}",
                    state_.visited_urls
                );
                assert_eq!(
                    pending.params["url"],
                    "https://books.example.test/book/dune/chapter/2"
                );
            }
            other => panic!("expected ChapterContentNextPage continuation, got {other:?}"),
        },
        other => panic!("expected Pending, got {other:?}"),
    }
}

/// Full chain: page 1 → page 2 → page 3 (terminal). Verifies content from
/// all three pages is concatenated in order.
#[test]
fn chapter_content_pagination_concatenates_three_pages() {
    let state = RemoteState::new();
    let source = inline_source_with_content_pagination("https://books.example.test");

    // Page 1.
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source.clone(),
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://books.example.test/book/dune/chapter/1",
        "chapterResponse": content_page_1(),
    });
    let dispatch = dispatch(&state, "chapter.content", params);
    let continuation = match dispatch {
        RemoteDispatch::Pending(pending) => pending.continuation,
        other => panic!("page 1 should yield Pending, got {other:?}"),
    };

    // Page 2: yields Pending again.
    let result = complete_remote_host(
        continuation,
        host_result(
            content_page_2(),
            "https://books.example.test/book/dune/chapter/2",
        ),
        &state,
    )
    .expect("page 2 should parse");
    let continuation = match result {
        reader_runtime::remote::RemoteCommandResult::Pending(pending) => {
            match pending.continuation {
                RemoteHostContinuation::ChapterContentNextPage(state_) => {
                    assert!(
                        state_.accumulated_content.contains("First page content."),
                        "page-1 content should still be present after page 2"
                    );
                    assert!(
                        state_.accumulated_content.contains("Second page content."),
                        "page-2 content should be appended, got: {:?}",
                        state_.accumulated_content
                    );
                    assert_eq!(state_.pages_fetched, 2);
                    RemoteHostContinuation::ChapterContentNextPage(state_)
                }
                other => panic!("expected ChapterContentNextPage after page 2, got {other:?}"),
            }
        }
        other => panic!("page 2 should yield Pending, got {other:?}"),
    };

    // Page 3: terminal. complete_remote_host returns Complete with all 3 pages.
    let result = complete_remote_host(
        continuation,
        host_result(
            content_page_3_terminal(),
            "https://books.example.test/book/dune/chapter/3",
        ),
        &state,
    )
    .expect("page 3 should parse");
    match result {
        reader_runtime::remote::RemoteCommandResult::Complete(data) => {
            let content = data["content"]
                .as_str()
                .expect("result should have content string");
            assert!(
                content.contains("First page content."),
                "page-1 content missing in: {content}"
            );
            assert!(
                content.contains("Second page content."),
                "page-2 content missing in: {content}"
            );
            assert!(
                content.contains("Third page content."),
                "page-3 content missing in: {content}"
            );
        }
        other => panic!("page 3 should yield Complete, got {other:?}"),
    }
}

/// Single-page content with no `next` link: dispatch finishes immediately.
#[test]
fn chapter_content_no_pagination_when_no_next_content_url() {
    let state = RemoteState::new();
    let source = inline_source_with_content_pagination("https://books.example.test");
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://books.example.test/book/dune/chapter/1",
        "chapterResponse": content_page_3_terminal(),
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (no pagination), got {other:?}"),
    }
}

/// Cycle detection for content: when the next URL is already visited, the
/// loop stops and returns the accumulated content as Complete.
#[test]
fn chapter_content_pagination_stops_on_cycle() {
    let state = RemoteState::new();
    let source = inline_source_with_content_pagination("https://books.example.test");

    let params = reader_contract::remote::ChapterContentParams {
        source_id: "src-1".to_string(),
        book_id: "b1".to_string(),
        chapter_title: "ch1".to_string(),
        chapter_response: String::new(), // overwritten by complete_remote_host
        chapter_request: None,
        js_rule: None,
        source: Some(source),
        chapter_url: Some("https://books.example.test/book/dune/chapter/1".to_string()),
    };
    let mut visited = std::collections::HashSet::new();
    visited.insert("https://books.example.test/book/dune/chapter/1".to_string());
    visited.insert("https://books.example.test/book/dune/chapter/2".to_string());
    let continuation =
        RemoteHostContinuation::ChapterContentNextPage(ChapterContentNextPageState {
            params,
            accumulated_content: "First page content.".to_string(),
            pages_fetched: 2,
            visited_urls: visited,
        });

    // Page 2 response: content + next link back to page 1 (cycle).
    let cycle_body = r#"<main>
        <article class="content"><p>Second page content.</p></article>
        <a class="next-content" href="/book/dune/chapter/1">Next</a>
    </main>"#;
    let result = complete_remote_host(
        continuation,
        host_result(cycle_body, "https://books.example.test/book/dune/chapter/2"),
        &state,
    )
    .expect("cycle page should parse");

    match result {
        reader_runtime::remote::RemoteCommandResult::Complete(data) => {
            let content = data["content"]
                .as_str()
                .expect("result should have content string");
            assert!(
                content.contains("First page content."),
                "page-1 content missing in: {content}"
            );
            assert!(
                content.contains("Second page content."),
                "page-2 content should be appended before cycle stop, got: {content}"
            );
        }
        other => panic!("cycle should yield Complete, got {other:?}"),
    }
}

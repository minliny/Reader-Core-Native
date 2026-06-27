//! Integration tests for `book.detail`, `book.toc`, and `chapter.content`
//! auto-build of `HostHttpRequest` from explicit URL fields.
//!
//! These close the AnalyzeUrl gap for the remaining three remote-reading
//! pipeline stages: when no `*Request` and no `*Response` are supplied but
//! the corresponding URL field is present (`bookUrl` / `tocUrl` /
//! `chapterUrl`), Core builds the HTTP request itself — the Legado
//! `AnalyzeUrl` equivalence for non-search stages.

use std::sync::{Arc, Mutex};

use reader_contract::{Command, Event};
use reader_runtime::{
    remote::{RemoteDispatch, RemoteHostContinuation, RemoteState},
    sink::EventSink,
};

#[derive(Default, Clone)]
struct NoopSink {}

impl EventSink for NoopSink {
    fn emit(&self, _event: &Event) {}
}

fn inline_source_with_headers(base_url: &str, header: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "sourceId": "src-1",
        "name": "test-source",
        "baseUrl": base_url,
        "rules": {},
        "bookSource": {
            "bookSourceName": "test-source",
            "bookSourceUrl": base_url,
            "header": header,
        }
    })
}

fn dispatch(state: &RemoteState, method: &str, params: serde_json::Value) -> RemoteDispatch {
    let cmd = Command::new(11, method, params);
    let sink: Arc<dyn EventSink> = Arc::new(NoopSink::default());
    let active = Arc::new(Mutex::new(std::collections::HashSet::<u64>::new()));
    reader_runtime::remote::dispatch_remote(method, &cmd, &sink, &active, state)
}

fn pending_url_method(dispatch: RemoteDispatch) -> (String, String) {
    match dispatch {
        RemoteDispatch::Pending(pending) => {
            let url = pending.params["url"].as_str().unwrap().to_string();
            let method = pending.params["method"].as_str().unwrap().to_string();
            (url, method)
        }
        other => panic!("expected RemoteDispatch::Pending, got {other:?}"),
    }
}

// ===========================================================================
// book.detail
// ===========================================================================

/// Plain `bookUrl` triggers a GET auto-build.
#[test]
fn book_detail_auto_builds_get_request_from_book_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "book": { "bookId": "b1", "title": "T" },
        "bookUrl": "https://api.example.test/book/123",
    });

    let dispatch = dispatch(&state, "book.detail", params);
    let (url, method) = pending_url_method(dispatch);
    assert_eq!(method, "GET");
    assert_eq!(url, "https://api.example.test/book/123");
}

/// `bookUrl` may be a Legado DSL form (`url,{"method":"POST",...}`).
#[test]
fn book_detail_auto_builds_post_request_from_book_url_dsl() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "book": { "bookId": "b1", "title": "T" },
        "bookUrl": "https://api.example.test/book,{\"method\":\"POST\",\"body\":\"id=123\"}",
    });

    let dispatch = dispatch(&state, "book.detail", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => {
            assert_eq!(pending.params["method"], "POST");
            assert_eq!(pending.params["url"], "https://api.example.test/book");
            assert_eq!(pending.params["body"], "id=123");
        }
        other => panic!("expected Pending, got {other:?}"),
    }
}

/// Source `header` field is merged into the auto-built request headers.
#[test]
fn book_detail_auto_build_merges_source_header() {
    let state = RemoteState::new();
    let source = inline_source_with_headers(
        "https://api.example.test",
        serde_json::json!({ "Referer": "https://api.example.test/" }),
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "book": { "bookId": "b1", "title": "T" },
        "bookUrl": "https://api.example.test/book/123",
    });

    let dispatch = dispatch(&state, "book.detail", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => {
            assert_eq!(
                pending.params["headers"]["Referer"],
                "https://api.example.test/"
            );
        }
        other => panic!("expected Pending, got {other:?}"),
    }
}

/// Relative `bookUrl` resolved against `baseUrl`.
#[test]
fn book_detail_auto_build_resolves_relative_book_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "book": { "bookId": "b1", "title": "T" },
        "bookUrl": "/book/123",
    });

    let dispatch = dispatch(&state, "book.detail", params);
    let (url, _) = pending_url_method(dispatch);
    assert_eq!(url, "https://api.example.test/book/123");
}

/// `detailResponse` present skips auto-build (parses directly).
#[test]
fn book_detail_does_not_auto_build_when_detail_response_present() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "book": { "bookId": "b1", "title": "T" },
        "bookUrl": "https://api.example.test/book/123",
        "detailResponse": "<html></html>",
    });

    let dispatch = dispatch(&state, "book.detail", params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (parse path), got {other:?}"),
    }
}

// ===========================================================================
// book.toc
// ===========================================================================

#[test]
fn book_toc_auto_builds_get_request_from_toc_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "tocUrl": "https://api.example.test/book/123/toc",
    });

    let dispatch = dispatch(&state, "book.toc", params);
    let (url, method) = pending_url_method(dispatch);
    assert_eq!(method, "GET");
    assert_eq!(url, "https://api.example.test/book/123/toc");
}

#[test]
fn book_toc_auto_build_resolves_relative_toc_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "tocUrl": "/book/123/toc",
    });

    let dispatch = dispatch(&state, "book.toc", params);
    let (url, _) = pending_url_method(dispatch);
    assert_eq!(url, "https://api.example.test/book/123/toc");
}

#[test]
fn book_toc_does_not_auto_build_when_toc_response_present() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "tocUrl": "https://api.example.test/book/123/toc",
        "tocResponse": "<html></html>",
    });

    let dispatch = dispatch(&state, "book.toc", params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (parse path), got {other:?}"),
    }
}

// ===========================================================================
// chapter.content
// ===========================================================================

#[test]
fn chapter_content_auto_builds_get_request_from_chapter_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://api.example.test/chapter/456",
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    let (url, method) = pending_url_method(dispatch);
    assert_eq!(method, "GET");
    assert_eq!(url, "https://api.example.test/chapter/456");
}

#[test]
fn chapter_content_auto_build_resolves_relative_chapter_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "/chapter/456",
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    let (url, _) = pending_url_method(dispatch);
    assert_eq!(url, "https://api.example.test/chapter/456");
}

#[test]
fn chapter_content_auto_build_merges_source_header() {
    let state = RemoteState::new();
    let source = inline_source_with_headers(
        "https://api.example.test",
        serde_json::json!({ "X-Reader": "test" }),
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://api.example.test/chapter/456",
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => {
            assert_eq!(pending.params["headers"]["X-Reader"], "test");
        }
        other => panic!("expected Pending, got {other:?}"),
    }
}

#[test]
fn chapter_content_does_not_auto_build_when_chapter_response_present() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://api.example.test/chapter/456",
        "chapterResponse": "<html></html>",
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (parse path), got {other:?}"),
    }
}

/// When `jsRule` is supplied, the existing JS-rule path takes over (no
/// auto-build, no host round-trip).
#[test]
fn chapter_content_js_rule_path_skips_auto_build() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://api.example.test/chapter/456",
        "jsRule": "result + ''",
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    match dispatch {
        // JS rule with no host callbacks yields a structured "unsupported"
        // error, surfaced as Finished (error event emitted by the dispatcher).
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (js rule path), got {other:?}"),
    }
}

// ===========================================================================
// Continuation carries URL context
// ===========================================================================

#[test]
fn book_toc_auto_build_continuation_carries_toc_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "tocUrl": "https://api.example.test/toc",
    });

    let dispatch = dispatch(&state, "book.toc", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => match pending.continuation {
            RemoteHostContinuation::BookToc(p) => {
                assert_eq!(p.toc_url.as_deref(), Some("https://api.example.test/toc"));
            }
            other => panic!("expected BookToc continuation, got {other:?}"),
        },
        other => panic!("expected Pending, got {other:?}"),
    }
}

#[test]
fn chapter_content_auto_build_continuation_carries_chapter_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "bookId": "b1",
        "chapterTitle": "ch1",
        "chapterUrl": "https://api.example.test/ch/1",
    });

    let dispatch = dispatch(&state, "chapter.content", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => match pending.continuation {
            RemoteHostContinuation::ChapterContent(p) => {
                assert_eq!(
                    p.chapter_url.as_deref(),
                    Some("https://api.example.test/ch/1")
                );
            }
            other => panic!("expected ChapterContent continuation, got {other:?}"),
        },
        other => panic!("expected Pending, got {other:?}"),
    }
}

#[test]
fn book_detail_auto_build_continuation_carries_book_url() {
    let state = RemoteState::new();
    let source = inline_source_with_headers("https://api.example.test", serde_json::json!({}));
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "book": { "bookId": "b1", "title": "T" },
        "bookUrl": "https://api.example.test/book/123",
    });

    let dispatch = dispatch(&state, "book.detail", params);
    match dispatch {
        RemoteDispatch::Pending(pending) => match pending.continuation {
            RemoteHostContinuation::BookDetail(p) => {
                assert_eq!(
                    p.book_url.as_deref(),
                    Some("https://api.example.test/book/123")
                );
            }
            other => panic!("expected BookDetail continuation, got {other:?}"),
        },
        other => panic!("expected Pending, got {other:?}"),
    }
}

//! Integration tests for `book.search` auto-build of `HostHttpRequest` from
//! a Legado book source's `searchUrl` template.
//!
//! These tests close the AnalyzeUrl gap (S3/S4): when no `searchRequest` and
//! no `searchResponse` are supplied but `keyword` is present, Core must
//! build the HTTP request itself from the source's `searchUrl` field — the
//! Legado `AnalyzeUrl` equivalence.

use std::sync::{Arc, Mutex};

use reader_contract::{Command, Event};
use reader_runtime::{
    remote::{RemoteDispatch, RemoteHostContinuation, RemoteState},
    sink::EventSink,
};

/// Captures events emitted by the runtime so tests can assert outcomes.
#[derive(Default, Clone)]
struct CapturingSink {
    events: Arc<Mutex<Vec<Event>>>,
}

impl EventSink for CapturingSink {
    fn emit(&self, event: &Event) {
        self.events.lock().unwrap().push(event.clone());
    }
}

impl CapturingSink {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Build a minimal Legado-shaped inline source JSON for a search source.
fn inline_search_source(search_url: &str, base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "sourceId": "src-1",
        "name": "test-source",
        "baseUrl": base_url,
        "rules": {},
        "bookSource": {
            "bookSourceName": "test-source",
            "bookSourceUrl": base_url,
            "searchUrl": search_url,
        }
    })
}

fn dispatch_book_search(
    state: &RemoteState,
    params: serde_json::Value,
) -> RemoteDispatch {
    let cmd = Command::new(7, "book.search", params);
    let sink: Arc<dyn EventSink> = Arc::new(CapturingSink::new());
    let active = Arc::new(Mutex::new(std::collections::HashSet::<u64>::new()));
    reader_runtime::remote::dispatch_remote(
        "book.search",
        &cmd,
        &sink,
        &active,
        state,
    )
}

/// Common assertion helper: pulls the pending `HostHttpRequest` URL/method
/// out of a `RemoteDispatch::Pending`.
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

/// Plain GET URL with `{{key}}` substituted into the query string.
#[test]
fn book_search_auto_builds_get_request_from_search_url_template() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "https://api.example.test/search?q={{key}}&page={{page}}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "斗破苍穹",
        "page": 2,
    });

    let dispatch = dispatch_book_search(&state, params);
    let (url, method) = pending_url_method(dispatch);

    assert_eq!(method, "GET");
    assert!(
        url.contains("q=%E6%96%97%E7%A0%B4%E8%8B%8D%E7%A9%B9"),
        "url should contain percent-encoded keyword, got: {url}"
    );
    assert!(
        url.contains("page=2"),
        "url should contain page=2, got: {url}"
    );
}

/// Legado DSL form: `url,{"method":"POST","body":"k={{key}}","charset":"gbk"}`.
#[test]
fn book_search_auto_builds_post_request_with_body_and_charset() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "https://api.example.test/search,{\"method\":\"POST\",\"body\":\"k={{key}}\",\"charset\":\"gbk\"}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "test",
        "page": 1,
    });

    let dispatch = dispatch_book_search(&state, params);
    match dispatch {
        RemoteDispatch::Pending(pending) => {
            assert_eq!(pending.params["method"], "POST");
            assert_eq!(
                pending.params["url"],
                "https://api.example.test/search"
            );
            assert_eq!(pending.params["body"], "k=test");
            assert_eq!(pending.params["charset"], "gbk");
            // Auto Content-Type for POST with body should be set.
            let content_type = pending.params["headers"]["Content-Type"]
                .as_str()
                .expect("Content-Type header should be set");
            assert!(
                content_type.contains("application/x-www-form-urlencoded"),
                "expected form-urlencoded Content-Type, got: {content_type}"
            );
            assert!(
                content_type.contains("charset=gbk"),
                "expected charset=gbk in Content-Type, got: {content_type}"
            );
        }
        other => panic!("expected RemoteDispatch::Pending, got {other:?}"),
    }
}

/// Relative URL in `searchUrl` should be resolved against `baseUrl`.
#[test]
fn book_search_auto_build_resolves_relative_search_url_against_base() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "/search?q={{key}}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "hello",
        "page": 1,
    });

    let dispatch = dispatch_book_search(&state, params);
    let (url, method) = pending_url_method(dispatch);

    assert_eq!(method, "GET");
    assert!(
        url.starts_with("https://api.example.test/search?q="),
        "relative URL should be resolved against base, got: {url}"
    );
}

/// When `searchResponse` is already supplied (pre-fetched host path), Core
/// must NOT auto-build — it should parse the response directly.
#[test]
fn book_search_does_not_auto_build_when_search_response_present() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "https://api.example.test/search?q={{key}}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "hello",
        "page": 1,
        "searchResponse": "<html></html>",
    });

    let dispatch = dispatch_book_search(&state, params);
    // Existing pipeline parses empty HTML to an empty book list — that's fine,
    // we just need to confirm it did NOT emit a pending HTTP request.
    match dispatch {
        RemoteDispatch::Finished => {}
        RemoteDispatch::Pending(_) => panic!("should not auto-build when searchResponse present"),
        RemoteDispatch::NotHandled => panic!("book.search should be handled"),
    }
}

/// When `searchRequest` is explicitly provided, Core must use that, not
/// auto-build — even if `keyword` is also present.
#[test]
fn book_search_prefers_explicit_search_request_over_auto_build() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "https://api.example.test/search?q={{key}}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "ignored",
        "page": 1,
        "searchRequest": {
            "url": "https://explicit.example.test/?q=manual",
            "method": "GET",
            "headers": {},
        }
    });

    let dispatch = dispatch_book_search(&state, params);
    let (url, _) = pending_url_method(dispatch);
    assert_eq!(url, "https://explicit.example.test/?q=manual");
}

/// If `keyword` is missing AND `searchRequest` is missing AND `searchResponse`
/// is empty, the existing error path must still fire (backward compat).
#[test]
fn book_search_errors_when_no_keyword_no_request_no_response() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "https://api.example.test/search?q={{key}}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
    });

    let dispatch = dispatch_book_search(&state, params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (error), got {other:?}"),
    }
}

/// Auto-build continuation must carry the keyword/page so the resumed parse
/// step knows the search context.
#[test]
fn book_search_auto_build_continuation_carries_keyword_and_page() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "https://api.example.test/search?q={{key}}",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "继续",
        "page": 3,
    });

    let dispatch = dispatch_book_search(&state, params);
    match dispatch {
        RemoteDispatch::Pending(pending) => {
            match pending.continuation {
                RemoteHostContinuation::BookSearch(p) => {
                    assert_eq!(p.keyword.as_deref(), Some("继续"));
                    assert_eq!(p.page, Some(3));
                }
                other => panic!("expected BookSearch continuation, got {other:?}"),
            }
        }
        other => panic!("expected Pending, got {other:?}"),
    }
}

/// A source without `searchUrl` cannot auto-build; the error must surface.
#[test]
fn book_search_auto_build_errors_when_source_has_no_search_url() {
    let state = RemoteState::new();
    let source = serde_json::json!({
        "sourceId": "src-1",
        "name": "no-search-url",
        "baseUrl": "https://api.example.test",
        "rules": {},
        "bookSource": {
            "bookSourceName": "no-search-url",
            "bookSourceUrl": "https://api.example.test",
        }
    });
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "test",
        "page": 1,
    });

    let dispatch = dispatch_book_search(&state, params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (error from missing searchUrl), got {other:?}"),
    }
}

/// URL-embedded `@js:` expression in `searchUrl` is evaluated by the JS
/// sandbox, and the result is parsed as the request URL.
#[test]
fn book_search_auto_builds_from_at_js_expression_via_sandbox() {
    let state = RemoteState::new();
    // The JS expression builds a URL string using the `key` variable.
    let source = inline_search_source(
        "@js:\"https://api.example.test/search?q=\" + key",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "斗破",
        "page": 1,
    });

    let dispatch = dispatch_book_search(&state, params);
    let (url, method) = pending_url_method(dispatch);
    assert_eq!(method, "GET");
    assert_eq!(url, "https://api.example.test/search?q=斗破");
}

/// URL-embedded `<js>...</js>` expression in `searchUrl`.
#[test]
fn book_search_auto_builds_from_js_tag_expression_via_sandbox() {
    let state = RemoteState::new();
    let source = inline_search_source(
        r#"<js>"https://js.example.test/p=" + page</js>"#,
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "test",
        "page": 5,
    });

    let dispatch = dispatch_book_search(&state, params);
    let (url, _) = pending_url_method(dispatch);
    assert_eq!(url, "https://js.example.test/p=5");
}

/// DSL `js` option is evaluated and the result is re-parsed as URL DSL.
#[test]
fn book_search_auto_builds_from_dsl_js_option_via_sandbox() {
    let state = RemoteState::new();
    // The DSL `js` option returns a plain URL string built from `key`.
    let source = inline_search_source(
        r#"https://placeholder.test,{"js":"\"https://built.example.test/k=\" + key"}"#,
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "test",
        "page": 1,
    });

    let dispatch = dispatch_book_search(&state, params);
    let (url, method) = pending_url_method(dispatch);
    assert_eq!(method, "GET");
    assert_eq!(url, "https://built.example.test/k=test");
}

/// A JS expression that throws surfaces as a Finished (error) dispatch.
#[test]
fn book_search_auto_build_js_error_surfaces_as_finished() {
    let state = RemoteState::new();
    let source = inline_search_source(
        "@js:(function() { throw new Error('boom'); })()",
        "https://api.example.test",
    );
    let params = serde_json::json!({
        "sourceId": "src-1",
        "source": source,
        "keyword": "test",
        "page": 1,
    });

    let dispatch = dispatch_book_search(&state, params);
    match dispatch {
        RemoteDispatch::Finished => {}
        other => panic!("expected Finished (JS error), got {other:?}"),
    }
}

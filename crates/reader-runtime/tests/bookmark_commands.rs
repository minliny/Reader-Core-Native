//! Integration tests for the `bookmark.*` command vertical.
//!
//! Mirrors Legado `Bookmark.kt` (entity) + `BookmarkDao.kt` (CRUD). Core
//! exposes pure CRUD over the in-memory `bookmarks` table; no host callback
//! is required. The `Bookmark` entity already exists in `reader-domain`;
//! these tests exercise the protocol layer (reader-contract DTOs +
//! reader-runtime dispatch).

use std::sync::{Arc, Mutex};

use reader_contract::{Command, Event};
use reader_runtime::{
    remote::{RemoteDispatch, RemoteState},
    sink::EventSink,
};

type EventLog = Arc<Mutex<Vec<Event>>>;

#[derive(Clone)]
struct CapturingSink {
    events: EventLog,
}

impl EventSink for CapturingSink {
    fn emit(&self, event: &Event) {
        self.events.lock().unwrap().push(event.clone());
    }
}

impl CapturingSink {
    fn new() -> (Self, EventLog) {
        let events: EventLog = Arc::new(Mutex::new(Vec::new()));
        let sink = Self {
            events: events.clone(),
        };
        (sink, events)
    }
}

fn last_event(events: &EventLog) -> Event {
    events
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("at least one event emitted")
}

fn dispatch_capture(
    state: &RemoteState,
    method: &str,
    params: serde_json::Value,
) -> (RemoteDispatch, Event) {
    let cmd = Command::new(101, method, params);
    let (sink, events) = CapturingSink::new();
    let sink: Arc<dyn EventSink> = Arc::new(sink);
    let active = Arc::new(Mutex::new(std::collections::HashSet::<u64>::new()));
    let disp = reader_runtime::remote::dispatch_remote(method, &cmd, &sink, &active, state);
    let event = last_event(&events);
    (disp, event)
}

fn expect_result(event: Event) -> serde_json::Value {
    match event {
        Event::Result { data, .. } => data,
        other => panic!("expected Event::Result, got {other:?}"),
    }
}

fn expect_error(event: Event) -> serde_json::Value {
    match event {
        Event::Error { error, .. } => serde_json::to_value(&error).expect("error serializes"),
        other => panic!("expected Event::Error, got {other:?}"),
    }
}

fn create_params(
    time: Option<i64>,
    book_name: &str,
    chapter_name: &str,
    content: &str,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "bookName": book_name,
        "bookAuthor": "Author",
        "chapterIndex": 3,
        "chapterPos": 120,
        "chapterName": chapter_name,
        "bookText": "原文片段",
        "content": content
    });
    if let Some(time) = time {
        params["time"] = serde_json::json!(time);
    }
    params
}

#[test]
fn create_with_explicit_time_round_trips() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(Some(1700000000), "My Book", "Ch.3", "笔记"),
    );
    let data = expect_result(event);
    let bookmark = &data["bookmark"];
    assert_eq!(bookmark["time"], 1700000000);
    assert_eq!(bookmark["bookName"], "My Book");
    assert_eq!(bookmark["bookAuthor"], "Author");
    assert_eq!(bookmark["chapterIndex"], 3);
    assert_eq!(bookmark["chapterPos"], 120);
    assert_eq!(bookmark["chapterName"], "Ch.3");
    assert_eq!(bookmark["bookText"], "原文片段");
    assert_eq!(bookmark["content"], "笔记");
}

#[test]
fn create_without_time_assigns_monotonic_time() {
    let state = RemoteState::new();
    let (_disp, first) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(None, "Book A", "c1", "n1"),
    );
    let (_disp, second) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(None, "Book B", "c2", "n2"),
    );
    let t1 = expect_result(first)["bookmark"]["time"].as_i64().unwrap();
    let t2 = expect_result(second)["bookmark"]["time"].as_i64().unwrap();
    assert!(t2 > t1, "times must be monotonic: {t1} -> {t2}");
}

#[test]
fn list_returns_all_bookmarks_sorted_by_time() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(Some(100), "A", "c1", "n1"),
    );
    let (_disp, _) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(Some(50), "B", "c2", "n2"),
    );

    let (_disp, event) = dispatch_capture(&state, "bookmark.list", serde_json::json!({}));
    let bookmarks = expect_result(event)["bookmarks"]
        .as_array()
        .expect("bookmarks array")
        .clone();
    assert_eq!(bookmarks.len(), 2);
    assert_eq!(bookmarks[0]["time"], 50);
    assert_eq!(bookmarks[1]["time"], 100);
}

#[test]
fn list_filtered_by_book_name_and_author() {
    let state = RemoteState::new();
    let mut a1 = create_params(Some(10), "Alpha", "c1", "n1");
    a1["bookAuthor"] = serde_json::json!("X");
    let mut a2 = create_params(Some(11), "Alpha", "c2", "n2");
    a2["bookAuthor"] = serde_json::json!("X");
    let mut b1 = create_params(Some(12), "Beta", "c3", "n3");
    b1["bookAuthor"] = serde_json::json!("Y");
    let (_disp, _) = dispatch_capture(&state, "bookmark.create", a1);
    let (_disp, _) = dispatch_capture(&state, "bookmark.create", a2);
    let (_disp, _) = dispatch_capture(&state, "bookmark.create", b1);

    let (_disp, event) = dispatch_capture(
        &state,
        "bookmark.list",
        serde_json::json!({ "bookName": "Alpha", "bookAuthor": "X" }),
    );
    let bookmarks = expect_result(event)["bookmarks"]
        .as_array()
        .expect("bookmarks array")
        .clone();
    assert_eq!(bookmarks.len(), 2);
    assert!(bookmarks.iter().all(|b| b["bookName"] == "Alpha"));
}

#[test]
fn update_applies_partial_changes_and_returns_updated_bookmark() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(Some(77), "Orig", "ch", "orig note"),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "bookmark.update",
        serde_json::json!({
            "time": 77,
            "content": "updated note",
            "chapterPos": 999
        }),
    );
    let bookmark = expect_result(event)["bookmark"].clone();
    assert_eq!(bookmark["time"], 77);
    assert_eq!(bookmark["bookName"], "Orig"); // unchanged
    assert_eq!(bookmark["chapterName"], "ch"); // unchanged
    assert_eq!(bookmark["content"], "updated note"); // updated
    assert_eq!(bookmark["chapterPos"], 999); // updated
}

#[test]
fn update_unknown_time_returns_invalid_params_error() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "bookmark.update",
        serde_json::json!({ "time": 999, "content": "x" }),
    );
    let err = expect_error(event);
    let code = err["code"].as_str().unwrap_or("");
    assert!(
        code.to_lowercase().contains("invalid_params"),
        "expected invalid_params error, got: {err}"
    );
}

#[test]
fn delete_existing_bookmark_returns_deleted_true() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "bookmark.create",
        create_params(Some(42), "Tmp", "ch", "note"),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "bookmark.delete",
        serde_json::json!({ "time": 42 }),
    );
    let data = expect_result(event);
    assert_eq!(data["time"], 42);
    assert_eq!(data["deleted"], true);

    let (_disp, list_event) = dispatch_capture(&state, "bookmark.list", serde_json::json!({}));
    let bookmarks = expect_result(list_event)["bookmarks"]
        .as_array()
        .expect("bookmarks array")
        .clone();
    assert_eq!(bookmarks.len(), 0);
}

#[test]
fn delete_unknown_time_is_idempotent_returns_deleted_false() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "bookmark.delete",
        serde_json::json!({ "time": 9999 }),
    );
    let data = expect_result(event);
    assert_eq!(data["time"], 9999);
    assert_eq!(data["deleted"], false);
}

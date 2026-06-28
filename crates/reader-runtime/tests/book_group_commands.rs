//! Integration tests for the `book-group.*` command vertical.
//!
//! Mirrors Legado `BookGroup.kt` (entity) + `BookGroupDao.kt` (CRUD). Core
//! exposes pure CRUD over the in-memory `book_groups` table; no host callback
//! is required.

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
    group_id: Option<i64>,
    group_name: &str,
    order: i32,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "groupName": group_name,
        "order": order,
        "enableRefresh": true,
        "show": true
    });
    if let Some(id) = group_id {
        params["groupId"] = serde_json::json!(id);
    }
    params
}

#[test]
fn create_with_explicit_id_round_trips() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "book-group.create",
        create_params(Some(1700000000), "我的分组", 0),
    );
    let data = expect_result(event);
    let group = &data["group"];
    assert_eq!(group["groupId"], 1700000000);
    assert_eq!(group["groupName"], "我的分组");
    assert_eq!(group["order"], 0);
    assert_eq!(group["enableRefresh"], true);
    assert_eq!(group["show"], true);
    // Optional cover absent.
    assert!(group.get("cover").is_none() || group["cover"].is_null());
}

#[test]
fn create_without_id_assigns_monotonic_id() {
    let state = RemoteState::new();
    let (_disp, first) = dispatch_capture(
        &state,
        "book-group.create",
        create_params(None, "g1", 0),
    );
    let (_disp, second) = dispatch_capture(
        &state,
        "book-group.create",
        create_params(None, "g2", 1),
    );
    let id1 = expect_result(first)["group"]["groupId"].as_i64().unwrap();
    let id2 = expect_result(second)["group"]["groupId"].as_i64().unwrap();
    assert!(id2 > id1, "ids must be monotonic: {id1} -> {id2}");
}

#[test]
fn list_returns_groups_sorted_by_order_then_id() {
    let state = RemoteState::new();
    let mut late = create_params(Some(1), "late", 10);
    late["order"] = serde_json::json!(10);
    let mut early = create_params(Some(2), "early", 1);
    early["order"] = serde_json::json!(1);
    let (_disp, _) = dispatch_capture(&state, "book-group.create", late);
    let (_disp, _) = dispatch_capture(&state, "book-group.create", early);

    let (_disp, event) = dispatch_capture(&state, "book-group.list", serde_json::json!({}));
    let groups = expect_result(event)["groups"]
        .as_array()
        .expect("groups array")
        .clone();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["groupName"], "early");
    assert_eq!(groups[1]["groupName"], "late");
}

#[test]
fn list_with_show_only_filters_hidden_groups() {
    let state = RemoteState::new();
    let mut visible = create_params(Some(1), "visible", 0);
    visible["show"] = serde_json::json!(true);
    let mut hidden = create_params(Some(2), "hidden", 0);
    hidden["show"] = serde_json::json!(false);
    let (_disp, _) = dispatch_capture(&state, "book-group.create", visible);
    let (_disp, _) = dispatch_capture(&state, "book-group.create", hidden);

    let (_disp, event) = dispatch_capture(
        &state,
        "book-group.list",
        serde_json::json!({ "showOnly": true }),
    );
    let groups = expect_result(event)["groups"]
        .as_array()
        .expect("groups array")
        .clone();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["groupName"], "visible");
}

#[test]
fn update_applies_partial_changes_and_returns_updated_group() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "book-group.create",
        create_params(Some(7), "orig", 0),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "book-group.update",
        serde_json::json!({
            "groupId": 7,
            "groupName": "renamed",
            "order": 5
        }),
    );
    let group = expect_result(event)["group"].clone();
    assert_eq!(group["groupId"], 7);
    assert_eq!(group["groupName"], "renamed");
    assert_eq!(group["order"], 5);
    assert_eq!(group["show"], true); // unchanged
}

#[test]
fn update_unknown_id_returns_invalid_params_error() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "book-group.update",
        serde_json::json!({ "groupId": 999, "groupName": "x" }),
    );
    let err = expect_error(event);
    let code = err["code"].as_str().unwrap_or("");
    assert!(
        code.to_lowercase().contains("invalid_params"),
        "expected invalid_params error, got: {err}"
    );
}

#[test]
fn delete_existing_group_returns_deleted_true() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "book-group.create",
        create_params(Some(5), "tmp", 0),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "book-group.delete",
        serde_json::json!({ "groupId": 5 }),
    );
    let data = expect_result(event);
    assert_eq!(data["groupId"], 5);
    assert_eq!(data["deleted"], true);

    let (_disp, list_event) = dispatch_capture(&state, "book-group.list", serde_json::json!({}));
    let groups = expect_result(list_event)["groups"]
        .as_array()
        .expect("groups array")
        .clone();
    assert_eq!(groups.len(), 0);
}

#[test]
fn delete_unknown_id_is_idempotent_returns_deleted_false() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "book-group.delete",
        serde_json::json!({ "groupId": 1234 }),
    );
    let data = expect_result(event);
    assert_eq!(data["groupId"], 1234);
    assert_eq!(data["deleted"], false);
}

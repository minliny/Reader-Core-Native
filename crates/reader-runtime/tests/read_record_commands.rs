//! Integration tests for the `read-record.*` command vertical.
//!
//! Mirrors Legado `ReadRecord.kt` (entity) + `ReadRecordDao.kt` (CRUD). Core
//! exposes pure CRUD over the in-memory `read_records` table; composite key
//! is `(deviceId, bookName)`. No host callback is required.

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

fn create_params(device_id: &str, book_name: &str, read_time: i64, last_read: i64) -> serde_json::Value {
    serde_json::json!({
        "deviceId": device_id,
        "bookName": book_name,
        "readTime": read_time,
        "lastRead": last_read
    })
}

#[test]
fn create_round_trips_and_persists() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "My Book", 3600000, 1700000000),
    );
    let data = expect_result(event);
    let record = &data["record"];
    assert_eq!(record["deviceId"], "dev-1");
    assert_eq!(record["bookName"], "My Book");
    assert_eq!(record["readTime"], 3600000);
    assert_eq!(record["lastRead"], 1700000000);
}

#[test]
fn create_with_empty_book_name_returns_invalid_params_error() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "  ", 0, 0),
    );
    let err = expect_error(event);
    let code = err["code"].as_str().unwrap_or("");
    assert!(
        code.to_lowercase().contains("invalid_params"),
        "expected invalid_params error, got: {err}"
    );
}

#[test]
fn create_upserts_on_composite_key() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "Book", 1000, 100),
    );
    let (_disp, second) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "Book", 2000, 200),
    );
    let record = expect_result(second)["record"].clone();
    assert_eq!(record["readTime"], 2000);
    assert_eq!(record["lastRead"], 200);

    let (_disp, list_event) = dispatch_capture(&state, "read-record.list", serde_json::json!({}));
    let records = expect_result(list_event)["records"]
        .as_array()
        .expect("records array")
        .clone();
    assert_eq!(records.len(), 1, "upsert should not duplicate rows");
}

#[test]
fn list_returns_all_records() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "A", 100, 1),
    );
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-2", "B", 200, 2),
    );

    let (_disp, event) = dispatch_capture(&state, "read-record.list", serde_json::json!({}));
    let records = expect_result(event)["records"]
        .as_array()
        .expect("records array")
        .clone();
    assert_eq!(records.len(), 2);
}

#[test]
fn list_filtered_by_device_id() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-A", "Book1", 100, 1),
    );
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-A", "Book2", 200, 2),
    );
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-B", "Book3", 300, 3),
    );

    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.list",
        serde_json::json!({ "deviceId": "dev-A" }),
    );
    let records = expect_result(event)["records"]
        .as_array()
        .expect("records array")
        .clone();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r["deviceId"] == "dev-A"));
}

#[test]
fn update_applies_partial_changes_and_returns_updated_record() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "Orig", 1000, 100),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.update",
        serde_json::json!({
            "deviceId": "dev-1",
            "bookName": "Orig",
            "readTime": 5000
        }),
    );
    let record = expect_result(event)["record"].clone();
    assert_eq!(record["deviceId"], "dev-1");
    assert_eq!(record["bookName"], "Orig");
    assert_eq!(record["readTime"], 5000); // updated
    assert_eq!(record["lastRead"], 100); // unchanged
}

#[test]
fn update_unknown_composite_key_returns_invalid_params_error() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.update",
        serde_json::json!({
            "deviceId": "nope",
            "bookName": "missing",
            "readTime": 1
        }),
    );
    let err = expect_error(event);
    let code = err["code"].as_str().unwrap_or("");
    assert!(
        code.to_lowercase().contains("invalid_params"),
        "expected invalid_params error, got: {err}"
    );
}

#[test]
fn delete_existing_record_returns_deleted_true() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "read-record.create",
        create_params("dev-1", "Tmp", 100, 1),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.delete",
        serde_json::json!({ "deviceId": "dev-1", "bookName": "Tmp" }),
    );
    let data = expect_result(event);
    assert_eq!(data["deviceId"], "dev-1");
    assert_eq!(data["bookName"], "Tmp");
    assert_eq!(data["deleted"], true);

    let (_disp, list_event) = dispatch_capture(&state, "read-record.list", serde_json::json!({}));
    let records = expect_result(list_event)["records"]
        .as_array()
        .expect("records array")
        .clone();
    assert_eq!(records.len(), 0);
}

#[test]
fn delete_unknown_composite_key_is_idempotent_returns_deleted_false() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "read-record.delete",
        serde_json::json!({ "deviceId": "nope", "bookName": "missing" }),
    );
    let data = expect_result(event);
    assert_eq!(data["deviceId"], "nope");
    assert_eq!(data["bookName"], "missing");
    assert_eq!(data["deleted"], false);
}

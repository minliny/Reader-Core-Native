//! Integration tests for the `replace-rule.*` command vertical.
//!
//! Mirrors Legado `ReplaceRule.kt` (entity) + `ReplaceRuleDao.kt` (CRUD).
//! Core exposes pure CRUD over the in-memory `replace_rules` table; no host
//! callback is required.

use std::sync::{Arc, Mutex};

use reader_contract::{Command, Event};
use reader_runtime::{
    remote::{RemoteDispatch, RemoteState},
    sink::EventSink,
};

/// Shared event log used by `CapturingSink`.
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
    id: Option<i64>,
    name: &str,
    pattern: &str,
    replacement: &str,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "name": name,
        "pattern": pattern,
        "replacement": replacement,
        "isRegex": true,
        "isEnabled": true,
        "order": 0,
        "scopeTitle": false,
        "scopeContent": true
    });
    if let Some(id) = id {
        params["id"] = serde_json::json!(id);
    }
    params
}

#[test]
fn create_with_explicit_id_round_trips() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "replace-rule.create",
        create_params(Some(42), "ads", "广告", "[AD]"),
    );
    let data = expect_result(event);
    let rule = &data["rule"];
    assert_eq!(rule["id"], 42);
    assert_eq!(rule["name"], "ads");
    assert_eq!(rule["pattern"], "广告");
    assert_eq!(rule["replacement"], "[AD]");
    assert_eq!(rule["isRegex"], true);
    assert_eq!(rule["isEnabled"], true);
    assert_eq!(rule["scopeContent"], true);
    assert_eq!(rule["scopeTitle"], false);
}

#[test]
fn create_without_id_assigns_monotonic_id() {
    let state = RemoteState::new();
    let (_disp, first) = dispatch_capture(
        &state,
        "replace-rule.create",
        create_params(None, "r1", "a", "b"),
    );
    let (_disp, second) = dispatch_capture(
        &state,
        "replace-rule.create",
        create_params(None, "r2", "c", "d"),
    );
    let id1 = expect_result(first)["rule"]["id"].as_i64().unwrap();
    let id2 = expect_result(second)["rule"]["id"].as_i64().unwrap();
    assert!(id2 > id1, "ids must be monotonic: {id1} -> {id2}");
}

#[test]
fn list_returns_created_rules_sorted_by_order() {
    let state = RemoteState::new();
    let mut late = create_params(Some(1), "late", "x", "y");
    late["order"] = serde_json::json!(10);
    let mut early = create_params(Some(2), "early", "p", "q");
    early["order"] = serde_json::json!(1);
    let (_disp, _) = dispatch_capture(&state, "replace-rule.create", late);
    let (_disp, _) = dispatch_capture(&state, "replace-rule.create", early);

    let (_disp, event) = dispatch_capture(&state, "replace-rule.list", serde_json::json!({}));
    let rules = expect_result(event)["rules"]
        .as_array()
        .expect("rules array")
        .clone();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["name"], "early");
    assert_eq!(rules[1]["name"], "late");
}

#[test]
fn list_with_enabled_only_filters_disabled_rules() {
    let state = RemoteState::new();
    let mut enabled = create_params(Some(1), "on", "a", "b");
    enabled["isEnabled"] = serde_json::json!(true);
    let mut disabled = create_params(Some(2), "off", "c", "d");
    disabled["isEnabled"] = serde_json::json!(false);
    let (_disp, _) = dispatch_capture(&state, "replace-rule.create", enabled);
    let (_disp, _) = dispatch_capture(&state, "replace-rule.create", disabled);

    let (_disp, event) = dispatch_capture(
        &state,
        "replace-rule.list",
        serde_json::json!({ "enabledOnly": true }),
    );
    let rules = expect_result(event)["rules"]
        .as_array()
        .expect("rules array")
        .clone();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["name"], "on");
}

#[test]
fn update_applies_partial_changes_and_returns_updated_rule() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "replace-rule.create",
        create_params(Some(7), "orig", "a", "b"),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "replace-rule.update",
        serde_json::json!({
            "id": 7,
            "name": "renamed",
            "replacement": "B"
        }),
    );
    let rule = expect_result(event)["rule"].clone();
    assert_eq!(rule["id"], 7);
    assert_eq!(rule["name"], "renamed");
    assert_eq!(rule["pattern"], "a"); // unchanged
    assert_eq!(rule["replacement"], "B"); // updated
}

#[test]
fn update_unknown_id_returns_invalid_params_error() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "replace-rule.update",
        serde_json::json!({ "id": 999, "name": "x" }),
    );
    let err = expect_error(event);
    let code = err["code"].as_str().unwrap_or("");
    assert!(
        code.to_lowercase().contains("invalid_params"),
        "expected invalid_params error, got: {err}"
    );
}

#[test]
fn delete_existing_rule_returns_deleted_true() {
    let state = RemoteState::new();
    let (_disp, _) = dispatch_capture(
        &state,
        "replace-rule.create",
        create_params(Some(5), "tmp", "a", "b"),
    );
    let (_disp, event) = dispatch_capture(
        &state,
        "replace-rule.delete",
        serde_json::json!({ "id": 5 }),
    );
    let data = expect_result(event);
    assert_eq!(data["id"], 5);
    assert_eq!(data["deleted"], true);

    let (_disp, list_event) = dispatch_capture(&state, "replace-rule.list", serde_json::json!({}));
    let rules = expect_result(list_event)["rules"]
        .as_array()
        .expect("rules array")
        .clone();
    assert_eq!(rules.len(), 0);
}

#[test]
fn delete_unknown_id_is_idempotent_returns_deleted_false() {
    let state = RemoteState::new();
    let (_disp, event) = dispatch_capture(
        &state,
        "replace-rule.delete",
        serde_json::json!({ "id": 1234 }),
    );
    let data = expect_result(event);
    assert_eq!(data["id"], 1234);
    assert_eq!(data["deleted"], false);
}

#[test]
fn created_rule_round_trips_through_storage_and_matches_legado_shape() {
    let state = RemoteState::new();
    let mut params = create_params(Some(11), "scope-test", "foo", "bar");
    params["scope"] = serde_json::json!("My Book");
    params["excludeScope"] = serde_json::json!("Banned Source");
    params["group"] = serde_json::json!("cleanup");
    params["isRegex"] = serde_json::json!(false);
    params["timeoutMillisecond"] = serde_json::json!(2000);

    let (_disp, event) = dispatch_capture(&state, "replace-rule.create", params);
    let rule = expect_result(event)["rule"].clone();
    assert_eq!(rule["id"], 11);
    assert_eq!(rule["scope"], "My Book");
    assert_eq!(rule["excludeScope"], "Banned Source");
    assert_eq!(rule["group"], "cleanup");
    assert_eq!(rule["isRegex"], false);
    assert_eq!(rule["timeoutMillisecond"], 2000);
}

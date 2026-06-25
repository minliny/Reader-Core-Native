//! Integration tests for the `host-replay` tool, exercising the public lib API
//! against the sample fixtures shipped in `samples/host-replay`.

use std::path::PathBuf;

use host_replay::*;
use serde_json::Value;

fn samples_dir() -> PathBuf {
    // Integration tests run with CWD = tools/host-replay.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples/host-replay")
}

fn load_samples() -> Vec<(PathBuf, Fixture)> {
    let dir = samples_dir();
    assert!(
        dir.is_dir(),
        "samples dir missing: {}",
        dir.display()
    );
    load_fixture_dir(&dir).expect("samples should load")
}

#[test]
fn all_samples_validate() {
    for (path, fixture) in load_samples() {
        validate(&fixture).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

#[test]
fn complete_envelope_has_protocol_shape() {
    let dir = samples_dir();
    let fixture = load_fixture(&dir.join("001-simple-get.json")).unwrap();
    let cmd = build_complete_command(&fixture, &dir, 42, 99).unwrap();
    assert_eq!(cmd["protocolVersion"], 1);
    assert_eq!(cmd["method"], "host.complete");
    assert_eq!(cmd["requestId"], 42);
    assert_eq!(cmd["params"]["operationId"], 99);
    assert_eq!(cmd["params"]["result"]["status"], 200);
    // body must be a JSON string (the protocol shape), not a parsed object.
    assert!(cmd["params"]["result"]["body"].is_string());
}

#[test]
fn error_envelope_has_protocol_shape() {
    let dir = samples_dir();
    let fixture = load_fixture(&dir.join("003-error-timeout.json")).unwrap();
    let cmd = build_error_command(&fixture, 7, 8).unwrap();
    assert_eq!(cmd["method"], "host.error");
    assert_eq!(cmd["params"]["operationId"], 8);
    assert_eq!(cmd["params"]["error"]["code"], "HTTP_TRANSPORT_TIMEOUT");
    assert_eq!(cmd["params"]["error"]["retryable"], true);
}

#[test]
fn redirect_fixture_emits_final_url() {
    let dir = samples_dir();
    let fixture = load_fixture(&dir.join("002-redirect-cookie.json")).unwrap();
    let cmd = build_complete_command(&fixture, &dir, 1, 1).unwrap();
    assert_eq!(
        cmd["params"]["result"]["finalUrl"],
        "https://login.example.test/dashboard"
    );
    // Set-Cookie multi-value must pass through as an array.
    assert!(cmd["params"]["result"]["headers"]["set-cookie"].is_array());
}

#[test]
fn bodyfile_loaded_as_text() {
    let dir = samples_dir();
    let fixture = load_fixture(&dir.join("004-post-with-bodyfile.json")).unwrap();
    let cmd = build_complete_command(&fixture, &dir, 1, 1).unwrap();
    let body = cmd["params"]["result"]["body"].as_str().unwrap();
    assert!(body.contains("synthetic-replay-token"));
}

#[test]
fn binary_body_emitted_as_base64() {
    let dir = samples_dir();
    let fixture = load_fixture(&dir.join("005-binary-body.json")).unwrap();
    let cmd = build_complete_command(&fixture, &dir, 1, 1).unwrap();
    // Binary body must come through as bodyBase64, not body.
    assert!(cmd["params"]["result"].get("body").is_none());
    let b64 = cmd["params"]["result"]["bodyBase64"].as_str().unwrap();
    assert!(!b64.is_empty());
}

#[test]
fn wildcard_fixture_matches_any_chapter_path() {
    let dir = samples_dir();
    let (_, fixture) = load_fixture_dir(&dir)
        .unwrap()
        .into_iter()
        .find(|(p, _)| p.file_name().unwrap() == "006-wildcard-chapter.json")
        .unwrap();
    let incoming = IncomingRequest {
        request_id: 1,
        operation_id: 1,
        capability: "http.execute".into(),
        params: serde_json::json!({"url": "https://content.example.test/chapters/99", "method": "GET"}),
    };
    assert!(matches(&fixture, &incoming));
}

#[test]
fn replay_correlates_incoming_operation_id() {
    // The emitted command must carry the INCOMING operationId, not the
    // fixture's recorded label — that is how Core correlates the completion.
    let dir = samples_dir();
    let (_, fixture) = load_fixture_dir(&dir)
        .unwrap()
        .into_iter()
        .find(|(p, _)| p.file_name().unwrap() == "001-simple-get.json")
        .unwrap();
    let incoming = IncomingRequest {
        request_id: 777,
        operation_id: 555,
        capability: "http.execute".into(),
        params: serde_json::json!({"url": "https://books.example.test/search?q=dune", "method": "GET"}),
    };
    assert!(matches(&fixture, &incoming));
    let cmd = build_command(&fixture, &dir, 1, incoming.operation_id).unwrap();
    assert_eq!(cmd["params"]["operationId"], 555);
}

#[test]
fn set_cookie_capture_into_jar() {
    let dir = samples_dir();
    let fixture = load_fixture(&dir.join("002-redirect-cookie.json")).unwrap();
    let resp = fixture.response.as_ref().unwrap();
    let set_cookies = extract_set_cookies(&resp.headers);
    assert_eq!(set_cookies.len(), 2);
    let mut jar = CookieJar::new();
    merge_set_cookies(&mut jar, "https://login.example.test", &set_cookies);
    let bucket = &jar["https://login.example.test"];
    assert_eq!(bucket.len(), 2);
    assert_eq!(bucket[0].name, "sid");
    assert!(bucket[0].http_only);
    assert!(bucket[0].secure);
}

#[test]
fn normalize_url_sorts_query_and_lowercases_host() {
    assert_eq!(
        normalize_url("https://EXAMPLE.test/search?b=2&a=1"),
        normalize_url("https://example.test/search?a=1&b=2")
    );
}

#[test]
fn emitted_command_is_valid_json_object() {
    let dir = samples_dir();
    for (_, fixture) in load_samples() {
        let cmd = build_command(&fixture, &dir, 1, 1).unwrap();
        assert!(cmd.is_object());
        assert!(cmd.get("method").is_some());
        // Round-trip through a string to confirm it's serializable.
        let s = serde_json::to_string(&cmd).unwrap();
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back["protocolVersion"], 1);
    }
}

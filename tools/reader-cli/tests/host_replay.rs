//! Host replay smoke for request/session contract fields. This drives
//! `reader-cli --host-replay` without opening a socket and asserts that Core
//! emits the exact `http.execute` descriptor from the fixture, then resumes with
//! host-provided response diagnostics.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_reader-cli");

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/fixtures/host_replay");
    p.push(name);
    p.canonicalize().unwrap_or(p)
}

#[test]
fn host_replay_roundtrips_request_session_contract_fields() {
    let events = run_host_replay_fixture("--host-replay", "request_session_search.json");

    assert_eq!(
        events.len(),
        2,
        "expected host.request + result: {events:?}"
    );

    let host_request = &events[0];
    assert_eq!(host_request["type"], "host.request");
    assert_eq!(host_request["capability"], "http.execute");
    assert_eq!(host_request["params"]["charset"], "gbk");
    assert_eq!(host_request["params"]["followRedirects"], false);
    assert_eq!(host_request["params"]["maxRedirects"], 0);
    assert_eq!(host_request["params"]["retry"]["maxAttempts"], 2);
    assert_eq!(host_request["params"]["usePlatformCookieJar"], false);
    assert_eq!(host_request["params"]["session"]["id"], "core-session-main");
    assert_eq!(host_request["params"]["headers"]["Cookie"], "sid=old");

    let result = &events[1];
    assert_eq!(result["type"], "result");
    assert_eq!(result["requestId"], 801);
    assert_eq!(result["data"]["books"].as_array().unwrap().len(), 0);
    assert_eq!(result["data"]["http"]["status"], 200);
    assert_eq!(
        result["data"]["http"]["finalUrl"],
        "https://books.example.test/search?q=dune"
    );
    assert_eq!(result["data"]["http"]["charsetHint"], "gbk");
    assert_eq!(
        result["data"]["http"]["headers"]["set-cookie"][0],
        "sid=new; Path=/; HttpOnly"
    );
    assert_eq!(result["data"]["http"]["session"]["id"], "core-session-main");
}

#[test]
fn host_replay_suite_runs_remote_reading_e2e() {
    let events = run_host_replay_fixture("--host-replay-suite", "remote_reading_e2e_suite.json");

    assert_eq!(
        events.len(),
        8,
        "expected four host.request/result pairs: {events:?}"
    );

    assert_host_request(
        &events[0],
        901,
        1,
        json!({
            "url": "https://books.example.test/search?q=dune",
            "method": "POST",
            "headers": {
                "Accept": "application/json",
                "Cookie": "sid=old",
                "X-Reader-Step": "search"
            },
            "body": "q=dune",
            "charset": "gbk",
            "followRedirects": false,
            "maxRedirects": 0,
            "retry": {
                "maxAttempts": 2,
                "backoffMillis": 50
            },
            "usePlatformCookieJar": false,
            "session": {
                "id": "core-session-main"
            }
        }),
    );
    assert_result(&events[1], 901);
    assert_eq!(events[1]["data"]["books"].as_array().unwrap().len(), 2);
    assert_eq!(events[1]["data"]["books"][0]["title"], "Dune");
    assert_http_result(
        &events[1],
        "search",
        "https://books.example.test/search?q=dune",
        "gbk",
        "sid=search; Path=/; HttpOnly",
    );

    assert_host_request(
        &events[2],
        911,
        2,
        json!({
            "url": "https://books.example.test/books/1",
            "method": "GET",
            "headers": {
                "Accept": "application/json",
                "Cookie": "sid=search",
                "X-Reader-Step": "detail"
            },
            "body": null,
            "charset": "utf-8",
            "followRedirects": true,
            "maxRedirects": 3,
            "retry": {
                "maxAttempts": 1,
                "backoffMillis": null
            },
            "usePlatformCookieJar": true,
            "session": {
                "id": "core-session-main"
            }
        }),
    );
    assert_result(&events[3], 911);
    assert_eq!(events[3]["data"]["book"]["author"], "Frank Herbert");
    assert_eq!(events[3]["data"]["book"]["intro"], "A desert planet.");
    assert_http_result(
        &events[3],
        "detail",
        "https://books.example.test/books/1?from=search",
        "utf-8",
        "sid=detail; Path=/; HttpOnly",
    );

    assert_host_request(
        &events[4],
        921,
        3,
        json!({
            "url": "https://books.example.test/books/1/toc",
            "method": "GET",
            "headers": {
                "Accept": "application/json",
                "Cookie": "sid=detail",
                "X-Reader-Step": "toc"
            },
            "body": null,
            "charset": "utf-8",
            "followRedirects": true,
            "maxRedirects": 2,
            "retry": {
                "maxAttempts": 2,
                "backoffMillis": 25
            },
            "usePlatformCookieJar": true,
            "session": {
                "id": "core-session-main"
            }
        }),
    );
    assert_result(&events[5], 921);
    assert_eq!(events[5]["data"]["toc"].as_array().unwrap().len(), 2);
    assert_eq!(
        events[5]["data"]["toc"][0]["url"],
        "https://books.example.test/books/1/chapters/1"
    );
    assert_http_result(
        &events[5],
        "toc",
        "https://books.example.test/books/1/toc",
        "utf-8",
        "sid=toc; Path=/; HttpOnly",
    );

    assert_host_request(
        &events[6],
        931,
        4,
        json!({
            "url": "https://books.example.test/books/1/chapters/1",
            "method": "GET",
            "headers": {
                "Accept": "text/html",
                "Cookie": "sid=toc",
                "X-Reader-Step": "chapter"
            },
            "body": null,
            "charset": "utf-8",
            "followRedirects": true,
            "maxRedirects": 2,
            "retry": {
                "maxAttempts": 3,
                "backoffMillis": 100
            },
            "usePlatformCookieJar": true,
            "session": {
                "id": "core-session-main"
            }
        }),
    );
    assert_result(&events[7], 931);
    assert_eq!(events[7]["data"]["via"], "rule");
    assert_eq!(
        events[7]["data"]["content"],
        "First paragraph of chapter one.\nSecond paragraph."
    );
    assert_http_result(
        &events[7],
        "chapter",
        "https://books.example.test/books/1/chapters/1",
        "utf-8",
        "sid=chapter; Path=/; HttpOnly",
    );
}

fn run_host_replay_fixture(mode: &str, name: &str) -> Vec<Value> {
    let output = Command::new(BIN)
        .arg(mode)
        .arg(fixture_path(name))
        .output()
        .expect("reader-cli binary");

    assert!(
        output.status.success(),
        "reader-cli failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a JSON event"))
        .collect()
}

fn assert_host_request(event: &Value, request_id: u64, operation_id: u64, expected_params: Value) {
    assert_eq!(event["type"], "host.request");
    assert_eq!(event["requestId"], request_id);
    assert_eq!(event["operationId"], operation_id);
    assert_eq!(event["capability"], "http.execute");
    assert_eq!(event["params"], expected_params);
}

fn assert_result(event: &Value, request_id: u64) {
    assert_eq!(event["type"], "result");
    assert_eq!(event["requestId"], request_id);
}

fn assert_http_result(
    event: &Value,
    step: &str,
    final_url: &str,
    charset_hint: &str,
    set_cookie: &str,
) {
    assert_eq!(event["data"]["http"]["status"], 200);
    assert_eq!(event["data"]["http"]["finalUrl"], final_url);
    assert_eq!(event["data"]["http"]["charsetHint"], charset_hint);
    assert_eq!(event["data"]["http"]["session"]["id"], "core-session-main");
    assert_eq!(event["data"]["http"]["headers"]["x-reader-step"], step);
    assert_eq!(
        event["data"]["http"]["headers"]["set-cookie"][0],
        set_cookie
    );
}

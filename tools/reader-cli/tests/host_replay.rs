//! Host replay smoke for request/session contract fields. This drives
//! `reader-cli --host-replay` without opening a socket and asserts that Core
//! emits the exact `http.execute` descriptor from the fixture, then resumes with
//! host-provided response diagnostics.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_reader-cli");

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/fixtures/host_replay");
    p.push(name);
    p.canonicalize().unwrap_or(p)
}

#[test]
fn host_replay_roundtrips_request_session_contract_fields() {
    let output = Command::new(BIN)
        .arg("--host-replay")
        .arg(fixture_path("request_session_search.json"))
        .output()
        .expect("reader-cli binary");

    assert!(
        output.status.success(),
        "reader-cli failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a JSON event"))
        .collect();

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

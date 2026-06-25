//! Host replay smoke for request/session contract fields. This drives
//! `reader-cli --host-replay` without opening a socket and asserts that Core
//! emits the exact `http.execute` descriptor from the fixture, then resumes with
//! host-provided response diagnostics.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

#[test]
fn host_record_outputs_replayable_single_step_fixture() {
    let recorded = run_host_record_fixture("--host-record", "request_session_search.json");

    assert_eq!(recorded["completionRequestId"], 802);
    assert_eq!(recorded["command"]["requestId"], 801);
    assert_eq!(recorded["expectHostRequest"]["capability"], "http.execute");
    assert_eq!(
        recorded["expectHostRequest"]["params"]["session"]["id"],
        "core-session-main"
    );
    assert_eq!(recorded["expectResult"]["sourceId"], "host-contract-src");
    assert_eq!(recorded["expectResult"]["http"]["status"], 200);
    assert_eq!(
        recorded["expectResult"]["http"]["finalUrl"],
        "https://books.example.test/search?q=dune"
    );

    let path = write_temp_json("host-record-single", &recorded);
    let events = run_host_replay_path("--host-replay", &path);
    let _ = fs::remove_file(&path);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "host.request");
    assert_eq!(events[1]["type"], "result");
    assert_eq!(events[1]["data"], recorded["expectResult"]);
}

#[test]
fn host_record_suite_outputs_replayable_comparison_fixture() {
    let recorded = run_host_record_fixture("--host-record-suite", "remote_reading_e2e_suite.json");
    let steps = recorded["steps"].as_array().expect("recorded steps");

    assert_eq!(steps.len(), 4);
    assert_eq!(steps[0]["name"], "book.search");
    assert_eq!(steps[0]["completionRequestId"], 1901);
    assert_eq!(steps[0]["expectHostRequest"]["capability"], "http.execute");
    assert_eq!(
        steps[0]["expectHostRequest"]["params"]["headers"]["Cookie"],
        "sid=old"
    );
    assert_eq!(steps[3]["expectResult"]["via"], "rule");
    assert_eq!(
        steps[3]["expectResult"]["http"]["headers"]["set-cookie"][0],
        "sid=chapter; Path=/; HttpOnly"
    );

    let path = write_temp_json("host-record-suite", &recorded);
    let events = run_host_replay_path("--host-replay-suite", &path);
    let _ = fs::remove_file(&path);

    assert_eq!(events.len(), 8);
    assert_eq!(events[0]["type"], "host.request");
    assert_eq!(events[7]["type"], "result");
    assert_eq!(events[7]["data"], steps[3]["expectResult"]);
}

#[test]
fn host_replay_suite_runs_legado_desensitized_corpus() {
    let events = run_host_replay_fixture(
        "--host-replay-suite",
        "legado_desensitized_corpus_suite.json",
    );

    assert_eq!(
        events.len(),
        12,
        "expected six host.request/result pairs: {events:?}"
    );

    assert_eq!(events[0]["type"], "host.request");
    assert_eq!(events[0]["requestId"], 3000);
    assert_eq!(
        events[0]["params"]["url"],
        "https://alpha.example.test/search.php?key=mirror"
    );
    assert_eq!(events[0]["params"]["method"], "POST");
    assert_eq!(events[0]["params"]["headers"]["Cookie"], "REDACTED_BOOT=1");
    assert_eq!(events[0]["params"]["charset"], "gbk");
    assert_eq!(events[0]["params"]["followRedirects"], false);
    assert_eq!(events[0]["params"]["usePlatformCookieJar"], false);
    assert_eq!(events[0]["params"]["session"]["id"], "legado-alpha-session");
    assert_result(&events[1], 3000);
    assert_eq!(events[1]["data"]["books"].as_array().unwrap().len(), 2);
    assert_eq!(events[1]["data"]["books"][0]["title"], "Mirror City");
    assert_eq!(
        events[1]["data"]["http"]["headers"]["set-cookie"][0],
        "REDACTED_SEARCH=1; Path=/; HttpOnly"
    );

    assert_eq!(events[2]["type"], "host.request");
    assert_eq!(events[2]["requestId"], 3010);
    assert_eq!(events[2]["params"]["followRedirects"], true);
    assert_eq!(events[2]["params"]["maxRedirects"], 5);
    assert_eq!(
        events[3]["data"]["http"]["finalUrl"],
        "https://alpha.example.test/mobile/book/alpha-1001"
    );
    assert_eq!(
        events[3]["data"]["book"]["intro"],
        "Desensitized detail synopsis from a real Legado-style source."
    );

    assert_eq!(events[4]["type"], "host.request");
    assert_eq!(events[4]["requestId"], 3020);
    assert_eq!(events[4]["params"]["charset"], "gb18030");
    assert_eq!(
        events[4]["params"]["headers"]["Cookie"],
        "REDACTED_DETAIL=1"
    );
    assert_result(&events[5], 3020);
    assert_eq!(events[5]["data"]["toc"].as_array().unwrap().len(), 3);
    assert_eq!(events[5]["data"]["toc"][0]["title"], "Volume 1 - Awakening");
    assert_eq!(events[5]["data"]["http"]["charsetHint"], "gb18030");

    assert_eq!(events[6]["type"], "host.request");
    assert_eq!(events[6]["requestId"], 3030);
    assert_eq!(events[6]["params"]["headers"]["Cookie"], "REDACTED_TOC=1");
    assert_eq!(events[6]["params"]["charset"], "gb18030");
    assert_eq!(events[6]["params"]["retry"]["maxAttempts"], 3);
    assert_result(&events[7], 3030);
    assert_eq!(
        events[7]["data"]["content"],
        "First line from the desensitized chapter.\nSecond line after a redirected mobile view."
    );
    assert_eq!(
        events[7]["data"]["http"]["finalUrl"],
        "https://alpha.example.test/amp/book/alpha-1001/chapter/1"
    );

    assert_eq!(events[8]["type"], "host.request");
    assert_eq!(events[8]["requestId"], 3100);
    assert_eq!(
        events[8]["params"]["url"],
        "https://beta.example.test/modules/article/search.php?searchkey=nebula"
    );
    assert_eq!(events[8]["params"]["charset"], "gbk");
    assert_eq!(events[8]["params"]["session"]["id"], "legado-beta-session");
    assert_result(&events[9], 3100);
    assert_eq!(events[9]["data"]["books"][0]["bookId"], "");
    assert_eq!(events[9]["data"]["books"][0]["title"], "Nebula Archive");
    assert_eq!(events[9]["data"]["books"][1]["title"], "Silent Harbor");

    assert_eq!(events[10]["type"], "host.request");
    assert_eq!(events[10]["requestId"], 3110);
    assert_eq!(
        events[10]["params"]["headers"]["Cookie"],
        "REDACTED_BETA_SEARCH=1"
    );
    assert_eq!(events[10]["params"]["session"]["id"], "legado-beta-session");
    assert_result(&events[11], 3110);
    assert_eq!(events[11]["data"]["book"]["author"], "Author C");
    assert_eq!(
        events[11]["data"]["http"]["headers"]["set-cookie"][0],
        "REDACTED_BETA_DETAIL=1; Path=/; HttpOnly"
    );
}

fn run_host_replay_fixture(mode: &str, name: &str) -> Vec<Value> {
    run_host_replay_path(mode, &fixture_path(name))
}

fn run_host_replay_path(mode: &str, path: &PathBuf) -> Vec<Value> {
    let stdout = run_cli_output(mode, path);
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a JSON event"))
        .collect()
}

fn run_host_record_fixture(mode: &str, name: &str) -> Value {
    let stdout = run_cli_output(mode, &fixture_path(name));
    serde_json::from_str(&stdout).expect("record output is a JSON fixture")
}

fn run_cli_output(mode: &str, path: &PathBuf) -> String {
    let output = Command::new(BIN)
        .arg(mode)
        .arg(path)
        .output()
        .expect("reader-cli binary");

    assert!(
        output.status.success(),
        "reader-cli failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn write_temp_json(prefix: &str, value: &Value) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!(
        "reader-cli-{prefix}-{}-{stamp}.json",
        std::process::id()
    ));
    fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    path
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

//! End-to-end smoke for the remote-reading vertical: drives `reader-cli
//! --fixture-vertical` through the full import → search → host-http search →
//! detail → toc → chapter → progress pipeline and the JS-unsupported path,
//! asserting each emitted event shape.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_reader-cli");

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/fixtures/remote_source/basic_source.json");
    p.canonicalize().unwrap_or(p)
}

#[test]
fn fixture_vertical_runs_full_pipeline() {
    let output = Command::new(BIN)
        .arg("--fixture-vertical")
        .arg(fixture_path())
        .output()
        .expect("reader-cli binary");

    assert!(
        output.status.success(),
        "reader-cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a JSON event"))
        .collect();

    // 9 events: import, search(inline), host.request(http), search(host body),
    // detail, toc, chapter(rule), progress, js(unsupported).
    assert_eq!(events.len(), 9, "expected 9 events, got {events:?}");

    assert_eq!(events[0]["data"]["imported"], true);
    assert_eq!(events[0]["data"]["sourceId"], "basic-src");

    let books = events[1]["data"]["books"].as_array().unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[0]["title"], "Dune");

    assert_eq!(events[2]["type"], "host.request");
    assert_eq!(events[2]["capability"], "http.execute");
    assert_eq!(
        events[2]["params"]["url"],
        "https://books.example.test/search?q=dune"
    );

    let host_books = events[3]["data"]["books"].as_array().unwrap();
    assert_eq!(host_books.len(), 2);
    assert_eq!(host_books[0]["title"], "Dune");
    assert_eq!(events[3]["data"]["http"]["status"], 200);
    assert_eq!(
        events[3]["data"]["http"]["headers"]["content-type"],
        "application/json"
    );

    assert_eq!(events[4]["data"]["book"]["author"], "Frank Herbert");
    assert_eq!(events[4]["data"]["book"]["intro"], "A desert planet.");

    let toc = events[5]["data"]["toc"].as_array().unwrap();
    assert_eq!(toc.len(), 2);

    assert_eq!(events[6]["data"]["via"], "rule");
    assert!(events[6]["data"]["content"]
        .as_str()
        .unwrap()
        .contains("First paragraph"));

    assert_eq!(events[7]["data"]["stored"], true);

    // JS unsupported: structured error, never a fake network result.
    assert_eq!(events[8]["type"], "error");
    assert_eq!(events[8]["error"]["code"], "INTERNAL");
    assert_eq!(events[8]["error"]["details"]["unsupported"], true);
}

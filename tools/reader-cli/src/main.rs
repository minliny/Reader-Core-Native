//! `reader-cli` — minimal Core protocol driver.
//!
//! This tool intentionally exercises the same JSON command path that hosts use:
//! `--info`, `--ping`, `--host-smoke`, `--json`, and `--stdin` all enqueue a
//! command and print emitted events as one JSON object per line.

mod conformance;

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use reader_contract::{methods, Command, CoreError, Event};
use reader_runtime::{EventSink, Runtime};

const EVENT_TIMEOUT: Duration = Duration::from_secs(2);

struct ChannelSink {
    tx: mpsc::Sender<Event>,
}

impl EventSink for ChannelSink {
    fn emit(&self, event: &Event) {
        let _ = self.tx.send(event.clone());
    }
}

enum Mode {
    Info,
    Ping,
    Status,
    HostSmoke,
    Conformance,
    Json(String),
    Stdin,
    /// Run the full remote-reading vertical pipeline against a fixture file.
    /// See `tests/fixtures/remote_source/basic_source.json` for the shape.
    FixtureVertical(PathBuf),
}

fn main() {
    if let Err(error) = run() {
        print_event(&Event::error(0, error));
        std::process::exit(2);
    }
}

fn run() -> Result<(), CoreError> {
    let (mode, config_json) = parse_args(std::env::args().skip(1))?;
    let (tx, rx) = mpsc::channel();
    let sink = Arc::new(ChannelSink { tx });
    let runtime = match config_json {
        Some(json) => Runtime::new_with_config_json(sink, json.as_bytes())?,
        None => Runtime::new(sink),
    };

    match mode {
        Mode::Info => run_command(
            &runtime,
            &rx,
            Command::new(1, methods::CORE_INFO, serde_json::json!({})),
        ),
        Mode::Ping => run_command(
            &runtime,
            &rx,
            Command::new(1, methods::RUNTIME_PING, serde_json::json!({})),
        ),
        Mode::Status => run_command(
            &runtime,
            &rx,
            Command::new(1, methods::RUNTIME_STATUS, serde_json::json!({})),
        ),
        Mode::Json(json) => {
            runtime.send_json(json.as_bytes())?;
            print_next_event(&rx)
        }
        Mode::Stdin => {
            let mut json = String::new();
            io::stdin().read_to_string(&mut json).map_err(|err| {
                CoreError::invalid_message("failed to read stdin")
                    .with_details(serde_json::json!({ "source": err.to_string() }))
            })?;
            runtime.send_json(json.as_bytes())?;
            print_next_event(&rx)
        }
        Mode::HostSmoke => run_host_smoke(&runtime, &rx),
        Mode::Conformance => {
            let report = conformance::run_conformance();
            println!("{}", report.to_json());
            if report.failed_count() == 0 {
                Ok(())
            } else {
                Err(CoreError::internal(format!(
                    "{} conformance case(s) failed",
                    report.failed_count()
                )))
            }
        }
        Mode::FixtureVertical(path) => run_fixture_vertical(&runtime, &rx, &path),
    }
}

fn parse_args<I>(args: I) -> Result<(Mode, Option<String>), CoreError>
where
    I: IntoIterator<Item = String>,
{
    let mut mode = None;
    let mut config_json = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--info" => set_mode(&mut mode, Mode::Info)?,
            "--ping" => set_mode(&mut mode, Mode::Ping)?,
            "--status" => set_mode(&mut mode, Mode::Status)?,
            "--host-smoke" => set_mode(&mut mode, Mode::HostSmoke)?,
            "--conformance" => set_mode(&mut mode, Mode::Conformance)?,
            "--stdin" => set_mode(&mut mode, Mode::Stdin)?,
            "--fixture-vertical" => {
                let Some(path) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--fixture-vertical requires a path to a fixture JSON file",
                    ));
                };
                set_mode(&mut mode, Mode::FixtureVertical(PathBuf::from(path)))?;
            }
            "--json" => {
                let Some(json) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--json requires a command payload",
                    ));
                };
                set_mode(&mut mode, Mode::Json(json))?;
            }
            "--config-json" => {
                let Some(json) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--config-json requires a config payload",
                    ));
                };
                config_json = Some(json);
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                return Err(CoreError::invalid_message(format!(
                    "unknown CLI option: {other}"
                )));
            }
        }
    }

    Ok((mode.unwrap_or(Mode::Info), config_json))
}

fn set_mode(slot: &mut Option<Mode>, mode: Mode) -> Result<(), CoreError> {
    if slot.is_some() {
        return Err(CoreError::invalid_message(
            "only one of --info, --ping, --status, --host-smoke, --conformance, --json, --stdin, or --fixture-vertical may be used",
        ));
    }
    *slot = Some(mode);
    Ok(())
}

fn run_command(runtime: &Runtime, rx: &Receiver<Event>, command: Command) -> Result<(), CoreError> {
    runtime.send(command)?;
    print_next_event(rx)
}

fn run_host_smoke(runtime: &Runtime, rx: &Receiver<Event>) -> Result<(), CoreError> {
    runtime.send(Command::new(
        1,
        methods::RUNTIME_HOST_SMOKE,
        serde_json::json!({
            "capability": "host.smoke.echo",
            "params": { "message": "reader-cli host smoke" }
        }),
    ))?;

    let host_request = recv_event(rx)?;
    print_event(&host_request);

    let operation_id = match host_request {
        Event::HostRequest { operation_id, .. } => operation_id,
        other => {
            return Err(CoreError::internal(format!(
                "expected host.request event, got {other:?}"
            )));
        }
    };

    runtime.send(Command::new(
        2,
        methods::HOST_COMPLETE,
        serde_json::json!({
            "operationId": operation_id,
            "result": { "status": "ok", "completedBy": "reader-cli" }
        }),
    ))?;

    print_next_event(rx)
}

fn run_fixture_vertical(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    path: &PathBuf,
) -> Result<(), CoreError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CoreError::invalid_message(format!("failed to read fixture {}", path.display()))
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })?;
    let fixture: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CoreError::invalid_message(format!("fixture {} is not valid JSON", path.display()))
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })?;

    let source = fixture
        .get("source")
        .cloned()
        .ok_or_else(|| CoreError::invalid_message("fixture missing `source`"))?;
    let book_id = fixture
        .get("bookId")
        .and_then(|v| v.as_str())
        .unwrap_or("1")
        .to_string();
    let chapter_title = fixture
        .get("chapterTitle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let search_response = fixture
        .get("searchResponse")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let detail_response = fixture
        .get("detailResponse")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let toc_response = fixture
        .get("tocResponse")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let chapter_response = fixture
        .get("chapterResponse")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // 1. source.import — inline source wins over storage lookup downstream.
    runtime.send(Command::new(
        1,
        methods::SOURCE_IMPORT,
        serde_json::json!({
            "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
            "name": source.get("name").cloned().unwrap_or_default(),
            "baseUrl": source.get("baseUrl").cloned().unwrap_or_default(),
            "rules": source.get("rules").cloned().unwrap_or(serde_json::Value::Null),
        }),
    ))?;
    print_next_event(rx)?;

    // 2. book.search
    runtime.send(Command::new(
        2,
        methods::BOOK_SEARCH,
        serde_json::json!({
            "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
            "searchResponse": search_response,
            "source": source,
        }),
    ))?;
    print_next_event(rx)?;

    // 3. book.search via host HTTP — Core emits http.execute, then continues
    //    parsing after the host completes with a response body.
    let base_url = source
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("https://books.example.test");
    runtime.send(Command::new(
        3,
        methods::BOOK_SEARCH,
        serde_json::json!({
            "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
            "searchRequest": {
                "url": format!("{base_url}/search?q=dune"),
                "headers": { "Accept": "application/json" }
            },
            "source": source,
        }),
    ))?;
    let host_request = recv_event(rx)?;
    print_event(&host_request);
    let operation_id = match host_request {
        Event::HostRequest {
            operation_id,
            capability,
            ..
        } if capability == "http.execute" => operation_id,
        other => {
            return Err(CoreError::internal(format!(
                "expected http.execute host.request event, got {other:?}"
            )));
        }
    };
    runtime.send(Command::new(
        4,
        methods::HOST_COMPLETE,
        serde_json::json!({
            "operationId": operation_id,
            "result": {
                "status": 200,
                "headers": { "content-type": "application/json" },
                "body": search_response
            }
        }),
    ))?;
    print_next_event(rx)?;

    // 4. book.detail — merge into a base book carrying bookId.
    let base_book = serde_json::json!({ "bookId": book_id });
    runtime.send(Command::new(
        5,
        methods::BOOK_DETAIL,
        serde_json::json!({
            "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
            "book": base_book,
            "detailResponse": detail_response,
            "source": source,
        }),
    ))?;
    print_next_event(rx)?;

    // 5. book.toc
    runtime.send(Command::new(
        6,
        methods::BOOK_TOC,
        serde_json::json!({
            "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
            "bookId": book_id,
            "tocResponse": toc_response,
            "source": source,
        }),
    ))?;
    print_next_event(rx)?;

    // 6. chapter.content (rule path)
    runtime.send(Command::new(
        7,
        methods::CHAPTER_CONTENT,
        serde_json::json!({
            "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
            "bookId": book_id,
            "chapterTitle": chapter_title,
            "chapterResponse": chapter_response,
            "source": source,
        }),
    ))?;
    print_next_event(rx)?;

    // 7. reading.progress.update
    runtime.send(Command::new(
        8,
        methods::READING_PROGRESS_UPDATE,
        serde_json::json!({
            "bookId": book_id,
            "chapterIndex": 0,
            "chapterOffset": 0,
            "chapterProgress": 0.25,
        }),
    ))?;
    print_next_event(rx)?;

    // 8. chapter.content (JS unsupported path) — a JS rule that calls java.get
    //    with no registered host callback must surface a structured unsupported
    //    error, never a fake network result.
    if let Some(js_rule) = fixture.get("jsRuleUnsupported").and_then(|v| v.as_str()) {
        runtime.send(Command::new(
            9,
            methods::CHAPTER_CONTENT,
            serde_json::json!({
                "sourceId": source.get("sourceId").cloned().unwrap_or_default(),
                "bookId": book_id,
                "chapterTitle": chapter_title,
                "chapterResponse": chapter_response,
                "jsRule": js_rule,
                "source": source,
            }),
        ))?;
        print_next_event(rx)?;
    }

    Ok(())
}

fn print_next_event(rx: &Receiver<Event>) -> Result<(), CoreError> {
    let event = recv_event(rx)?;
    print_event(&event);
    Ok(())
}

fn recv_event(rx: &Receiver<Event>) -> Result<Event, CoreError> {
    rx.recv_timeout(EVENT_TIMEOUT)
        .map_err(|_| CoreError::internal("timed out waiting for runtime event"))
}

fn print_event(event: &Event) {
    println!("{}", serde_json::to_string(event).unwrap_or_default());
}

fn print_usage() {
    eprintln!(
        "usage: reader-cli [--info|--ping|--status|--host-smoke|--conformance|--json '<command>'|--stdin|--fixture-vertical <path>] [--config-json '<config>']"
    );
}

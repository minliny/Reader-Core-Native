//! `reader-cli` — minimal Core protocol driver.
//!
//! This tool intentionally exercises the same JSON command path that hosts use:
//! `--info`, `--ping`, `--host-smoke`, `--json`, and `--stdin` all enqueue a
//! command and print emitted events as one JSON object per line.

mod conformance;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use reader_content::{BookSourceRequestContext, RemoteContentPipeline};
use reader_contract::{methods, Command, CoreError, Event, HostCapability};
use reader_domain::{Book, BookSourceSemantics, LegadoBookSource};
use reader_runtime::{EventSink, Runtime};
use serde_json::Value;

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
    /// Run the BookSource semantic pipeline directly and print stable JSON.
    BookSourceFixture(PathBuf),
    /// Replay a host HTTP request/complete pair against Core without opening a
    /// real socket.
    HostReplay(PathBuf),
    /// Replay a sequence of host HTTP request/complete pairs against one
    /// runtime instance.
    HostReplaySuite(PathBuf),
    /// Execute a host replay input and print a normalized fixture with
    /// recorded host request/result expectations.
    HostRecord(PathBuf),
    /// Execute a host replay suite input and print a normalized suite fixture
    /// with recorded expectations.
    HostRecordSuite(PathBuf),
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
        Mode::BookSourceFixture(path) => run_booksource_fixture(&path),
        Mode::HostReplay(path) => run_host_replay(&runtime, &rx, &path),
        Mode::HostReplaySuite(path) => run_host_replay_suite(&runtime, &rx, &path),
        Mode::HostRecord(path) => run_host_record(&runtime, &rx, &path),
        Mode::HostRecordSuite(path) => run_host_record_suite(&runtime, &rx, &path),
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
            "--booksource-fixture" => {
                let Some(path) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--booksource-fixture requires a path to a fixture JSON file",
                    ));
                };
                set_mode(&mut mode, Mode::BookSourceFixture(PathBuf::from(path)))?;
            }
            "--host-replay" => {
                let Some(path) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--host-replay requires a path to a fixture JSON file",
                    ));
                };
                set_mode(&mut mode, Mode::HostReplay(PathBuf::from(path)))?;
            }
            "--host-replay-suite" => {
                let Some(path) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--host-replay-suite requires a path to a fixture JSON file",
                    ));
                };
                set_mode(&mut mode, Mode::HostReplaySuite(PathBuf::from(path)))?;
            }
            "--host-record" => {
                let Some(path) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--host-record requires a path to a fixture JSON file",
                    ));
                };
                set_mode(&mut mode, Mode::HostRecord(PathBuf::from(path)))?;
            }
            "--host-record-suite" => {
                let Some(path) = iter.next() else {
                    return Err(CoreError::invalid_message(
                        "--host-record-suite requires a path to a fixture JSON file",
                    ));
                };
                set_mode(&mut mode, Mode::HostRecordSuite(PathBuf::from(path)))?;
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
            "only one of --info, --ping, --status, --host-smoke, --conformance, --json, --stdin, --fixture-vertical, --booksource-fixture, --host-replay, --host-replay-suite, --host-record, or --host-record-suite may be used",
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
        } if capability == HostCapability::HttpExecute => operation_id,
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

fn run_booksource_fixture(path: &PathBuf) -> Result<(), CoreError> {
    let fixture = read_json_fixture(path, "BookSource fixture")?;
    let book_source_value = fixture
        .get("bookSource")
        .cloned()
        .ok_or_else(|| CoreError::invalid_message("BookSource fixture missing `bookSource`"))?;
    let book_source: LegadoBookSource =
        serde_json::from_value(book_source_value).map_err(|err| {
            CoreError::invalid_message("BookSource fixture `bookSource` is invalid")
                .with_details(serde_json::json!({ "source": err.to_string() }))
        })?;

    let source_id =
        fixture_string(&fixture, "sourceId").unwrap_or_else(|| "booksource-fixture".to_string());
    let semantics = BookSourceSemantics::from_legado(
        &source_id,
        fixture.get("name").and_then(Value::as_str),
        fixture.get("baseUrl").and_then(Value::as_str),
        &book_source,
    );
    let pipeline = RemoteContentPipeline::new();
    let mut context = fixture_context(&fixture, &semantics);

    let search = pipeline
        .search_book_source(
            &semantics,
            fixture_string(&fixture, "searchResponse")
                .as_deref()
                .unwrap_or_default(),
            &context,
        )
        .map_err(content_error)?;
    if let Some(book) = search.first() {
        context.book_url = book.book_id.clone();
    }

    let explore = pipeline
        .explore_book_source(
            &semantics,
            fixture_string(&fixture, "exploreResponse")
                .as_deref()
                .unwrap_or_default(),
            &context,
        )
        .map_err(content_error)?;

    let base_book = fixture
        .get("book")
        .cloned()
        .map(serde_json::from_value::<Book>)
        .transpose()
        .map_err(|err| {
            CoreError::invalid_message("BookSource fixture `book` is invalid")
                .with_details(serde_json::json!({ "source": err.to_string() }))
        })?
        .or_else(|| search.first().cloned())
        .unwrap_or_else(|| Book {
            book_id: context.book_url.clone(),
            title: String::new(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        });

    let detail = pipeline
        .detail_book_source(
            &semantics,
            &base_book,
            fixture_string(&fixture, "detailResponse")
                .as_deref()
                .unwrap_or_default(),
            &context,
        )
        .map_err(content_error)?;
    if let Some(toc_url) = detail.toc_url.as_ref() {
        context.current_url = toc_url.clone();
    }

    let toc = pipeline
        .toc_book_source(
            &semantics,
            fixture_string(&fixture, "tocResponse")
                .as_deref()
                .unwrap_or_default(),
            &context,
        )
        .map_err(content_error)?;
    if let Some(chapter) = toc.chapters.first() {
        context.chapter_url = chapter.url.clone();
        context.current_url = chapter.url.clone();
    }

    let content = pipeline
        .content_book_source(
            &semantics,
            fixture_string(&fixture, "contentResponse")
                .as_deref()
                .unwrap_or_default(),
            &context,
        )
        .map_err(content_error)?;

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "sourceId": semantics.source_id.clone(),
            "source": semantics,
            "search": { "books": search },
            "explore": explore,
            "detail": detail,
            "toc": toc,
            "content": content,
        }))
        .unwrap_or_default()
    );
    Ok(())
}

fn fixture_context(fixture: &Value, semantics: &BookSourceSemantics) -> BookSourceRequestContext {
    let mut variables = fixture_variables(fixture);
    variables
        .entry("sourceId".into())
        .or_insert_with(|| semantics.source_id.clone());
    BookSourceRequestContext {
        base_url: fixture_string(fixture, "baseUrl").unwrap_or_else(|| semantics.base_url.clone()),
        current_url: fixture_string(fixture, "currentUrl")
            .unwrap_or_else(|| semantics.base_url.clone()),
        book_url: fixture_string(fixture, "bookUrl").unwrap_or_default(),
        chapter_url: fixture_string(fixture, "chapterUrl").unwrap_or_default(),
        variables,
    }
}

fn fixture_variables(fixture: &Value) -> BTreeMap<String, String> {
    fixture
        .get("variables")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.into())))
                .collect()
        })
        .unwrap_or_default()
}

fn fixture_string(fixture: &Value, key: &str) -> Option<String> {
    fixture
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn content_error(err: reader_content::ContentError) -> CoreError {
    CoreError::internal(err.to_string())
}

fn run_host_replay(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    path: &PathBuf,
) -> Result<(), CoreError> {
    let fixture = read_json_fixture(path, "host replay fixture")?;
    run_host_replay_step(runtime, rx, &fixture)
}

fn run_host_replay_suite(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    path: &PathBuf,
) -> Result<(), CoreError> {
    let fixture = read_json_fixture(path, "host replay suite fixture")?;
    let steps = fixture
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::invalid_message("host replay suite fixture missing `steps`"))?;
    if steps.is_empty() {
        return Err(CoreError::invalid_message(
            "host replay suite fixture `steps` must not be empty",
        ));
    }
    for step in steps {
        run_host_replay_step(runtime, rx, step)?;
    }
    Ok(())
}

fn run_host_record(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    path: &PathBuf,
) -> Result<(), CoreError> {
    let fixture = read_json_fixture(path, "host record fixture")?;
    let recording = record_host_replay_step(runtime, rx, &fixture)?;
    print_json_value(&recording);
    Ok(())
}

fn run_host_record_suite(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    path: &PathBuf,
) -> Result<(), CoreError> {
    let fixture = read_json_fixture(path, "host record suite fixture")?;
    let steps = fixture
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::invalid_message("host record suite fixture missing `steps`"))?;
    if steps.is_empty() {
        return Err(CoreError::invalid_message(
            "host record suite fixture `steps` must not be empty",
        ));
    }
    let recorded_steps = steps
        .iter()
        .map(|step| record_host_replay_step(runtime, rx, step))
        .collect::<Result<Vec<_>, _>>()?;
    print_json_value(&serde_json::json!({ "steps": recorded_steps }));
    Ok(())
}

fn read_json_fixture(path: &PathBuf, label: &str) -> Result<Value, CoreError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CoreError::invalid_message(format!("failed to read {label} {}", path.display()))
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        CoreError::invalid_message(format!("{label} {} is not valid JSON", path.display()))
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })
}

fn parse_replay_command(fixture: &Value) -> Result<(Command, Value), CoreError> {
    let command_value = fixture
        .get("command")
        .cloned()
        .ok_or_else(|| CoreError::invalid_message("host replay fixture missing `command`"))?;
    let command_json = serde_json::to_vec(&command_value).map_err(|err| {
        CoreError::invalid_message("host replay command is not serializable")
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })?;
    let command = Command::from_json_bytes(&command_json)?;
    Ok((command, command_value))
}

fn replay_completion_request_id(command: &Command, fixture: &Value) -> Result<u64, CoreError> {
    match fixture.get("completionRequestId") {
        Some(value) => value.as_u64().filter(|id| *id > 0).ok_or_else(|| {
            CoreError::invalid_message("host replay completionRequestId must be a positive integer")
        }),
        None => command.request_id.checked_add(1).ok_or_else(|| {
            CoreError::invalid_message("host replay command requestId is too large")
        }),
    }
}

fn host_result_for_replay(fixture: &Value) -> Result<Value, CoreError> {
    fixture
        .get("hostResult")
        .cloned()
        .ok_or_else(|| CoreError::invalid_message("host replay fixture missing `hostResult`"))
}

fn run_host_replay_step(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    fixture: &Value,
) -> Result<(), CoreError> {
    let (command, _) = parse_replay_command(fixture)?;
    let completion_request_id = replay_completion_request_id(&command, fixture)?;

    runtime.send(command)?;

    let host_request = recv_event(rx)?;
    print_event(&host_request);

    let operation_id =
        match &host_request {
            Event::HostRequest {
                operation_id,
                capability,
                params,
                ..
            } => {
                if let Some(expected) = fixture.get("expectHostRequest") {
                    let actual_capability = Value::String(capability.as_str().to_string());
                    if expected.get("capability") != Some(&actual_capability) {
                        return Err(CoreError::internal("host replay capability mismatch")
                            .with_details(serde_json::json!({
                                "expected": expected.get("capability"),
                                "actual": capability.as_str(),
                            })));
                    }
                    if expected.get("params") != Some(params) {
                        return Err(CoreError::internal("host replay params mismatch")
                            .with_details(serde_json::json!({
                                "expected": expected.get("params"),
                                "actual": params,
                            })));
                    }
                }
                *operation_id
            }
            other => {
                return Err(CoreError::internal(format!(
                    "expected host.request event, got {other:?}"
                )));
            }
        };

    let host_result = host_result_for_replay(fixture)?;
    runtime.send(Command::new(
        completion_request_id,
        methods::HOST_COMPLETE,
        serde_json::json!({
            "operationId": operation_id,
            "result": host_result,
        }),
    ))?;

    let result = recv_event(rx)?;
    print_event(&result);

    if let Some(expected_result) = fixture.get("expectResult") {
        match &result {
            Event::Result { data, .. } if data == expected_result => {}
            Event::Result { data, .. } => {
                return Err(
                    CoreError::internal("host replay result data mismatch").with_details(
                        serde_json::json!({
                            "expected": expected_result,
                            "actual": data,
                        }),
                    ),
                );
            }
            other => {
                return Err(CoreError::internal(format!(
                    "expected result event after host.complete, got {other:?}"
                )));
            }
        }
    }

    if let Some(expected_http) = fixture.get("expectResultHttp") {
        match &result {
            Event::Result { data, .. } if data.get("http") == Some(expected_http) => {}
            Event::Result { data, .. } => {
                return Err(
                    CoreError::internal("host replay result http mismatch").with_details(
                        serde_json::json!({
                            "expected": expected_http,
                            "actual": data.get("http"),
                        }),
                    ),
                );
            }
            other => {
                return Err(CoreError::internal(format!(
                    "expected result event after host.complete, got {other:?}"
                )));
            }
        }
    }

    Ok(())
}

fn record_host_replay_step(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    fixture: &Value,
) -> Result<Value, CoreError> {
    let (command, command_value) = parse_replay_command(fixture)?;
    let completion_request_id = replay_completion_request_id(&command, fixture)?;

    runtime.send(command)?;
    let host_request = recv_event(rx)?;
    let (operation_id, expect_host_request) = match host_request {
        Event::HostRequest {
            operation_id,
            capability,
            params,
            ..
        } => (
            operation_id,
            serde_json::json!({
                "capability": capability.as_str(),
                "params": params,
            }),
        ),
        other => {
            return Err(CoreError::internal(format!(
                "expected host.request event, got {other:?}"
            )));
        }
    };

    let host_result = host_result_for_replay(fixture)?;
    runtime.send(Command::new(
        completion_request_id,
        methods::HOST_COMPLETE,
        serde_json::json!({
            "operationId": operation_id,
            "result": host_result,
        }),
    ))?;

    let result = recv_event(rx)?;
    let expect_result = match result {
        Event::Result { data, .. } => data,
        Event::Error { error, .. } => {
            return Err(
                CoreError::internal("host record expected result event").with_details(
                    serde_json::json!({
                        "error": error,
                    }),
                ),
            );
        }
        other => {
            return Err(CoreError::internal(format!(
                "expected result event after host.complete, got {other:?}"
            )));
        }
    };

    let mut output = serde_json::Map::new();
    if let Some(name) = fixture.get("name").cloned() {
        output.insert("name".to_string(), name);
    }
    output.insert(
        "completionRequestId".to_string(),
        serde_json::json!(completion_request_id),
    );
    output.insert("command".to_string(), command_value);
    output.insert("expectHostRequest".to_string(), expect_host_request);
    output.insert("hostResult".to_string(), host_result);
    output.insert("expectResult".to_string(), expect_result);
    Ok(Value::Object(output))
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

fn print_json_value(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

fn print_usage() {
    eprintln!(
        "usage: reader-cli [--info|--ping|--status|--host-smoke|--conformance|--json '<command>'|--stdin|--fixture-vertical <path>|--booksource-fixture <path>|--host-replay <path>|--host-replay-suite <path>|--host-record <path>|--host-record-suite <path>] [--config-json '<config>']"
    );
}

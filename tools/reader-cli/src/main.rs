//! `reader-cli` — minimal Core protocol driver.
//!
//! This tool intentionally exercises the same JSON command path that hosts use:
//! `--info`, `--ping`, `--host-smoke`, `--json`, and `--stdin` all enqueue a
//! command and print emitted events as one JSON object per line.

use std::io::{self, Read};
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
    HostSmoke,
    Json(String),
    Stdin,
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
            "--host-smoke" => set_mode(&mut mode, Mode::HostSmoke)?,
            "--stdin" => set_mode(&mut mode, Mode::Stdin)?,
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
            "only one of --info, --ping, --host-smoke, --json, or --stdin may be used",
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
        "usage: reader-cli [--info|--ping|--host-smoke|--json '<command>'|--stdin] [--config-json '<config>']"
    );
}

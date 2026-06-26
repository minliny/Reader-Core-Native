//! `host-replay` — offline replay of `host.request` responses.
//!
//! Reads local replay fixtures and emits `host.complete` / `host.error` JSON
//! commands. No network. See `src/lib.rs` for the fixture format.
//!
//! # Commands
//!
//! - `show <fixture.json>` — emit the command for one fixture.
//! - `replay` — read `host.request` JSON lines from stdin, match against loaded
//!   fixtures, emit `host.complete`/`host.error` JSON lines to stdout.
//! - `list` — list loaded fixtures and their match keys.
//! - `validate <fixture.json>` — check internal consistency.
//!
//! # Common flags
//!
//! - `--dir <dir>` — fixture directory (default: `samples/host-replay`).
//! - `--fixture <file>` — add a single fixture (repeatable; merged with `--dir`).
//! - `--trace` — print redirect-chain trace to stderr.
//! - `--update-jar <file>` — merge response `Set-Cookie` into a jar file.
//! - `--pretty` — pretty-print JSON output.
//! - `--request-id <n>` — host command requestId for `show` (default: 1).

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

use host_replay::*;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("host-replay: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Debug, Default)]
struct CommonOpts {
    dir: Option<PathBuf>,
    fixtures: Vec<PathBuf>,
    trace: bool,
    update_jar: Option<PathBuf>,
    pretty: bool,
    request_id: u64,
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).peekable();
    let command = args
        .next()
        .ok_or_else(|| usage_error())?
        .trim()
        .to_string();

    let mut common = CommonOpts {
        request_id: 1,
        ..Default::default()
    };

    match command.as_str() {
        "--help" | "-h" | "help" => {
            print_help();
            return Ok(());
        }
        "show" => {
            let fixture_path = parse_show(&mut args, &mut common)?;
            let fixture = load_fixture(&fixture_path).map_err(|e| e.to_string())?;
            validate(&fixture).map_err(|e| e.to_string())?;
            let dir = fixture_path.parent().unwrap_or(Path::new("."));
            emit(&fixture, dir, &common, common.request_id, fixture.request.operation_id.unwrap_or(1))?;
        }
        "replay" => {
            parse_common(&mut args, &mut common)?;
            let fixtures = load_all(&common)?;
            run_replay(&fixtures, &common)?;
        }
        "list" => {
            parse_common(&mut args, &mut common)?;
            let fixtures = load_all(&common)?;
            run_list(&fixtures);
        }
        "validate" => {
            let fixture_path = parse_show(&mut args, &mut common)?;
            let fixture = load_fixture(&fixture_path).map_err(|e| e.to_string())?;
            validate(&fixture).map_err(|e| e.to_string())?;
            println!("ok: {}", fixture_path.display());
        }
        other => {
            return Err(format!("unknown command: {other}\n\n{}", usage_text()));
        }
    }
    Ok(())
}

fn parse_show<I: Iterator<Item = String>>(
    args: &mut std::iter::Peekable<I>,
    common: &mut CommonOpts,
) -> Result<PathBuf, String> {
    let mut path: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--trace" => common.trace = true,
            "--pretty" => common.pretty = true,
            "--update-jar" => {
                common.update_jar = Some(PathBuf::from(args.next().ok_or("--update-jar needs a path")?));
            }
            "--request-id" => {
                common.request_id =
                    args.next().ok_or("--request-id needs a value")?.parse().map_err(|_| "invalid --request-id")?;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            other => {
                if path.is_some() {
                    return Err(format!("unexpected extra argument: {other}"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    path.ok_or_else(|| "show requires a fixture path".to_string())
}

fn parse_common<I: Iterator<Item = String>>(
    args: &mut std::iter::Peekable<I>,
    common: &mut CommonOpts,
) -> Result<(), String> {
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => {
                common.dir = Some(PathBuf::from(args.next().ok_or("--dir needs a path")?));
            }
            "--fixture" => {
                common
                    .fixtures
                    .push(PathBuf::from(args.next().ok_or("--fixture needs a path")?));
            }
            "--trace" => common.trace = true,
            "--pretty" => common.pretty = true,
            "--update-jar" => {
                common.update_jar = Some(PathBuf::from(args.next().ok_or("--update-jar needs a path")?));
            }
            "--request-id" => {
                common.request_id =
                    args.next().ok_or("--request-id needs a value")?.parse().map_err(|_| "invalid --request-id")?;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok(())
}

/// Load fixtures from `--dir` (default `samples/host-replay`) plus any `--fixture`.
fn load_all(common: &CommonOpts) -> Result<Vec<(PathBuf, Fixture)>, String> {
    let mut out = Vec::new();
    let dir = common
        .dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("samples/host-replay"));
    if dir.is_dir() {
        out.extend(load_fixture_dir(&dir).map_err(|e| e.to_string())?);
    } else if common.dir.is_some() {
        return Err(format!("--dir {} is not a directory", dir.display()));
    }
    for f in &common.fixtures {
        let fixture = load_fixture(f).map_err(|e| e.to_string())?;
        out.push((f.clone(), fixture));
    }
    if out.is_empty() {
        return Err(format!(
            "no fixtures found (looked in {} and {} --fixture)",
            dir.display(),
            common.fixtures.len()
        ));
    }
    Ok(out)
}

fn run_replay(
    fixtures: &[(PathBuf, Fixture)],
    common: &CommonOpts,
) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut request_id_counter = common.request_id.max(1);
    let mut matched = 0usize;
    let mut unmatched = 0usize;

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("read stdin: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let incoming = match parse_incoming(&line) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("host-replay: skipping line: {e}");
                continue;
            }
        };

        let hit = fixtures
            .iter()
            .find(|(_, f)| matches(f, &incoming));

        match hit {
            Some((path, fixture)) => {
                let dir = path.parent().unwrap_or(Path::new("."));
                let operation_id = incoming.operation_id;
                let cmd = build_command(fixture, dir, request_id_counter, operation_id)
                    .map_err(|e| e.to_string())?;
                write_json(&mut out, &cmd, common.pretty)?;
                writeln!(out).map_err(|e| e.to_string())?;
                out.flush().map_err(|e| e.to_string())?;

                if common.trace {
                    if let Some(trace) = redirect_trace(fixture) {
                        eprintln!(
                            "trace: {}",
                            serde_json::to_string(&trace).map_err(|e| e.to_string())?
                        );
                    }
                }
                if let Some(jar_path) = &common.update_jar {
                    update_jar(fixture, &incoming, jar_path)?;
                }
                request_id_counter += 1;
                matched += 1;
            }
            None => {
                unmatched += 1;
                eprintln!(
                    "host-replay: no fixture matched operationId={} capability={} url={}",
                    incoming.operation_id,
                    incoming.capability,
                    incoming.params.get("url").and_then(|v| v.as_str()).unwrap_or("")
                );
            }
        }
    }

    eprintln!(
        "host-replay: replay done — matched={matched} unmatched={unmatched}",
    );
    Ok(())
}

fn run_list(fixtures: &[(PathBuf, Fixture)]) {
    for (path, f) in fixtures {
        let url = f
            .request
            .url_pattern
            .clone()
            .unwrap_or_else(|| f.request.url.clone());
        println!(
            "{}\t{}\t{}\t{}",
            path.display(),
            f.request.method,
            url,
            match f.outcome {
                Outcome::Complete => "complete",
                Outcome::Error => "error",
            }
        );
    }
}

fn emit(
    fixture: &Fixture,
    dir: &Path,
    common: &CommonOpts,
    request_id: u64,
    operation_id: u64,
) -> Result<(), String> {
    let cmd = build_command(fixture, dir, request_id, operation_id).map_err(|e| e.to_string())?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_json(&mut out, &cmd, common.pretty)?;
    writeln!(out).map_err(|e| e.to_string())?;
    if common.trace {
        if let Some(trace) = redirect_trace(fixture) {
            eprintln!(
                "trace: {}",
                serde_json::to_string(&trace).map_err(|e| e.to_string())?
            );
        }
    }
    Ok(())
}

fn update_jar(
    fixture: &Fixture,
    incoming: &IncomingRequest,
    jar_path: &Path,
) -> Result<(), String> {
    let mut jar: CookieJar = if let Ok(raw) = std::fs::read_to_string(jar_path) {
        serde_json::from_str(&raw).unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    // Overlay the fixture's own snapshot first (lower priority than the live jar).
    for (origin, cookies) in &fixture.cookie_jar {
        jar.entry(origin.clone()).or_default().extend(cookies.iter().cloned());
    }
    if let Some(resp) = &fixture.response {
        let set_cookies = extract_set_cookies(&resp.headers);
        if !set_cookies.is_empty() {
            let url = incoming
                .params
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or(&fixture.request.url);
            let origin = url_origin(url);
            merge_set_cookies(&mut jar, &origin, &set_cookies);
        }
    }
    let raw = serde_json::to_string_pretty(&jar).map_err(|e| e.to_string())?;
    std::fs::write(jar_path, raw).map_err(|e| format!("write jar {}: {e}", jar_path.display()))?;
    Ok(())
}

fn write_json(out: &mut impl Write, value: &Value, pretty: bool) -> Result<(), String> {
    if pretty {
        serde_json::to_writer_pretty(out, value).map_err(|e| e.to_string())
    } else {
        serde_json::to_writer(out, value).map_err(|e| e.to_string())
    }
}

fn usage_error() -> String {
    usage_text()
}

fn usage_text() -> String {
    "usage: host-replay <command> [options]\n\
     commands: show <fixture> | replay | list | validate <fixture>\n\
     common flags: --dir <dir> --fixture <file> --trace --update-jar <file> --pretty --request-id <n>"
        .to_string()
}

fn print_help() {
    println!("{}", usage_text());
    println!();
    println!("Fixture format: reader-host-replay/1  (see samples/host-replay/FORMAT.md)");
    println!("This tool is dev-time only; it never opens a socket and never modifies the protocol schema.");
}

//! Debug tool: load a Legado source JSON + recording, run content_book_source on
//! the L5-content recorded response, and print what the parser produces.
//!
//! Usage:
//!   cargo run -p reader-content --example debug_content -- <source.json> <recording.json>

use std::env;
use std::fs;
use std::path::PathBuf;

use reader_content::{BookSourceRequestContext, RemoteContentPipeline};
use reader_domain::{BookSourceSemantics, LegadoBookSource};
use serde_json::Value;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: debug_content <source.json> <recording.json>");
        std::process::exit(2);
    }
    let source_path = PathBuf::from(&args[1]);
    let recording_path = PathBuf::from(&args[2]);

    let source_raw = fs::read_to_string(&source_path).unwrap();
    let source_json: Value = serde_json::from_str(&source_raw).unwrap();

    let legado: LegadoBookSource =
        serde_json::from_value(source_json.clone()).unwrap_or_else(|err| {
            eprintln!("WARN: LegadoBookSource deserialize failed: {err}");
            LegadoBookSource::default()
        });

    let source_id = source_json
        .get("sourceId")
        .and_then(Value::as_str)
        .unwrap_or("debug")
        .to_string();
    let name = legado
        .book_source_name
        .clone()
        .unwrap_or_else(|| source_id.clone());
    let base_url = legado
        .book_source_url
        .clone()
        .unwrap_or_default();

    let semantics = BookSourceSemantics::from_legado(
        &source_id,
        Some(&name),
        Some(&base_url),
        &legado,
    );

    println!("=== Source: {} ({}) ===", name, source_id);
    println!("content rule       = {:?}", semantics.rules.content.content);
    println!("content title rule = {:?}", semantics.rules.content.title);
    println!("content nextUrl    = {:?}", semantics.rules.content.next_url);
    println!("content replaceRegex= {:?}", semantics.rules.content.replace_regex);
    println!("content sourceRegex= {:?}", semantics.rules.content.source_regex);
    println!("content raw        = {:?}", semantics.rules.content.raw);
    println!();

    let rec_raw = fs::read_to_string(&recording_path).unwrap();
    let rec: Value = serde_json::from_str(&rec_raw).unwrap();

    let l5 = rec
        .get("steps")
        .and_then(Value::as_array)
        .and_then(|steps| {
            steps
                .iter()
                .find(|s| s.get("level").and_then(Value::as_str) == Some("L5-content"))
        });

    let Some(l5) = l5 else {
        eprintln!("NO L5-content step in recording");
        std::process::exit(3);
    };

    let body = l5
        .get("response_body")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status = l5.get("response_status").and_then(Value::as_u64).unwrap_or(0);
    let url = l5
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("");

    println!("L5 url    = {url}");
    println!("L5 status = {status}");
    println!("L5 body len = {}", body.len());
    println!("L5 body (first 300 chars) = {}", body.chars().take(300).collect::<String>());
    println!();

    let pipeline = RemoteContentPipeline::new();
    let mut context = BookSourceRequestContext::for_semantics(&semantics);
    context.current_url = url.to_string();
    context.base_url = semantics.base_url.clone();

    match pipeline.content_book_source(&semantics, body, &context) {
        Ok(content) => {
            println!("=== content_book_source result ===");
            println!("title  = {:?}", content.title);
            println!("content len (chars) = {}", content.content.chars().count());
            println!("content len (bytes) = {}", content.content.len());
            println!("content (first 500 chars) = {}", content.content.chars().take(500).collect::<String>());
            println!("content (last 200 chars)  = {}", content.content.chars().rev().take(200).collect::<String>().chars().rev().collect::<String>());
            if let Some(next) = &content.next_content_url {
                println!("next_content_url = {next}");
            }
        }
        Err(err) => {
            println!("content parse ERROR: {err:?}");
        }
    }
}

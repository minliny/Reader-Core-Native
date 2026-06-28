//! Debug tool: load a Legado source JSON + recording, run toc_book_source on the
//! L4-toc recorded response, and print what the parser produces.
//!
//! Usage:
//!   cargo run -p reader-content --example debug_toc -- <source.json> <recording.json>

use std::env;
use std::fs;
use std::path::PathBuf;

use reader_content::{BookSourceRequestContext, RemoteContentPipeline};
use reader_domain::{BookSourceSemantics, LegadoBookSource};
use serde_json::Value;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: debug_toc <source.json> <recording.json>");
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

    let semantics = BookSourceSemantics::from_legado(&source_id, Some(&name), Some(&base_url), &legado);

    println!("=== Source: {} ({}) ===", name, source_id);
    println!("chapterList = {:?}", semantics.rules.toc.list);
    println!("chapterName = {:?}", semantics.rules.toc.name);
    println!("chapterUrl  = {:?}", semantics.rules.toc.url);
    println!("nextTocUrl  = {:?}", semantics.rules.toc.next_url);
    println!();

    let rec_raw = fs::read_to_string(&recording_path).unwrap();
    let rec: Value = serde_json::from_str(&rec_raw).unwrap();

    let l4 = rec
        .get("steps")
        .and_then(Value::as_array)
        .and_then(|steps| {
            steps
                .iter()
                .find(|s| s.get("level").and_then(Value::as_str) == Some("L4-toc"))
        });

    let Some(l4) = l4 else {
        eprintln!("NO L4-toc step in recording");
        std::process::exit(3);
    };

    let body = l4
        .get("response_body")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status = l4.get("response_status").and_then(Value::as_u64).unwrap_or(0);
    let url = l4
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("");

    println!("L4 url    = {url}");
    println!("L4 status = {status}");
    println!("L4 body len = {}", body.len());
    println!("L4 body (first 300 chars) = {}", body.chars().take(300).collect::<String>());
    println!();

    let pipeline = RemoteContentPipeline::new();
    let mut context = BookSourceRequestContext::for_semantics(&semantics);
    context.current_url = url.to_string();
    context.base_url = base_url.clone();

    match pipeline.toc_book_source(&semantics, body, &context) {
        Ok(toc) => {
            println!("TOC chapters count = {}", toc.chapters.len());
            for (i, ch) in toc.chapters.iter().take(5).enumerate() {
                println!("  [{i}] title={:?} url={:?}", ch.title, ch.url);
            }
            if let Some(next) = &toc.next_toc_url {
                println!("next_toc_url = {next}");
            }
        }
        Err(err) => {
            println!("TOC parse ERROR: {err:?}");
        }
    }
}

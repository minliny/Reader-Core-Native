//! CLI entry point for the text-normalize dev tool.
//!
//! Reads text from a file or stdin, applies a normalization preset, and writes
//! the normalized text and its FNV-1a hash.
//!
//! Usage:
//!   text-normalize [OPTIONS] [FILE]
//!
//! When FILE is omitted or "-", reads from stdin.
//!
//! Output:
//!   - default: normalized text to stdout, hash to stderr
//!   - --hash-only: only the hash to stdout
//!   - --json: {"hash":"...","text":"..."} to stdout

use std::io::{self, Read, Write};
use std::process::ExitCode;

use reader_text_normalize::{normalize, Mode, Normalized, UnicodeForm, WidthStrategy};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

fn run() -> Result<(), u8> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = Mode::Conservative;
    let mut width_override: Option<WidthStrategy> = None;
    let mut unicode_override: Option<UnicodeForm> = None;
    let mut collapse_whitespace_override: Option<bool> = None;
    let mut no_trim = false;
    let mut no_bom = false;
    let mut no_entities = false;
    let mut no_newline_norm = false;
    let mut hash_only = false;
    let mut json = false;
    let mut file: Option<String> = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "--conservative" => mode = Mode::Conservative,
            "--lenient" => mode = Mode::Lenient,
            "--width" => {
                let v = iter.next().ok_or_else(|| missing_value("--width"))?;
                width_override = Some(parse_width(&v)?);
            }
            "--unicode" => {
                let v = iter.next().ok_or_else(|| missing_value("--unicode"))?;
                unicode_override = Some(parse_unicode(&v)?);
            }
            "--collapse-whitespace" => collapse_whitespace_override = Some(true),
            "--no-collapse-whitespace" => collapse_whitespace_override = Some(false),
            "--no-trim" => no_trim = true,
            "--no-bom" => no_bom = true,
            "--no-entities" => no_entities = true,
            "--no-newline-norm" => no_newline_norm = true,
            "--hash-only" => hash_only = true,
            "--json" => json = true,
            "--" => {
                file = iter.next();
            }
            s if s.starts_with('-') && s != "-" => {
                eprintln!("text-normalize: unknown option '{s}'");
                eprintln!("see `text-normalize --help`");
                return Err(2);
            }
            s => file = Some(s.to_string()),
        }
    }

    let mut opts = mode.to_options();
    if let Some(w) = width_override {
        opts.width_strategy = w;
    }
    if let Some(u) = unicode_override {
        opts.unicode_form = u;
    }
    if let Some(c) = collapse_whitespace_override {
        opts.collapse_whitespace = c;
    }
    if no_trim {
        opts.trim = false;
    }
    if no_bom {
        opts.strip_bom = false;
    }
    if no_entities {
        opts.decode_html_entities = false;
    }
    if no_newline_norm {
        opts.normalize_newlines = false;
    }

    let input = match file.as_deref() {
        None | Some("-") => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| io_err(&e))?;
            buf
        }
        Some(path) => std::fs::read_to_string(path).map_err(|e| io_err(&e))?,
    };

    let Normalized { text, hash } = normalize(&input, &opts);

    let stdout = io::stdout();
    let mut out = stdout.lock();
    if hash_only {
        writeln!(out, "{hash}").map_err(|e| io_err(&e))?;
    } else if json {
        let escaped = json_escape(&text);
        writeln!(out, "{{\"hash\":\"{hash}\",\"text\":\"{escaped}\"}}").map_err(|e| io_err(&e))?;
    } else {
        write!(out, "{text}").map_err(|e| io_err(&e))?;
        if !text.ends_with('\n') {
            writeln!(out).map_err(|e| io_err(&e))?;
        }
        eprintln!("{hash}");
    }
    out.flush().map_err(|e| io_err(&e))?;
    Ok(())
}

fn parse_width(v: &str) -> Result<WidthStrategy, u8> {
    match v {
        "keep" | "Keep" => Ok(WidthStrategy::Keep),
        "tohalf" | "ToHalf" => Ok(WidthStrategy::ToHalf),
        _ => {
            eprintln!("text-normalize: --width expects 'keep' or 'tohalf', got '{v}'");
            Err(2)
        }
    }
}

fn parse_unicode(v: &str) -> Result<UnicodeForm, u8> {
    match v {
        "none" | "None" => Ok(UnicodeForm::None),
        "nfc" | "Nfc" | "NFC" => Ok(UnicodeForm::Nfc),
        "nfkc" | "Nfkc" | "NFKC" => Ok(UnicodeForm::Nfkc),
        _ => {
            eprintln!("text-normalize: --unicode expects 'none|nfc|nfkc', got '{v}'");
            Err(2)
        }
    }
}

fn missing_value(flag: &str) -> u8 {
    eprintln!("text-normalize: missing value for {flag}");
    2
}

fn io_err(e: &io::Error) -> u8 {
    eprintln!("text-normalize: {e}");
    1
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn print_help() {
    let help = "\
text-normalize — dev-time text normalization for benchmark chapter comparison

USAGE:
    text-normalize [OPTIONS] [FILE]

    FILE   Read input from FILE. Omit or use '-' to read from stdin.

MODES (preset option bundles):
    --conservative   NFC, CRLF→LF, blank-line collapse, HTML entity decode,
                     BOM strip, edge trim (newlines only). Preserves full-width
                     glyphs and inline whitespace. [default]
    --lenient        NFKC, full-width→half-width, all whitespace runs collapsed
                     to a single space, HTML entity decode, BOM strip, full trim.

OVERRIDES:
    --width <keep|tohalf>            Full-width/half-width strategy.
    --unicode <none|nfc|nfkc>        Unicode normalization form.
    --collapse-whitespace            Collapse all whitespace runs to single space.
    --no-collapse-whitespace         Preserve whitespace runs.
    --no-trim                        Do not trim edges.
    --no-bom                         Do not strip leading BOM.
    --no-entities                    Do not decode HTML entities.
    --no-newline-norm                Do not normalize CRLF/CR to LF.

OUTPUT:
    (default)    Normalized text to stdout, FNV-1a hash to stderr.
    --hash-only  Only the 16-char hex hash to stdout.
    --json       {\"hash\":\"...\",\"text\":\"...\"} to stdout.

EXAMPLES:
    text-normalize chapter.txt > normalized.txt
    text-normalize --lenient --hash-only chapter.txt
    cat raw.html | text-normalize --lenient --json
";
    print!("{help}");
}

//! Fixture-driven MOBI parser tests.
//!
//! Migrates the Reader-Core `MOBIParser.swift` clean-room semantics into Rust:
//! PDB signature detection + (full PDB/PalmDOC/MOBI/EXTH parse when records
//! exist, else clean-room readable text-fragment fallback). Fixtures are the
//! sanitized samples from Reader-Core's `format_differential` manifest.

use reader_local_book::{parse_mobi_book, LocalBookFormat, LocalBookInput};

const MOBI_CLEAN: &[u8] = include_bytes!("fixtures/mobi/mobi_clean_room_text_fragment.mobi");
const MOBI_BINARY: &[u8] = include_bytes!("fixtures/mobi/mobi_binary_metadata_only_fallback.mobi");

fn input<'a>(book_id: &'a str, file_name: &'a str, bytes: &'a [u8]) -> LocalBookInput<'a> {
    LocalBookInput {
        book_id,
        file_name: Some(file_name),
        title: None,
        author: None,
        bytes,
    }
}

#[test]
fn parses_mobi_clean_room_text_fragment() {
    let book = parse_mobi_book(input("mobi-001", "fixture.mobi", MOBI_CLEAN))
        .expect("clean-room MOBI fixture must parse");

    assert_eq!(book.format, LocalBookFormat::Mobi);
    // Title comes from the PDB name field (first 32 bytes, null-padded).
    assert_eq!(book.book.title, "Fixture MOBI");
    // Manifest: minimumChapterCount=2, expectedChapterTitles=["Chapter 1","Chapter 2"].
    assert!(
        book.chapters.len() >= 2,
        "expected >=2 chapters, got {}",
        book.chapters.len()
    );
    let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("Chapter 1")),
        "missing Chapter 1 in {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("Chapter 2")),
        "missing Chapter 2 in {titles:?}"
    );
    let body: String = book
        .chapters
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("MOBI clean-room chapter text"),
        "missing preview fragment: {body}"
    );
    assert!(
        body.contains("MOBI second chapter text"),
        "missing second preview fragment: {body}"
    );
}

#[test]
fn parses_mobi_binary_metadata_only_fallback() {
    let book = parse_mobi_book(input("mobi-002", "binary.mobi", MOBI_BINARY))
        .expect("binary-only MOBI must produce a metadata-only entry, not an error");

    assert_eq!(book.format, LocalBookFormat::Mobi);
    assert_eq!(book.book.title, "Fixture MOBI Binary");
    // No readable text fragments must be fabricated from the binary-only payload.
    let body: String = book
        .chapters
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !body.contains("MOBI clean-room"),
        "metadata-only entry must not fabricate readable text: {body}"
    );
}

#[test]
fn rejects_empty_mobi_input() {
    let err = parse_mobi_book(input("mobi-003", "empty.mobi", &[]))
        .expect_err("empty input must fail closed");
    assert!(err.to_string().to_lowercase().contains("empty"), "{err}");
}

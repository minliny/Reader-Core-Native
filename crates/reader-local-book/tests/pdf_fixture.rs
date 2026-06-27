//! Fixture-driven PDF parser tests.
//!
//! Migrates Reader-Core `SimplePDFTextExtractor.swift` clean-room semantics
//! into Rust: %PDF- signature detection + content-stream Tj/TJ text
//! extraction (BT/ET blocks, paren-delimited strings, escape sequences).
//! Fixtures are the sanitized samples from Reader-Core's `format_differential`
//! manifest.

use reader_local_book::{parse_pdf_book, LocalBookFormat, LocalBookInput};

const PDF_TEXT_PAGE: &[u8] = include_bytes!("fixtures/pdf/pdf_text_page_pdfkit.pdf");
const PDF_EMPTY_TEXT: &[u8] =
    include_bytes!("fixtures/pdf/pdf_empty_text_requires_ocr_fallback.pdf");
const PDF_INVALID: &[u8] = include_bytes!("fixtures/pdf/pdf_invalid_document_rejected.pdf");

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
fn parses_pdf_text_page_pdfkit() {
    let book = parse_pdf_book(input("pdf-001", "fixture.pdf", PDF_TEXT_PAGE))
        .expect("text-bearing PDF fixture must parse");

    assert_eq!(book.format, LocalBookFormat::Pdf);
    // Manifest: minimumChapterCount=1, expectedChapterTitles=["Page 1"].
    assert!(
        !book.chapters.is_empty(),
        "expected >=1 chapter, got {}",
        book.chapters.len()
    );
    let first = &book.chapters[0];
    assert!(
        first.title.contains("Page 1"),
        "expected first chapter title to contain 'Page 1', got {:?}",
        first.title
    );
    assert!(
        first.content.contains("Recovery 31 PDF Page One"),
        "missing expected preview fragment in chapter body: {:?}",
        first.content
    );
}

#[test]
fn pdf_empty_text_yields_ocr_unavailable_chapter() {
    let book = parse_pdf_book(input("pdf-002", "empty.pdf", PDF_EMPTY_TEXT))
        .expect("empty-text PDF must still produce a chapter, not an error");

    assert_eq!(book.format, LocalBookFormat::Pdf);
    assert!(
        !book.chapters.is_empty(),
        "expected >=1 chapter even when text is unavailable, got {}",
        book.chapters.len()
    );
    // The chapter must NOT fabricate readable page text. The body should
    // signal the OCR-unavailable diagnostic per the manifest.
    let body = &book.chapters[0].content;
    assert!(
        body.contains("text_unavailable_without_ocr") || body.trim().is_empty(),
        "empty-text PDF chapter must not fabricate readable text, got: {body:?}"
    );
    assert!(
        !body.contains("Recovery 31"),
        "empty-text PDF must not leak text from other fixtures: {body:?}"
    );
}

#[test]
fn rejects_invalid_pdf_document() {
    let err = parse_pdf_book(input("pdf-003", "bad.pdf", PDF_INVALID))
        .expect_err("truncated/invalid PDF must fail closed");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("invalid") || msg.contains("pdf"),
        "expected error to mention invalid/pdf, got: {msg}"
    );
}

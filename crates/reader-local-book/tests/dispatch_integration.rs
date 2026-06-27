//! Dispatch integration tests for `parse_local_book`.
//!
//! Verifies that the top-level dispatcher routes each fixture to the
//! correct format-specific parser (TXT/EPUB/PDF/MOBI) by detecting the
//! format from bytes + file extension.

use reader_local_book::{parse_local_book, LocalBookFormat, LocalBookInput};

const TXT_FIXTURE: &[u8] = include_bytes!("fixtures/txt/cjk_chapters.txt");
const EPUB_FIXTURE: &[u8] = include_bytes!("fixtures/epub/epub2_ncx_flat_toc.epub");
const PDF_FIXTURE: &[u8] = include_bytes!("fixtures/pdf/pdf_text_page_pdfkit.pdf");
const MOBI_FIXTURE: &[u8] = include_bytes!("fixtures/mobi/mobi_clean_room_text_fragment.mobi");

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
fn dispatch_routes_txt_to_txt_parser() {
    let book = parse_local_book(input("disp-txt", "cjk.txt", TXT_FIXTURE))
        .expect("TXT dispatch must succeed");
    assert_eq!(book.format, LocalBookFormat::Txt);
    assert!(!book.chapters.is_empty());
}

#[test]
fn dispatch_routes_epub_to_epub_parser() {
    let book = parse_local_book(input("disp-epub", "fixture.epub", EPUB_FIXTURE))
        .expect("EPUB dispatch must succeed");
    assert_eq!(book.format, LocalBookFormat::Epub);
    assert!(book.chapters.len() >= 2);
    assert_eq!(book.book.title, "Fixture EPUB");
}

#[test]
fn dispatch_routes_pdf_to_pdf_parser() {
    let book = parse_local_book(input("disp-pdf", "fixture.pdf", PDF_FIXTURE))
        .expect("PDF dispatch must succeed");
    assert_eq!(book.format, LocalBookFormat::Pdf);
    assert!(book.chapters.len() >= 1);
    assert!(book.chapters[0]
        .content
        .contains("Recovery 31 PDF Page One"));
}

#[test]
fn dispatch_routes_mobi_to_mobi_parser() {
    let book = parse_local_book(input("disp-mobi", "fixture.mobi", MOBI_FIXTURE))
        .expect("MOBI dispatch must succeed");
    assert_eq!(book.format, LocalBookFormat::Mobi);
    assert!(book.chapters.len() >= 2);
}

#[test]
fn dispatch_rejects_empty_input() {
    let err = parse_local_book(input("disp-empty", "empty.txt", &[]))
        .expect_err("empty input must fail closed");
    assert!(err.to_string().to_lowercase().contains("empty"), "{err}");
}

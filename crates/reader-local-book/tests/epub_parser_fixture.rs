//! Fixture-driven EPUB parser tests.
//!
//! Builds the EPUB parser against Legado `EpubFile.kt` semantics:
//! ZIP container + META-INF/container.xml + OPF (metadata/manifest/spine)
//! + EPUB3 nav / EPUB2 NCX / spine fallback + XHTML body extraction with
//!   script/style stripping. Fixtures are the sanitized samples from
//!   Reader-Core's `format_differential` manifest.

use reader_local_book::{parse_epub_book, LocalBookFormat, LocalBookInput};

const EPUB3_NAV: &[u8] = include_bytes!("fixtures/epub/epub3_nav_spine_resource_cover.epub");
const EPUB2_NCX: &[u8] = include_bytes!("fixtures/epub/epub2_ncx_flat_toc.epub");
const EPUB_SPINE_ONLY: &[u8] = include_bytes!("fixtures/epub/epub_spine_only_fallback.epub");

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
fn parses_epub3_nav_with_spine_resources() {
    let book = parse_epub_book(input("epub-001", "fixture.epub", EPUB3_NAV))
        .expect("EPUB3 nav fixture must parse");

    assert_eq!(book.format, LocalBookFormat::Epub);
    assert_eq!(book.book.title, "Fixture EPUB");
    // Manifest: minimumChapterCount=2, expectedChapterTitleFragments=["Chapter One","Nested Two"].
    assert!(
        book.chapters.len() >= 2,
        "expected >=2 chapters, got {}",
        book.chapters.len()
    );
    let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("Chapter One")),
        "missing 'Chapter One' in {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("Nested Two")),
        "missing 'Nested Two' in {titles:?}"
    );
    let body: String = book
        .chapters
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("EPUB text one."),
        "missing preview fragment: {body}"
    );
    assert!(
        body.contains("EPUB text two."),
        "missing preview fragment: {body}"
    );
    // Manifest: forbiddenPreviewFragments=["bad()"] — script must be stripped.
    assert!(
        !body.contains("bad()"),
        "script content leaked into chapter body: {body}"
    );
}

#[test]
fn parses_epub2_ncx_flat_toc() {
    let book = parse_epub_book(input("epub-002", "fixture.epub", EPUB2_NCX))
        .expect("EPUB2 NCX fixture must parse");

    assert_eq!(book.format, LocalBookFormat::Epub);
    assert_eq!(book.book.title, "Fixture EPUB");
    // Manifest: expectedChapterTitles=["NCX One","NCX Two"].
    assert!(
        book.chapters.len() >= 2,
        "expected >=2 chapters, got {}",
        book.chapters.len()
    );
    let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("NCX One")),
        "missing 'NCX One' in {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("NCX Two")),
        "missing 'NCX Two' in {titles:?}"
    );
    let body: String = book
        .chapters
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("EPUB text one."),
        "missing preview fragment: {body}"
    );
    assert!(
        body.contains("EPUB text two."),
        "missing preview fragment: {body}"
    );
    assert!(
        !body.contains("bad()"),
        "script content leaked into chapter body: {body}"
    );
}

#[test]
fn parses_epub_spine_only_fallback() {
    let book = parse_epub_book(input("epub-003", "fixture.epub", EPUB_SPINE_ONLY))
        .expect("spine-only EPUB fixture must parse via spine fallback");

    assert_eq!(book.format, LocalBookFormat::Epub);
    assert_eq!(book.book.title, "Fixture EPUB");
    // Manifest: expectedChapterTitles=["Chapter One","Chapter Two"] (from spine xhtml titles).
    assert!(
        book.chapters.len() >= 2,
        "expected >=2 chapters, got {}",
        book.chapters.len()
    );
    let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("Chapter One")),
        "missing 'Chapter One' in {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("Chapter Two")),
        "missing 'Chapter Two' in {titles:?}"
    );
    let body: String = book
        .chapters
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("EPUB text one."),
        "missing preview fragment: {body}"
    );
    assert!(
        body.contains("EPUB text two."),
        "missing preview fragment: {body}"
    );
    assert!(
        !body.contains("bad()"),
        "script content leaked into chapter body: {body}"
    );
}

#[test]
fn rejects_empty_epub_input() {
    let err = parse_epub_book(input("epub-004", "empty.epub", &[]))
        .expect_err("empty input must fail closed");
    assert!(err.to_string().to_lowercase().contains("empty"), "{err}");
}

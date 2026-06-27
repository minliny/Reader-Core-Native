//! Direct verification of the yodu.org MultiRule fixture against the
//! reader-content pipeline, bypassing the (broken WIP) reader-cli binary.
//!
//! This mirrors what `fixture_vertical_runs_legado_yodu_multirule_real_source
//! _pipeline` asserts at the CLI level, but drives the pipeline directly so
//! the rb-legado-css-multirule-operator fix can be validated even when
//! reader-cli is in a broken intermediate state from another agent.

use reader_content::RemoteContentPipeline;
use reader_domain::{Book, Source};
use serde_json::Value;

fn load_fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/remote_source/legado_yodu_multirule_vertical.json"
    );
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture JSON")
}

#[test]
fn yodu_search_returns_non_empty_books_with_multirule_booklist() {
    let fixture = load_fixture();
    let source_json = fixture["source"].clone();
    let source: Source = serde_json::from_value(source_json).expect("source");
    let search_html = fixture["searchResponse"].as_str().expect("searchResponse");

    let pipeline = RemoteContentPipeline::new();
    let semantics = source
        .book_source_semantics()
        .expect("book_source_semantics");
    let context = reader_content::BookSourceRequestContext::for_semantics(&semantics);
    let books = pipeline
        .search_book_source(&semantics, search_html, &context)
        .expect("search_book_source");

    assert!(
        !books.is_empty(),
        "search should return non-empty books after rb-legado-css-multirule-operator fix"
    );
    let first = &books[0];
    assert!(
        !first.title.is_empty(),
        "first book title should be non-empty (|| / && on CSS paths), got {:?}",
        first.title
    );
    assert!(
        !first.author.is_empty(),
        "first book author should be non-empty (&& AND-merge), got {:?}",
        first.author
    );
    assert!(
        !first.book_id.is_empty(),
        "first book bookId should be non-empty"
    );
    // || OR-fallback: bookList `class.ser-ret@li||class.j_bookList@li` — the
    // second branch (`j_bookList`) does not exist in this fixture's HTML, so
    // the first branch must win and return 15 books (ser-ret has 15 <li>).
    assert_eq!(
        books.len(),
        15,
        "|| OR-fallback should pick the ser-ret branch (15 books), got {}",
        books.len()
    );
}

#[test]
fn yodu_detail_returns_non_empty_title_with_multirule_name() {
    let fixture = load_fixture();
    let source_json = fixture["source"].clone();
    let source: Source = serde_json::from_value(source_json).expect("source");
    let detail_html = fixture["detailResponse"].as_str().expect("detailResponse");

    let pipeline = RemoteContentPipeline::new();
    let semantics = source
        .book_source_semantics()
        .expect("book_source_semantics");
    let context = reader_content::BookSourceRequestContext::for_semantics(&semantics);
    let base = Book {
        book_id: "https://www.yodu.org/book/17551/".to_string(),
        ..serde_json::from_str("{}").expect("empty book")
    };
    let detail = pipeline
        .detail_book_source(&semantics, &base, detail_html, &context)
        .expect("detail_book_source");

    assert!(
        !detail.book.title.is_empty(),
        "detail title should be non-empty after || fix on `h3@text||h2@text`, got {:?}",
        detail.book.title
    );
    assert!(
        !detail.book.author.is_empty(),
        "detail author should be non-empty, got {:?}",
        detail.book.author
    );
}

#[test]
fn yodu_toc_returns_non_empty_chapters_with_bare_text_href_extraction() {
    let fixture = load_fixture();
    let source_json = fixture["source"].clone();
    let source: Source = serde_json::from_value(source_json).expect("source");
    let toc_html = fixture["tocResponse"].as_str().expect("tocResponse");

    let pipeline = RemoteContentPipeline::new();
    let semantics = source
        .book_source_semantics()
        .expect("book_source_semantics");
    let context = reader_content::BookSourceRequestContext::for_semantics(&semantics);
    let toc = pipeline
        .toc_book_source(&semantics, toc_html, &context)
        .expect("toc_book_source");

    assert!(
        !toc.chapters.is_empty(),
        "toc should return non-empty chapters after bare-extraction fix (text/href), got {}",
        toc.chapters.len()
    );
    let first = &toc.chapters[0];
    assert!(
        !first.title.is_empty(),
        "first chapter title should be non-empty, got {:?}",
        first.title
    );
    assert!(
        !first.url.is_empty(),
        "first chapter url should be non-empty, got {:?}",
        first.url
    );
}

#[test]
fn yodu_chapter_content_returns_non_empty_body() {
    let fixture = load_fixture();
    let source_json = fixture["source"].clone();
    let source: Source = serde_json::from_value(source_json).expect("source");
    let chapter_html = fixture["chapterResponse"]
        .as_str()
        .expect("chapterResponse");

    let pipeline = RemoteContentPipeline::new();
    let semantics = source
        .book_source_semantics()
        .expect("book_source_semantics");
    let context = reader_content::BookSourceRequestContext::for_semantics(&semantics);
    let content = pipeline
        .content_book_source(&semantics, chapter_html, &context)
        .expect("content_book_source");

    assert!(
        !content.content.is_empty(),
        "chapter content should be non-empty, got len={}",
        content.content.len()
    );
    assert!(
        content.content.contains("萧炎"),
        "chapter content should contain real body text (萧炎), got: {}",
        &content.content[..content.content.len().min(200)]
    );
}

//! Edge-case integration tests for the TXT local-book parser.
//!
//! Covers the boundary conditions called out in the local-content-runtime
//! scope: empty files, no-heading files, duplicate headings, abnormal line
//! breaks (CRLF / lone CR / mixed), UTF-8 BOM, and metadata extraction.

use reader_local_book::{parse_txt, parse_txt_with_options, TxtParseOptions};

#[test]
fn empty_file_produces_no_chapters_and_no_title() {
    let parsed = parse_txt("");
    assert_eq!(parsed.metadata().title, "");
    assert_eq!(parsed.chapter_count(), 0);
    assert!(parsed.toc().is_empty());
    assert_eq!(parsed.chapter_body(0), None);
}

#[test]
fn whitespace_only_file_produces_no_chapters() {
    let parsed = parse_txt("   \n\n  \n");
    assert_eq!(parsed.metadata().title, "");
    assert_eq!(parsed.chapter_count(), 0);
}

#[test]
fn no_headings_folds_content_into_single_chapter() {
    let text = "这是一本没有章节标题的书。\n它只有正文内容。\n第二段正文。";
    let parsed = parse_txt(text);
    // First non-empty line becomes the title.
    assert_eq!(parsed.metadata().title, "这是一本没有章节标题的书。");
    assert_eq!(parsed.chapter_count(), 1);
    assert_eq!(parsed.chapters()[0].title, "正文");
    assert_eq!(parsed.chapters()[0].body, "它只有正文内容。\n第二段正文。");
}

#[test]
fn no_headings_with_extract_title_disabled_keeps_title_in_body() {
    let text = "书名行\n正文内容";
    let parsed = parse_txt_with_options(
        text,
        &TxtParseOptions {
            extract_title: false,
            min_chapter_chars: 0,
        },
    );
    assert_eq!(parsed.metadata().title, "");
    assert_eq!(parsed.chapter_count(), 1);
    assert_eq!(parsed.chapters()[0].body, "书名行\n正文内容");
}

#[test]
fn duplicate_headings_are_kept_and_distinguished_by_index() {
    let text = "第一章\nA\n第一章\nB\n第一章\nC";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapter_count(), 3);
    let toc = parsed.toc();
    assert_eq!(
        toc.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
        vec!["第一章", "第一章", "第一章"]
    );
    assert_eq!(
        toc.iter().map(|t| t.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(parsed.chapter_body(0), Some("A"));
    assert_eq!(parsed.chapter_body(1), Some("B"));
    assert_eq!(parsed.chapter_body(2), Some("C"));
}

#[test]
fn toc_entries_have_empty_url_for_local_books() {
    let text = "第一章 A\nbody\n第二章 B\nbody";
    let parsed = parse_txt(text);
    for entry in parsed.toc() {
        assert_eq!(entry.url, "");
    }
}

#[test]
fn crlf_line_endings_are_normalized_to_lf() {
    let text = "书名\r\n第一章 开始\r\n第一行\r\n第二行\r\n";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapters()[0].body, "第一行\n第二行");
}

#[test]
fn lone_cr_line_endings_are_normalized_to_lf() {
    // Old Mac-style line endings.
    let text = "书名\r第一章 开始\r第一行\r第二行\r";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapters()[0].body, "第一行\n第二行");
}

#[test]
fn mixed_line_endings_are_normalized() {
    let text = "书名\n第一章 开始\r\n第一行\r第二行\n第三行\r\n";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapters()[0].body, "第一行\n第二行\n第三行");
}

#[test]
fn utf8_bom_is_stripped_before_parsing() {
    let text = "\u{FEFF}书名\n第一章 开始\n正文";
    let parsed = parse_txt(text);
    assert_eq!(parsed.metadata().title, "书名");
    assert_eq!(parsed.chapter_count(), 1);
    assert_eq!(parsed.chapters()[0].body, "正文");
}

#[test]
fn leading_and_trailing_blank_lines_around_body_are_trimmed() {
    let text = "书名\n\n\n第一章 开始\n\n\n第一行\n\n\n";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapters()[0].body, "第一行");
}

#[test]
fn content_before_first_heading_becomes_prologue_chapter() {
    let text = "书名\n前言内容\n第一章 开始\n正文";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapter_count(), 2);
    assert_eq!(parsed.chapters()[0].title, "序");
    assert_eq!(parsed.chapters()[0].body, "前言内容");
    assert_eq!(parsed.chapters()[1].title, "第一章 开始");
    assert_eq!(parsed.chapters()[1].body, "正文");
}

#[test]
fn metadata_includes_kind_and_last_chapter() {
    let text = "书名\n第一章 A\nbody-a\n第三章 C\nbody-c";
    let parsed = parse_txt(text);
    assert_eq!(parsed.metadata().kind.as_deref(), Some("TXT"));
    assert_eq!(parsed.metadata().last_chapter.as_deref(), Some("第三章 C"));
    assert_eq!(parsed.metadata().author, "");
}

#[test]
fn english_chapter_headings_are_detected() {
    let text = "My Book\nChapter 1\nFirst body\nChapter 2\nSecond body";
    let parsed = parse_txt(text);
    assert_eq!(parsed.metadata().title, "My Book");
    assert_eq!(parsed.chapter_count(), 2);
    assert_eq!(parsed.chapters()[0].title, "Chapter 1");
    assert_eq!(parsed.chapters()[1].title, "Chapter 2");
}

#[test]
fn special_headings_are_detected() {
    let text = "书名\n楔子\n引子内容\n第一章 开始\n正文";
    let parsed = parse_txt(text);
    assert_eq!(parsed.chapter_count(), 2);
    assert_eq!(parsed.chapters()[0].title, "楔子");
    assert_eq!(parsed.chapters()[1].title, "第一章 开始");
}

#[test]
fn min_chapter_chars_merges_short_middle_segment() {
    let text = "书名\n第一章\n正常正文内容足够长\n第二章\n短\n第三章\n又是正常正文内容足够长";
    let parsed = parse_txt_with_options(
        text,
        &TxtParseOptions {
            extract_title: true,
            min_chapter_chars: 5,
        },
    );
    // The short "第二章" segment (1 char < 5) is merged into "第一章".
    assert_eq!(parsed.chapter_count(), 2);
    assert_eq!(parsed.chapters()[0].title, "第一章");
    assert_eq!(parsed.chapters()[1].title, "第三章");
    assert!(parsed.chapters()[0].body.contains("第二章"));
}

#[test]
fn full_chapter_pipeline_round_trips_through_domain_model() {
    let text = "三体\n第一章 科学边界\n汪淼来到会议室。\n第二章 疯狂年代\n叶文洁看到红岸。";
    let parsed = parse_txt(text);

    // Metadata → Book
    let book = parsed.metadata();
    assert_eq!(book.title, "三体");
    assert_eq!(book.kind.as_deref(), Some("TXT"));

    // TOC → Vec<TocEntry>
    let toc = parsed.toc();
    assert_eq!(toc.len(), 2);
    assert_eq!(toc[0].title, "第一章 科学边界");
    assert_eq!(toc[1].title, "第二章 疯狂年代");

    // Chapter body → String
    assert_eq!(parsed.chapter_body(0), Some("汪淼来到会议室。"));
    assert_eq!(parsed.chapter_body(1), Some("叶文洁看到红岸。"));
}

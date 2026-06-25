//! Fixture-driven TXT parsing stability tests.
//!
//! Loads a real `.txt` fixture (UTF-8 BOM + CRLF line endings, mixed CJK and
//! English chapter headings) and pins the parsed DTO. Any drift in BOM/CRLF
//! normalization or chapter-heading detection changes the parsed shape and
//! fails here, before it can leak into a stored local-book snapshot.

use reader_local_book::{parse_txt, ParsedTxt};

const FIXTURE: &str = include_str!("fixtures/txt/cjk_chapters.txt");

fn parsed() -> ParsedTxt {
    parse_txt(FIXTURE)
}

#[test]
fn fixture_strips_bom_and_extracts_title() {
    let parsed = parsed();
    let metadata = parsed.metadata();
    // The BOM must not survive into the title.
    assert!(!metadata.title.contains('\u{FEFF}'));
    assert_eq!(metadata.title, "示例书名");
    assert_eq!(metadata.author, "");
    assert_eq!(metadata.kind.as_deref(), Some("TXT"));
    assert_eq!(metadata.last_chapter.as_deref(), Some("Chapter 3"));
}

#[test]
fn fixture_detects_prologue_plus_three_chapters() {
    let parsed = parsed();
    let chapters = parsed.chapters();
    assert_eq!(
        chapters
            .iter()
            .map(|c| c.title.as_str())
            .collect::<Vec<_>>(),
        vec!["序", "第1章 启程", "第2章 相遇", "Chapter 3"]
    );
}

#[test]
fn fixture_normalizes_crlf_and_trims_bodies() {
    let parsed = parsed();
    let chapters = parsed.chapters();
    assert_eq!(chapters[0].body, "这一段是序言内容，出现在第一章之前。");
    assert_eq!(chapters[1].body, "正文一：主角离开村庄。");
    assert_eq!(chapters[2].body, "正文二：路上遇见同伴。");
    assert_eq!(chapters[3].body, "body three: the road goes on.");
    // CRLF must be normalized to LF; no carriage returns survive.
    for chapter in chapters {
        assert!(!chapter.body.contains('\r'));
    }
}

#[test]
fn fixture_toc_is_stable_and_local_urls_empty() {
    let parsed = parsed();
    let toc = parsed.toc();
    assert_eq!(
        toc.iter()
            .map(|entry| (entry.index, entry.title.as_str(), entry.url.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, "序", ""),
            (1, "第1章 启程", ""),
            (2, "第2章 相遇", ""),
            (3, "Chapter 3", ""),
        ]
    );
}

#[test]
fn fixture_chapter_body_accessor_round_trips() {
    let parsed = parsed();
    assert_eq!(parsed.chapter_count(), 4);
    assert_eq!(parsed.chapter_body(2).unwrap(), "正文二：路上遇见同伴。");
    assert!(parsed.chapter_body(4).is_none());
}

#[test]
fn fixture_parse_is_deterministic_across_calls() {
    let first = parsed();
    let second = parsed();
    assert_eq!(first.metadata(), second.metadata());
    assert_eq!(first.chapters(), second.chapters());
}

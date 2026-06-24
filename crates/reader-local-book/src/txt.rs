//! TXT local-book parser.
//!
//! Turns raw decoded TXT text into a [`ParsedTxt`] carrying book-level metadata
//! (a [`Book`](reader_domain::Book)) and an ordered list of [`TxtChapter`]
//! entries. The output is designed to feed the existing domain model directly:
//!
//! - [`ParsedTxt::metadata`] → `Book`
//! - [`ParsedTxt::toc`] → `Vec<TocEntry>`
//! - [`ParsedTxt::chapter_body`] → chapter body `String`
//!
//! Chapter title detection is pattern-based and std-only (no regex dependency).
//! It recognises the common Chinese and English chapter heading conventions:
//!
//! - `第N章` / `第N节` / `第N卷` / `第N回` / `第N部` / `第N篇` (Chinese or Arabic
//!   numerals, including full-width digits)
//! - `Chapter N` / `CHAPTER N` (Arabic or Roman numerals)
//! - Special headings: `楔子`, `序章`, `序言`, `前言`, `引子`, `引言`, `后记`,
//!   `尾声`, `终章`, `番外…`, `Prologue`, `Epilogue`
//!
//! Encoding detection (GBK/GB18030 → UTF-8) is out of scope for this std-only
//! implementation; callers must supply already-decoded `&str`. See the gap note
//! in [`crate`] docs.

use reader_domain::{Book, TocEntry};

/// Kind label stamped onto parsed books.
const TXT_KIND: &str = "TXT";

/// Default title used when a TXT file has content but no detectable chapter
/// headings and no leading title line.
const UNTITLED_CHAPTER: &str = "正文";

/// Title used for leading content that appears before the first detected
/// chapter heading.
const PROLOGUE_TITLE: &str = "序";

/// A single parsed TXT chapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtChapter {
    /// Chapter heading text (trimmed).
    pub title: String,
    /// Chapter body text with normalized `\n` line endings and trimmed edges.
    pub body: String,
}

/// The result of parsing a TXT document.
///
/// Carries book-level metadata plus the ordered chapter list. All accessors
/// return data structures consumable by the existing domain model.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTxt {
    metadata: Book,
    chapters: Vec<TxtChapter>,
}

impl ParsedTxt {
    /// Book-level metadata extracted from the TXT file.
    pub fn metadata(&self) -> &Book {
        &self.metadata
    }

    /// The ordered list of parsed chapters.
    pub fn chapters(&self) -> &[TxtChapter] {
        &self.chapters
    }

    /// Build a domain table-of-contents from the parsed chapters.
    ///
    /// `url` is left empty for local books; the `index` field is the stable
    /// key for [`Self::chapter_body`].
    pub fn toc(&self) -> Vec<TocEntry> {
        self.chapters
            .iter()
            .enumerate()
            .map(|(i, ch)| TocEntry {
                index: i as u32,
                title: ch.title.clone(),
                url: String::new(),
            })
            .collect()
    }

    /// Read the body text of the chapter at `index`, or `None` if out of range.
    pub fn chapter_body(&self, index: usize) -> Option<&str> {
        self.chapters.get(index).map(|c| c.body.as_str())
    }

    /// Number of parsed chapters.
    pub fn chapter_count(&self) -> usize {
        self.chapters.len()
    }
}

/// Tunable options for [`parse_txt_with_options`].
#[derive(Debug, Clone)]
pub struct TxtParseOptions {
    /// Whether to treat the first non-empty, non-chapter-heading line as the
    /// book title. Defaults to `true`.
    pub extract_title: bool,
    /// Minimum body length (in `char`s) for a detected segment to stand on its
    /// own as a chapter. Segments shorter than this are merged into the
    /// previous chapter to absorb false-positive heading detections. `0`
    /// disables merging. Defaults to `0`.
    pub min_chapter_chars: usize,
}

impl Default for TxtParseOptions {
    fn default() -> Self {
        Self {
            extract_title: true,
            min_chapter_chars: 0,
        }
    }
}

/// Parse a TXT document with default options.
pub fn parse_txt(text: &str) -> ParsedTxt {
    parse_txt_with_options(text, &TxtParseOptions::default())
}

/// Parse a TXT document with the given options.
pub fn parse_txt_with_options(text: &str, options: &TxtParseOptions) -> ParsedTxt {
    let normalized = normalize_input(text);
    let lines: Vec<&str> = normalized.lines().collect();

    // --- Extract book title (first non-empty, non-chapter line) -------------
    let mut title = String::new();
    let mut content_start = 0usize;
    if options.extract_title && !lines.is_empty() {
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if is_chapter_title(trimmed) {
                // First non-empty line is already a chapter heading — no title.
                content_start = i;
                break;
            }
            title = trimmed.to_string();
            content_start = i + 1;
            break;
        }
    }

    // --- Detect chapter heading boundaries ----------------------------------
    let mut boundaries: Vec<(String, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(content_start) {
        let trimmed = line.trim();
        if is_chapter_title(trimmed) {
            boundaries.push((trimmed.to_string(), i));
        }
    }

    // --- Build chapters -----------------------------------------------------
    let chapters = if boundaries.is_empty() {
        // No headings: fold all remaining content into a single chapter.
        let body = join_and_trim(&lines[content_start..]);
        if body.is_empty() {
            Vec::new()
        } else {
            vec![TxtChapter {
                title: UNTITLED_CHAPTER.to_string(),
                body,
            }]
        }
    } else {
        let mut result = Vec::new();

        // Content before the first heading becomes a prologue chapter.
        let first = boundaries[0].1;
        if first > content_start {
            let pre_body = join_and_trim(&lines[content_start..first]);
            if !pre_body.is_empty() {
                result.push(TxtChapter {
                    title: PROLOGUE_TITLE.to_string(),
                    body: pre_body,
                });
            }
        }

        // Slice between consecutive headings.
        for (idx, (heading, start)) in boundaries.iter().enumerate() {
            let end = if idx + 1 < boundaries.len() {
                boundaries[idx + 1].1
            } else {
                lines.len()
            };
            let body = if *start + 1 <= end {
                join_and_trim(&lines[*start + 1..end])
            } else {
                String::new()
            };
            result.push(TxtChapter {
                title: heading.clone(),
                body,
            });
        }

        if options.min_chapter_chars > 0 && result.len() > 1 {
            result = merge_short_chapters(result, options.min_chapter_chars);
        }

        result
    };

    let metadata = Book {
        book_id: String::new(),
        title,
        author: String::new(),
        cover_url: None,
        intro: None,
        kind: Some(TXT_KIND.to_string()),
        last_chapter: chapters.last().map(|c| c.title.clone()),
    };

    ParsedTxt { metadata, chapters }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Strip a leading UTF-8 BOM and normalize CRLF / lone CR to LF.
fn normalize_input(text: &str) -> String {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Join a slice of lines with `\n` and trim leading/trailing blank lines plus
/// trailing whitespace on each line.
fn join_and_trim(lines: &[&str]) -> String {
    let trimmed_lines: Vec<&str> = lines.iter().map(|l| l.trim_end()).collect();
    let start = trimmed_lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(trimmed_lines.len());
    let end = trimmed_lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        return String::new();
    }
    trimmed_lines[start..end].join("\n")
}

/// Merge chapters whose body is shorter than `min_chars` into the previous
/// chapter. The first chapter is always kept even if short.
fn merge_short_chapters(chapters: Vec<TxtChapter>, min_chars: usize) -> Vec<TxtChapter> {
    let mut result: Vec<TxtChapter> = Vec::new();
    for ch in chapters {
        if !result.is_empty() && ch.body.chars().count() < min_chars {
            let prev = result.last_mut().unwrap();
            if !prev.body.is_empty() {
                prev.body.push('\n');
            }
            prev.body.push_str(&ch.title);
            prev.body.push('\n');
            prev.body.push_str(&ch.body);
        } else {
            result.push(ch);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Chapter title detection
// ---------------------------------------------------------------------------

/// Returns `true` if `line` looks like a chapter heading.
fn is_chapter_title(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 50 {
        return false;
    }
    if matches_chapter_pattern(trimmed) {
        return true;
    }
    // Retry with surrounding brackets stripped: 【…】, 《…》, […], 「…」
    if let Some(inner) = strip_wrapping_brackets(trimmed) {
        return matches_chapter_pattern(inner);
    }
    false
}

fn matches_chapter_pattern(s: &str) -> bool {
    is_chinese_chapter(s) || is_english_chapter(s) || is_special_heading(s)
}

/// `第<numeral>章|节|卷|回|部|篇 [title]`
fn is_chinese_chapter(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('第') else {
        return false;
    };
    for (i, c) in rest.char_indices() {
        if is_chapter_marker(c) {
            let numeral = &rest[..i];
            return is_valid_numeral(numeral);
        }
        if !is_numeral_char(c) {
            return false;
        }
    }
    false
}

fn is_chapter_marker(c: char) -> bool {
    matches!(c, '章' | '节' | '卷' | '回' | '部' | '篇')
}

fn is_valid_numeral(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_numeral_char)
}

fn is_numeral_char(c: char) -> bool {
    matches!(
        c,
        '0'..='9'
            | '０'..='９'
            | '零' | '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九'
            | '十' | '百' | '千' | '万' | '亿' | '两' | '〇'
            | '壹' | '贰' | '叁' | '肆' | '伍' | '陆' | '柒' | '捌' | '玖'
            | '拾' | '佰' | '仟'
    )
}

/// `Chapter N` / `CHAPTER N` (Arabic or Roman), case-insensitive.
fn is_english_chapter(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("chapter") else {
        return false;
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        return false;
    }
    // Read only the number token: consecutive digits or roman-numeral letters.
    // This stops at delimiters like `:` / `-` / `.` that may follow the number.
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'))
        .collect();
    is_arabic_number(&token) || is_roman_numeral(&token)
}

fn is_arabic_number(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn is_roman_numeral(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            matches!(
                c,
                'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M' | 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
        })
}

/// Special standalone headings.
fn is_special_heading(s: &str) -> bool {
    const EXACT: &[&str] = &[
        "楔子", "序章", "序言", "前言", "引子", "引言", "后记", "尾声", "终章", "终卷", "结语",
    ];
    if EXACT.contains(&s) {
        return true;
    }
    if let Some(suffix) = s.strip_prefix("番外") {
        // Allow short suffixes like `番外一`, `番外1`, `番外·标题`.
        return suffix.chars().count() <= 20;
    }
    let lower = s.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "prologue" | "epilogue" | "prolog" | "epilog"
    )
}

/// Strip a single layer of matching wrapping brackets, returning the inner
/// string if `s` is wrapped.
fn strip_wrapping_brackets(s: &str) -> Option<&str> {
    let pairs: &[(char, char)] = &[
        ('【', '】'),
        ('《', '》'),
        ('[', ']'),
        ('「', '」'),
        ('“', '”'),
    ];
    for (open, close) in pairs {
        if s.starts_with(*open) && s.ends_with(*close) {
            let start = open.len_utf8();
            let end = s.len().saturating_sub(close.len_utf8());
            if start < end {
                return Some(&s[start..end]);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chinese_chapter_headings() {
        assert!(is_chapter_title("第一章"));
        assert!(is_chapter_title("第一章 引子"));
        assert!(is_chapter_title("第1章"));
        assert!(is_chapter_title("第1章 大漠英雄"));
        assert!(is_chapter_title("第二十三章"));
        assert!(is_chapter_title("第一百零一章"));
        assert!(is_chapter_title("第三节"));
        assert!(is_chapter_title("第一卷"));
        assert!(is_chapter_title("第一回"));
        assert!(is_chapter_title("第一部"));
        assert!(is_chapter_title("第一篇"));
        assert!(is_chapter_title("第１章")); // full-width digit
    }

    #[test]
    fn rejects_non_chapter_lines_starting_with_第() {
        assert!(!is_chapter_title("第一次世界大战爆发"));
        assert!(!is_chapter_title("第三方"));
        assert!(!is_chapter_title("第章"));
        assert!(!is_chapter_title("第三者"));
    }

    #[test]
    fn detects_english_chapter_headings() {
        assert!(is_chapter_title("Chapter 1"));
        assert!(is_chapter_title("Chapter 11"));
        assert!(is_chapter_title("CHAPTER I"));
        assert!(is_chapter_title("Chapter 1: The Beginning"));
        assert!(is_chapter_title("chapter ii"));
    }

    #[test]
    fn detects_special_headings() {
        assert!(is_chapter_title("楔子"));
        assert!(is_chapter_title("序章"));
        assert!(is_chapter_title("番外一"));
        assert!(is_chapter_title("番外1"));
        assert!(is_chapter_title("Prologue"));
        assert!(is_chapter_title("Epilogue"));
    }

    #[test]
    fn detects_bracketed_headings() {
        assert!(is_chapter_title("【第一章 引子】"));
        assert!(is_chapter_title("《序章》"));
        assert!(is_chapter_title("[Chapter 1]"));
    }

    #[test]
    fn rejects_long_lines_and_body_text() {
        assert!(!is_chapter_title(
            "这是一段很长的正文内容，它显然不是一个章节标题，因为它超过了长度限制并且读起来像是一段正常的段落文字。"
        ));
        assert!(!is_chapter_title(""));
        assert!(!is_chapter_title("   "));
    }

    #[test]
    fn parses_simple_two_chapter_text() {
        let text = "书名\n第一章 开始\n内容一\n第二章 结束\n内容二";
        let parsed = parse_txt(text);
        assert_eq!(parsed.metadata().title, "书名");
        assert_eq!(parsed.chapter_count(), 2);
        assert_eq!(parsed.chapters()[0].title, "第一章 开始");
        assert_eq!(parsed.chapters()[0].body, "内容一");
        assert_eq!(parsed.chapters()[1].title, "第二章 结束");
        assert_eq!(parsed.chapters()[1].body, "内容二");
    }

    #[test]
    fn toc_and_chapter_body_round_trip() {
        let text = "第一章 A\nbody-a\n第二章 B\nbody-b";
        let parsed = parse_txt(text);
        let toc = parsed.toc();
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].index, 0);
        assert_eq!(toc[0].title, "第一章 A");
        assert_eq!(toc[0].url, "");
        assert_eq!(parsed.chapter_body(0), Some("body-a"));
        assert_eq!(parsed.chapter_body(1), Some("body-b"));
        assert_eq!(parsed.chapter_body(99), None);
    }
}

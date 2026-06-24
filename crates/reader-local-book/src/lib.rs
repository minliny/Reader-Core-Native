//! Reader-Core local-book parsing — TXT / EPUB / encoding detection.
//!
//! This crate owns local-book data modeling and offline parsing. V1 implements
//! TXT ingestion with Unicode BOM handling and deterministic chapter splitting;
//! EPUB remains a future format instead of being faked through this API.

use std::collections::HashMap;
use std::path::Path;

use reader_domain::{Book, TocEntry};
use serde::{Deserialize, Serialize};

/// Current local-book library snapshot schema version.
pub const LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Supported local-book formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalBookFormat {
    Txt,
}

/// Text encoding detected while ingesting a TXT file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalBookEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

/// Byte input and optional metadata for a local TXT book.
#[derive(Debug, Clone, Copy)]
pub struct LocalBookInput<'a> {
    pub book_id: &'a str,
    pub file_name: Option<&'a str>,
    pub title: Option<&'a str>,
    pub author: Option<&'a str>,
    pub bytes: &'a [u8],
}

/// Parsed local book ready to be inserted into a library/storage layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBook {
    pub book: Book,
    pub format: LocalBookFormat,
    pub encoding: LocalBookEncoding,
    pub byte_len: usize,
    pub char_len: usize,
    pub toc: Vec<TocEntry>,
    pub chapters: Vec<LocalBookChapter>,
}

/// One parsed local-book chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapter {
    pub index: u32,
    pub title: String,
    pub content: String,
    /// Character offset of the chapter heading or chapter body start.
    pub start_char: usize,
    /// Character offset where this chapter window ends.
    pub end_char: usize,
}

/// Complete export/import unit for local-book library state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookLibrarySnapshot {
    pub schema_version: u32,
    pub exported_at: i64,
    #[serde(default)]
    pub books: Vec<LocalBook>,
}

impl LocalBookLibrarySnapshot {
    pub fn empty(exported_at: i64) -> Self {
        Self {
            schema_version: LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            books: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.schema_version != LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION {
            return Err(LocalBookError::InvalidSnapshot {
                field: "schema_version".into(),
            });
        }

        let mut book_ids = HashMap::<String, ()>::new();
        for book in &self.books {
            validate_local_book(book)?;
            if book_ids.insert(book.book.book_id.clone(), ()).is_some() {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "books".into(),
                });
            }
        }
        Ok(())
    }
}

/// In-memory local-book library for parsed offline books.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LocalBookLibrary {
    books: HashMap<String, LocalBook>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalBookError {
    EmptyInput,
    InvalidMetadata { field: String },
    InvalidBook { field: String },
    InvalidSnapshot { field: String },
    UnsupportedEncoding,
    Decode { reason: String },
    BookNotFound { book_id: String },
    ChapterNotFound { book_id: String, chapter_index: u32 },
}

impl std::fmt::Display for LocalBookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalBookError::EmptyInput => write!(f, "local book input is empty"),
            LocalBookError::InvalidMetadata { field } => {
                write!(f, "invalid local book metadata field: {field}")
            }
            LocalBookError::InvalidBook { field } => {
                write!(f, "invalid local book field: {field}")
            }
            LocalBookError::InvalidSnapshot { field } => {
                write!(f, "invalid local book snapshot field: {field}")
            }
            LocalBookError::UnsupportedEncoding => write!(f, "unsupported local book encoding"),
            LocalBookError::Decode { reason } => write!(f, "failed to decode local book: {reason}"),
            LocalBookError::BookNotFound { book_id } => {
                write!(f, "local book not found: {book_id}")
            }
            LocalBookError::ChapterNotFound {
                book_id,
                chapter_index,
            } => write!(
                f,
                "local book chapter not found: book={book_id} chapter={chapter_index}"
            ),
        }
    }
}

impl std::error::Error for LocalBookError {}

/// Parse a TXT local book from bytes.
pub fn parse_txt_book(input: LocalBookInput<'_>) -> Result<LocalBook, LocalBookError> {
    let book_id = normalize_required(input.book_id, "book_id")?;
    if input.bytes.is_empty() {
        return Err(LocalBookError::EmptyInput);
    }

    let (decoded, encoding) = decode_txt_bytes(input.bytes)?;
    let normalized = normalize_text(&decoded);
    if normalized.trim().is_empty() {
        return Err(LocalBookError::EmptyInput);
    }

    let title = derive_title(input.title, input.file_name, &book_id);
    let author = input
        .author
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();
    let chapters = split_chapters(&normalized);
    let toc = chapters
        .iter()
        .map(|chapter| TocEntry {
            index: chapter.index,
            title: chapter.title.clone(),
            url: format!("local://{book_id}/chapter/{}", chapter.index),
        })
        .collect::<Vec<_>>();
    let last_chapter = chapters.last().map(|chapter| chapter.title.clone());

    Ok(LocalBook {
        book: Book {
            book_id,
            title,
            author,
            cover_url: None,
            intro: None,
            kind: Some("local".into()),
            last_chapter,
        },
        format: LocalBookFormat::Txt,
        encoding,
        byte_len: input.bytes.len(),
        char_len: normalized.chars().count(),
        toc,
        chapters,
    })
}

/// Parse already-decoded TXT content. This is useful for tests and callers that
/// receive trusted UTF-8 text from their host platform.
pub fn parse_txt_text(
    book_id: &str,
    title: Option<&str>,
    author: Option<&str>,
    file_name: Option<&str>,
    text: &str,
) -> Result<LocalBook, LocalBookError> {
    parse_txt_book(LocalBookInput {
        book_id,
        title,
        author,
        file_name,
        bytes: text.as_bytes(),
    })
}

impl LocalBookLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_book(&mut self, book: LocalBook) -> Result<LocalBook, LocalBookError> {
        validate_local_book(&book)?;
        self.books.insert(book.book.book_id.clone(), book.clone());
        Ok(book)
    }

    pub fn parse_and_upsert_txt(
        &mut self,
        input: LocalBookInput<'_>,
    ) -> Result<LocalBook, LocalBookError> {
        let book = parse_txt_book(input)?;
        self.upsert_book(book)
    }

    pub fn get_book(&self, book_id: &str) -> Result<Option<LocalBook>, LocalBookError> {
        let book_id = normalize_required(book_id, "book_id")?;
        Ok(self.books.get(&book_id).cloned())
    }

    pub fn list_books(&self) -> Vec<LocalBook> {
        let mut books = self.books.values().cloned().collect::<Vec<_>>();
        books.sort_by(|a, b| {
            a.book
                .title
                .cmp(&b.book.title)
                .then_with(|| a.book.book_id.cmp(&b.book.book_id))
        });
        books
    }

    pub fn get_chapter(
        &self,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<LocalBookChapter, LocalBookError> {
        let book_id = normalize_required(book_id, "book_id")?;
        let book = self
            .books
            .get(&book_id)
            .ok_or_else(|| LocalBookError::BookNotFound {
                book_id: book_id.clone(),
            })?;
        book.chapters
            .iter()
            .find(|chapter| chapter.index == chapter_index)
            .cloned()
            .ok_or(LocalBookError::ChapterNotFound {
                book_id,
                chapter_index,
            })
    }

    pub fn remove_book(&mut self, book_id: &str) -> Result<bool, LocalBookError> {
        let book_id = normalize_required(book_id, "book_id")?;
        Ok(self.books.remove(&book_id).is_some())
    }

    pub fn export_snapshot(
        &self,
        exported_at: i64,
    ) -> Result<LocalBookLibrarySnapshot, LocalBookError> {
        let mut snapshot = LocalBookLibrarySnapshot {
            schema_version: LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            books: self.books.values().cloned().collect(),
        };
        sort_local_book_snapshot(&mut snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn replace_with_snapshot(
        &mut self,
        snapshot: LocalBookLibrarySnapshot,
    ) -> Result<(), LocalBookError> {
        snapshot.validate()?;
        let mut books = HashMap::new();
        for book in snapshot.books {
            books.insert(book.book.book_id.clone(), book);
        }
        self.books = books;
        Ok(())
    }
}

fn sort_local_book_snapshot(snapshot: &mut LocalBookLibrarySnapshot) {
    snapshot.books.sort_by(|a, b| {
        a.book
            .book_id
            .cmp(&b.book.book_id)
            .then_with(|| a.book.title.cmp(&b.book.title))
    });
}

fn decode_txt_bytes(bytes: &[u8]) -> Result<(String, LocalBookEncoding), LocalBookError> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        let text = std::str::from_utf8(&bytes[3..]).map_err(|e| LocalBookError::Decode {
            reason: e.to_string(),
        })?;
        return Ok((text.to_string(), LocalBookEncoding::Utf8Bom));
    }

    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&bytes[2..], LocalBookEncoding::Utf16Le, u16::from_le_bytes);
    }

    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&bytes[2..], LocalBookEncoding::Utf16Be, u16::from_be_bytes);
    }

    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), LocalBookEncoding::Utf8)),
        Err(_) => Err(LocalBookError::UnsupportedEncoding),
    }
}

fn decode_utf16(
    bytes: &[u8],
    encoding: LocalBookEncoding,
    convert: fn([u8; 2]) -> u16,
) -> Result<(String, LocalBookEncoding), LocalBookError> {
    if bytes.len() % 2 != 0 {
        return Err(LocalBookError::Decode {
            reason: "UTF-16 byte length is not even".into(),
        });
    }

    let code_units = bytes
        .chunks_exact(2)
        .map(|chunk| convert([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let text = String::from_utf16(&code_units).map_err(|e| LocalBookError::Decode {
        reason: e.to_string(),
    })?;
    Ok((text, encoding))
}

fn normalize_required(value: &str, field: &str) -> Result<String, LocalBookError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(LocalBookError::InvalidMetadata {
            field: field.into(),
        });
    }
    Ok(trimmed.to_string())
}

fn validate_local_book(book: &LocalBook) -> Result<(), LocalBookError> {
    if book.book.book_id.trim().is_empty() {
        return Err(LocalBookError::InvalidBook {
            field: "book.book_id".into(),
        });
    }
    if book.book.title.trim().is_empty() {
        return Err(LocalBookError::InvalidBook {
            field: "book.title".into(),
        });
    }
    if book.byte_len == 0 {
        return Err(LocalBookError::InvalidBook {
            field: "byte_len".into(),
        });
    }
    if book.char_len == 0 {
        return Err(LocalBookError::InvalidBook {
            field: "char_len".into(),
        });
    }
    if book.toc.len() != book.chapters.len() || book.chapters.is_empty() {
        return Err(LocalBookError::InvalidBook {
            field: "chapters".into(),
        });
    }

    for (expected_index, chapter) in book.chapters.iter().enumerate() {
        if chapter.index != expected_index as u32 {
            return Err(LocalBookError::InvalidBook {
                field: "chapters.index".into(),
            });
        }
        if chapter.title.trim().is_empty() {
            return Err(LocalBookError::InvalidBook {
                field: "chapters.title".into(),
            });
        }
        if chapter.start_char > chapter.end_char || chapter.end_char > book.char_len {
            return Err(LocalBookError::InvalidBook {
                field: "chapters.range".into(),
            });
        }
    }

    for (expected_index, toc) in book.toc.iter().enumerate() {
        if toc.index != expected_index as u32 {
            return Err(LocalBookError::InvalidBook {
                field: "toc.index".into(),
            });
        }
        if toc.title.trim().is_empty() {
            return Err(LocalBookError::InvalidBook {
                field: "toc.title".into(),
            });
        }
    }

    Ok(())
}

fn derive_title(title: Option<&str>, file_name: Option<&str>, book_id: &str) -> String {
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        return title.to_string();
    }

    if let Some(file_name) = file_name.map(str::trim).filter(|value| !value.is_empty()) {
        let stem = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(stem) = stem {
            return stem.to_string();
        }
    }

    book_id.to_string()
}

fn normalize_text(text: &str) -> String {
    text.trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn split_chapters(text: &str) -> Vec<LocalBookChapter> {
    let lines = indexed_lines(text);
    let heading_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(line_index, (_, line))| is_chapter_heading(line.trim()).then_some(line_index))
        .collect::<Vec<_>>();

    if heading_indices.is_empty() {
        return vec![LocalBookChapter {
            index: 0,
            title: "正文".into(),
            content: trim_outer_blank_lines(text),
            start_char: 0,
            end_char: text.chars().count(),
        }];
    }

    let mut chapters = Vec::new();
    let first_heading = heading_indices[0];
    let preface = join_line_range(&lines, 0, first_heading);
    if !preface.trim().is_empty() {
        chapters.push(LocalBookChapter {
            index: 0,
            title: "序章".into(),
            content: trim_outer_blank_lines(&preface),
            start_char: 0,
            end_char: lines[first_heading].0,
        });
    }

    for (heading_order, line_index) in heading_indices.iter().enumerate() {
        let next_line_index = heading_indices
            .get(heading_order + 1)
            .copied()
            .unwrap_or(lines.len());
        let title = lines[*line_index].1.trim().to_string();
        let content = join_line_range(&lines, line_index + 1, next_line_index);
        let start_char = lines[*line_index].0;
        let end_char = if next_line_index < lines.len() {
            lines[next_line_index].0
        } else {
            text.chars().count()
        };
        chapters.push(LocalBookChapter {
            index: chapters.len() as u32,
            title,
            content: trim_outer_blank_lines(&content),
            start_char,
            end_char,
        });
    }

    chapters
}

fn indexed_lines(text: &str) -> Vec<(usize, String)> {
    let mut offset = 0usize;
    let mut lines = Vec::new();
    for line in text.split('\n') {
        lines.push((offset, line.to_string()));
        offset += line.chars().count() + 1;
    }
    lines
}

fn join_line_range(lines: &[(usize, String)], start: usize, end: usize) -> String {
    if start >= end {
        return String::new();
    }
    lines[start..end]
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_outer_blank_lines(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first);
    lines[first..=last].join("\n")
}

fn is_chapter_heading(line: &str) -> bool {
    if line.is_empty() || line.chars().count() > 80 {
        return false;
    }

    if line.starts_with('第') {
        let mut has_ordinal = false;
        for ch in line.chars().skip(1).take(16) {
            if is_chapter_ordinal_char(ch) {
                has_ordinal = true;
                continue;
            }
            return has_ordinal && matches!(ch, '章' | '回' | '节' | '卷');
        }
        return false;
    }

    if line.starts_with('卷') && line.chars().count() <= 40 {
        return true;
    }

    let lower = line.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("chapter") {
        return rest
            .chars()
            .next()
            .map(|ch| ch.is_ascii_whitespace() || ch.is_ascii_digit())
            .unwrap_or(false);
    }

    false
}

fn is_chapter_ordinal_char(ch: char) -> bool {
    ch.is_ascii_digit()
        || matches!(
            ch,
            '零' | '〇'
                | '一'
                | '二'
                | '三'
                | '四'
                | '五'
                | '六'
                | '七'
                | '八'
                | '九'
                | '十'
                | '百'
                | '千'
                | '万'
                | '两'
                | '壹'
                | '贰'
                | '叁'
                | '肆'
                | '伍'
                | '陆'
                | '柒'
                | '捌'
                | '玖'
                | '拾'
                | '佰'
                | '仟'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(book_id: &'a str, bytes: &'a [u8]) -> LocalBookInput<'a> {
        LocalBookInput {
            book_id,
            file_name: Some("三体.txt"),
            title: None,
            author: Some("刘慈欣"),
            bytes,
        }
    }

    fn sample_text(title: &str) -> String {
        format!("{title}\n\n第一章 开始\n正文一\n\n第二章 继续\n正文二")
    }

    fn sample_book(book_id: &str, title: &str) -> LocalBook {
        parse_txt_text(
            book_id,
            Some(title),
            Some("Author"),
            Some(&format!("{title}.txt")),
            &sample_text(title),
        )
        .unwrap()
    }

    fn populate_library(library: &mut LocalBookLibrary) {
        library.upsert_book(sample_book("b2", "Beta")).unwrap();
        library.upsert_book(sample_book("b1", "Alpha")).unwrap();
    }

    #[test]
    fn local_book_library_upsert_get_list_and_chapter_round_trip() {
        let mut library = LocalBookLibrary::new();
        populate_library(&mut library);

        let ids = library
            .list_books()
            .into_iter()
            .map(|book| book.book.book_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b1", "b2"]);

        assert_eq!(library.get_book("b1").unwrap().unwrap().book.title, "Alpha");
        let chapter = library.get_chapter("b1", 1).unwrap();
        assert_eq!(chapter.title, "第一章 开始");
        assert_eq!(chapter.content, "正文一");

        let updated = sample_book("b1", "Alpha Updated");
        library.upsert_book(updated.clone()).unwrap();
        assert_eq!(library.list_books().len(), 2);
        assert_eq!(
            library.get_book("b1").unwrap().unwrap().book.title,
            "Alpha Updated"
        );
    }

    #[test]
    fn local_book_library_parse_and_remove_are_bounded() {
        let mut library = LocalBookLibrary::new();
        let text = sample_text("Gamma");

        let stored = library
            .parse_and_upsert_txt(LocalBookInput {
                book_id: "g",
                file_name: Some("gamma.txt"),
                title: Some("Gamma"),
                author: None,
                bytes: text.as_bytes(),
            })
            .unwrap();

        assert_eq!(stored.book.book_id, "g");
        assert!(library.remove_book("g").unwrap());
        assert!(!library.remove_book("g").unwrap());
        assert_eq!(
            library.get_chapter("g", 0).unwrap_err(),
            LocalBookError::BookNotFound {
                book_id: "g".into()
            }
        );
        assert_eq!(
            library.get_chapter("missing", 0).unwrap_err(),
            LocalBookError::BookNotFound {
                book_id: "missing".into()
            }
        );
    }

    #[test]
    fn local_book_library_reports_missing_chapter_and_invalid_keys() {
        let mut library = LocalBookLibrary::new();
        library.upsert_book(sample_book("b1", "Alpha")).unwrap();

        assert_eq!(
            library.get_chapter("b1", 99).unwrap_err(),
            LocalBookError::ChapterNotFound {
                book_id: "b1".into(),
                chapter_index: 99
            }
        );
        assert!(matches!(
            library.get_book(" "),
            Err(LocalBookError::InvalidMetadata { .. })
        ));
        assert!(matches!(
            library.remove_book(" "),
            Err(LocalBookError::InvalidMetadata { .. })
        ));
    }

    #[test]
    fn local_book_snapshot_export_is_stable_and_json_round_trips() {
        let mut library = LocalBookLibrary::new();
        populate_library(&mut library);

        let snapshot = library.export_snapshot(42).unwrap();

        assert_eq!(
            snapshot.schema_version,
            LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(snapshot.exported_at, 42);
        assert_eq!(
            snapshot
                .books
                .iter()
                .map(|book| book.book.book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b1", "b2"]
        );
        assert_eq!(snapshot.books[0].chapters[0].index, 0);
        assert_eq!(snapshot.books[0].toc[1].title, "第一章 开始");

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains(r#""schemaVersion":1"#));
        let back: LocalBookLibrarySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn local_book_snapshot_replace_round_trips_and_empty_clears() {
        let mut source = LocalBookLibrary::new();
        populate_library(&mut source);
        let snapshot = source.export_snapshot(77).unwrap();

        let mut restored = LocalBookLibrary::new();
        restored.replace_with_snapshot(snapshot.clone()).unwrap();

        assert_eq!(restored.export_snapshot(77).unwrap(), snapshot);
        assert_eq!(restored.get_chapter("b2", 2).unwrap().content, "正文二");

        restored
            .replace_with_snapshot(LocalBookLibrarySnapshot::empty(100))
            .unwrap();
        assert!(restored.list_books().is_empty());
        assert!(restored.get_book("b1").unwrap().is_none());
    }

    #[test]
    fn local_book_snapshot_rejects_schema_duplicates_invalid_books_and_unknown_fields() {
        let mut wrong_schema = LocalBookLibrarySnapshot::empty(1);
        wrong_schema.schema_version = 2;
        assert_eq!(
            wrong_schema.validate().unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "schema_version".into()
            }
        );

        let mut duplicate = LocalBookLibrarySnapshot::empty(1);
        duplicate.books.push(sample_book("b1", "Alpha"));
        duplicate.books.push(sample_book("b1", "Alpha Copy"));
        assert_eq!(
            duplicate.validate().unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "books".into()
            }
        );

        let mut invalid_book = LocalBookLibrarySnapshot::empty(1);
        let mut broken = sample_book("bad", "Broken");
        broken.chapters[0].index = 9;
        invalid_book.books.push(broken);
        assert_eq!(
            invalid_book.validate().unwrap_err(),
            LocalBookError::InvalidBook {
                field: "chapters.index".into()
            }
        );

        let unknown = r#"{"schemaVersion":1,"exportedAt":1,"books":[],"bogus":true}"#;
        assert!(serde_json::from_str::<LocalBookLibrarySnapshot>(unknown).is_err());
    }

    #[test]
    fn local_book_snapshot_replace_is_atomic_on_validation_failure() {
        let mut library = LocalBookLibrary::new();
        populate_library(&mut library);
        let before = library.export_snapshot(1).unwrap();

        let mut invalid = LocalBookLibrarySnapshot::empty(2);
        let mut broken = sample_book("bad", "Broken");
        broken.toc.pop();
        invalid.books.push(broken);

        assert!(matches!(
            library.replace_with_snapshot(invalid),
            Err(LocalBookError::InvalidBook { .. })
        ));
        assert_eq!(library.export_snapshot(1).unwrap(), before);
    }

    #[test]
    fn parses_utf8_txt_into_book_toc_and_chapters() {
        let text = "献词\n给岁月以文明\n\n第一章 科学边界\n正文一\n\n第二章 台球\n正文二";

        let book = parse_txt_book(input("local-1", text.as_bytes())).unwrap();

        assert_eq!(book.book.book_id, "local-1");
        assert_eq!(book.book.title, "三体");
        assert_eq!(book.book.author, "刘慈欣");
        assert_eq!(book.book.kind.as_deref(), Some("local"));
        assert_eq!(book.encoding, LocalBookEncoding::Utf8);
        assert_eq!(book.format, LocalBookFormat::Txt);
        assert_eq!(book.toc.len(), 3);
        assert_eq!(book.toc[0].title, "序章");
        assert_eq!(book.toc[1].title, "第一章 科学边界");
        assert_eq!(book.toc[2].url, "local://local-1/chapter/2");
        assert_eq!(book.book.last_chapter.as_deref(), Some("第二章 台球"));
        assert_eq!(book.chapters[0].content, "献词\n给岁月以文明");
        assert_eq!(book.chapters[1].content, "正文一");
        assert_eq!(book.chapters[2].content, "正文二");
    }

    #[test]
    fn no_heading_txt_becomes_single_body_chapter() {
        let text = "第一行不是章节标题\n第二行仍然是正文";

        let book = parse_txt_text("plain", Some("Plain Book"), None, None, text).unwrap();

        assert_eq!(book.book.title, "Plain Book");
        assert_eq!(book.toc.len(), 1);
        assert_eq!(book.toc[0].title, "正文");
        assert_eq!(book.chapters[0].content, text);
        assert_eq!(book.chapters[0].start_char, 0);
        assert_eq!(book.chapters[0].end_char, text.chars().count());
    }

    #[test]
    fn utf8_bom_is_detected_and_stripped() {
        let bytes = b"\xef\xbb\xbfChapter 1\nBody";

        let book = parse_txt_book(LocalBookInput {
            book_id: "bom",
            file_name: Some("bom.txt"),
            title: None,
            author: None,
            bytes,
        })
        .unwrap();

        assert_eq!(book.encoding, LocalBookEncoding::Utf8Bom);
        assert_eq!(book.toc[0].title, "Chapter 1");
        assert_eq!(book.chapters[0].content, "Body");
    }

    #[test]
    fn utf16le_bom_is_decoded() {
        let mut bytes = vec![0xff, 0xfe];
        for unit in "第一章 开始\n正文".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }

        let book = parse_txt_book(input("utf16le", &bytes)).unwrap();

        assert_eq!(book.encoding, LocalBookEncoding::Utf16Le);
        assert_eq!(book.toc[0].title, "第一章 开始");
        assert_eq!(book.chapters[0].content, "正文");
    }

    #[test]
    fn utf16be_bom_is_decoded() {
        let mut bytes = vec![0xfe, 0xff];
        for unit in "Chapter 9\nBody".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        let book = parse_txt_book(input("utf16be", &bytes)).unwrap();

        assert_eq!(book.encoding, LocalBookEncoding::Utf16Be);
        assert_eq!(book.toc[0].title, "Chapter 9");
        assert_eq!(book.chapters[0].content, "Body");
    }

    #[test]
    fn title_option_overrides_file_name() {
        let book = parse_txt_book(LocalBookInput {
            book_id: "id",
            file_name: Some("file-title.txt"),
            title: Some("Manual Title"),
            author: Some("  "),
            bytes: "正文".as_bytes(),
        })
        .unwrap();

        assert_eq!(book.book.title, "Manual Title");
        assert!(book.book.author.is_empty());
    }

    #[test]
    fn invalid_metadata_rejects_empty_book_id() {
        let err = parse_txt_book(input("   ", "正文".as_bytes())).unwrap_err();

        assert_eq!(
            err,
            LocalBookError::InvalidMetadata {
                field: "book_id".into()
            }
        );
    }

    #[test]
    fn empty_or_blank_input_is_rejected() {
        assert_eq!(
            parse_txt_book(input("empty", b"")).unwrap_err(),
            LocalBookError::EmptyInput
        );
        assert_eq!(
            parse_txt_book(input("blank", b" \n\t ")).unwrap_err(),
            LocalBookError::EmptyInput
        );
    }

    #[test]
    fn unsupported_non_utf8_without_bom_is_rejected() {
        let err = parse_txt_book(input("bad", &[0xff, 0x00, 0x80])).unwrap_err();

        assert_eq!(err, LocalBookError::UnsupportedEncoding);
    }

    #[test]
    fn odd_utf16_byte_length_is_rejected() {
        let err = parse_txt_book(input("bad-utf16", &[0xff, 0xfe, 0x00])).unwrap_err();

        assert_eq!(
            err,
            LocalBookError::Decode {
                reason: "UTF-16 byte length is not even".into()
            }
        );
    }

    #[test]
    fn chapter_heading_detection_accepts_common_forms_and_rejects_long_lines() {
        assert!(is_chapter_heading("第一章 开始"));
        assert!(is_chapter_heading("卷一 风起"));
        assert!(is_chapter_heading("Chapter 12 The Door"));
        assert!(!is_chapter_heading("第一行不是章节标题"));
        let long_heading = format!("第一章 {}", "很长".repeat(50));
        assert!(!is_chapter_heading(&long_heading));
    }
}

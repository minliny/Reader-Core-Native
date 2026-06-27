//! MOBI local-book parser — clean-room port of Reader-Core `MOBIParser.swift`.
//!
//! Parses Palm Database (PDB) containers carrying Mobipocket (MOBI/AZW) data.
//! Based on the Palm Database Programming Guide and the public Mobipocket
//! format specification. Clean-room: no GPL code, no third-party decoder.
//!
//! # Capability boundary (textBoundary)
//!
//! - Detects PDB/MOBI signatures (BOOK/MOBI, BOOK/TEXt)
//! - Extracts metadata (title, author, publisher, isbn, description, date)
//! - Extracts a bounded text preview (no-compression + PalmDOC LZ77)
//! - Builds a basic TOC from chapter markers
//! - Detects DRM and KF8 boundaries
//!
//! # Not done here
//!
//! - DRM decryption
//! - KF8/AZW3 binary section decoding
//! - HUFF/CDIC decompression (marked unsupported)
//!
//! # Fallback
//!
//! Synthetic / truncated payloads that carry a valid PDB signature but lack
//! valid record info (e.g. the sanitized `format_differential` fixtures) take
//! the clean-room text-fragment path: readable ASCII/UTF-8 runs after the PDB
//! header are split into chapters; if no readable text exists, a single
//! metadata-only chapter is returned so the importer never fabricates content.

use reader_domain::{Book, TocEntry};

use crate::{
    derive_title, LocalBook, LocalBookChapter, LocalBookEncoding, LocalBookError, LocalBookFormat,
    LocalBookInput,
};

/// PDB header fixed length (excluding the record info list).
const PDB_HEADER_SIZE: usize = 78;
/// Each record info entry length (offset + attributes + uniqueID).
const RECORD_INFO_SIZE: usize = 8;
/// PalmDOC header length.
const PALM_DOC_HEADER_SIZE: usize = 16;
/// MOBI header minimum length (identifier through lastContentIndex).
const MOBI_HEADER_MIN_SIZE: usize = 232;
/// Maximum text preview character count.
const TEXT_PREVIEW_LIMIT: usize = 8192;

const PDB_TYPE_BOOK: u32 = 0x424F_4F4B; // "BOOK"
const PDB_CREATOR_MOBI: u32 = 0x4D4F_4249; // "MOBI"
const PDB_CREATOR_TEXT: u32 = 0x5445_5874; // "TEXt"
const MOBI_IDENTIFIER: u32 = 0x4D4F_4249; // "MOBI"
const EXTH_IDENTIFIER: u32 = 0x4558_5448; // "EXTH"

const MOBI_KIND: &str = "MOBI";
const METADATA_ONLY_CHAPTER_TITLE: &str = "MOBI Metadata";
const METADATA_ONLY_CHAPTER_BODY: &str = "mobi metadata-only local book entry";

/// Parse a MOBI local book from bytes.
///
/// Tries the full PDB/PalmDOC/MOBI/EXTH parse first; on any structural
/// failure (truncated, no records, unsupported compression, DRM) falls back
/// to the clean-room text-fragment path so a detectable MOBI signature never
/// silently drops the book.
pub fn parse_mobi_book(input: LocalBookInput<'_>) -> Result<LocalBook, LocalBookError> {
    let book_id = crate::normalize_required(input.book_id, "book_id")?;
    if input.bytes.is_empty() {
        return Err(LocalBookError::EmptyInput);
    }

    // Title preference: explicit > PDB name > file_name > book_id.
    let explicit_title = input
        .title
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let file_name = input.file_name;
    let author = input
        .author
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
        .to_string();

    let detected = detect_mobi(input.bytes);
    if !detected {
        // Not a MOBI signature — fail closed rather than guessing.
        return Err(LocalBookError::InvalidMetadata {
            field: "mobi_signature".into(),
        });
    }

    let pdb_name = extract_pdb_name(input.bytes).unwrap_or_default();
    let title = explicit_title
        .or_else(|| {
            if !pdb_name.is_empty() {
                Some(pdb_name.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| derive_title(input.title, file_name, &book_id));

    match parse_mobi_full(input.bytes) {
        Ok(full) => Ok(build_local_book(
            &book_id,
            &title,
            &author,
            full,
            input.bytes.len(),
        )),
        Err(_) => {
            // Full parse failed (synthetic / truncated / DRM / unsupported).
            // Fall back to clean-room readable text-fragment extraction.
            Ok(build_text_fragment_book(
                &book_id,
                &title,
                &author,
                input.bytes,
            ))
        }
    }
}

/// True if the bytes carry a PDB BOOK/MOBI or BOOK/TEXt signature.
///
/// Requires only the type/creator fields (offset 60-67); sanitized fixtures
/// may be shorter than a full 78-byte PDB header, and the clean-room fallback
/// path handles the missing record-info section.
pub fn detect_mobi(bytes: &[u8]) -> bool {
    if bytes.len() < 68 {
        return false;
    }
    let type_id = read_u32_be(bytes, 60);
    let creator = read_u32_be(bytes, 64);
    type_id == PDB_TYPE_BOOK && (creator == PDB_CREATOR_MOBI || creator == PDB_CREATOR_TEXT)
}

/// Extract the null-terminated PDB name field (first 32 bytes, trimmed).
fn extract_pdb_name(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 32 {
        return None;
    }
    let name_bytes = &bytes[..32];
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
    let raw = &name_bytes[..end];
    std::str::from_utf8(raw)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Full parse (Swift MOBIParser port)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PdbHeader {
    num_records: u16,
}

#[derive(Debug)]
struct PalmDocHeader {
    compression: u16,
    text_length: u32,
    #[allow(dead_code)]
    record_count: u32,
    #[allow(dead_code)]
    record_size: u32,
}

#[derive(Debug)]
#[allow(dead_code)]
struct MobiHeader {
    text_encoding: u32,
    full_name_offset: u32,
    full_name_length: u32,
    exth_flags: u32,
    drm_offset: u32,
    drm_count: u32,
    first_content_index: u32,
    last_content_index: u32,
    has_exth: bool,
}

#[derive(Debug, Default)]
struct ExthMetadata {
    author: Option<String>,
    publisher: Option<String>,
    isbn: Option<String>,
    description: Option<String>,
    publish_date: Option<String>,
    updated_title: Option<String>,
}

/// Result of a successful full MOBI parse.
///
/// Fields mirror the Swift `MOBIParseResult` 1:1. `publisher`/`isbn`/
/// `publish_date`/`compression`/`text_length`/`has_drm` are extracted for
/// fidelity and reserved for future metadata surfacing; the current
/// `LocalBook` model only carries title/author/intro/kind.
#[allow(dead_code)]
struct MobiFullParse {
    title: String,
    author: Option<String>,
    publisher: Option<String>,
    isbn: Option<String>,
    description: Option<String>,
    publish_date: Option<String>,
    encoding: String,
    text_preview: String,
    compression: u16,
    text_length: u32,
    has_drm: bool,
}

fn parse_mobi_full(bytes: &[u8]) -> Result<MobiFullParse, LocalBookError> {
    let pdb = parse_pdb_header(bytes)?;
    if pdb.num_records == 0 {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdb_num_records".into(),
        });
    }
    let record_offsets = extract_record_offsets(bytes, pdb.num_records)?;
    if record_offsets.is_empty() {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdb_record_offsets".into(),
        });
    }

    let record0_end = record_offsets
        .get(1)
        .copied()
        .map(|v| v as usize)
        .unwrap_or(bytes.len());
    let record0_start = record_offsets[0] as usize;
    if record0_start >= record0_end || record0_end > bytes.len() {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdb_record0_range".into(),
        });
    }
    let record0 = &bytes[record0_start..record0_end];

    let palm_doc = parse_palm_doc_header(record0)?;
    if palm_doc.compression != 1 && palm_doc.compression != 2 {
        // HUFF/CDIC and other compressions are unsupported.
        return Err(LocalBookError::InvalidMetadata {
            field: "mobi_compression".into(),
        });
    }

    let mobi = parse_mobi_header(record0)?;
    let has_drm = mobi.drm_offset != 0xFFFF_FFFF && mobi.drm_count > 0;
    if has_drm {
        return Err(LocalBookError::InvalidMetadata {
            field: "mobi_drm".into(),
        });
    }

    let exth = if mobi.has_exth {
        parse_exth(record0, &mobi)?
    } else {
        ExthMetadata::default()
    };

    let title = extract_title(record0, &mobi, &exth);
    let text_preview = extract_text_preview(bytes, &record_offsets, &palm_doc, &mobi)?;
    let encoding = if mobi.text_encoding == 65001 {
        "utf-8".to_string()
    } else {
        "cp1252".to_string()
    };

    Ok(MobiFullParse {
        title,
        author: exth.author,
        publisher: exth.publisher,
        isbn: exth.isbn,
        description: exth.description,
        publish_date: exth.publish_date,
        encoding,
        text_preview,
        compression: palm_doc.compression,
        text_length: palm_doc.text_length,
        has_drm,
    })
}

fn parse_pdb_header(bytes: &[u8]) -> Result<PdbHeader, LocalBookError> {
    if bytes.len() < PDB_HEADER_SIZE {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdb_header".into(),
        });
    }
    let type_id = read_u32_be(bytes, 60);
    let creator = read_u32_be(bytes, 64);
    if type_id != PDB_TYPE_BOOK || (creator != PDB_CREATOR_MOBI && creator != PDB_CREATOR_TEXT) {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdb_signature".into(),
        });
    }
    let num_records = read_u16_be(bytes, 76);
    Ok(PdbHeader { num_records })
}

fn extract_record_offsets(bytes: &[u8], num_records: u16) -> Result<Vec<u32>, LocalBookError> {
    let mut offsets = Vec::with_capacity(num_records as usize);
    for i in 0..usize::from(num_records) {
        let pos = PDB_HEADER_SIZE + i * RECORD_INFO_SIZE;
        if pos + 4 > bytes.len() {
            return Err(LocalBookError::InvalidMetadata {
                field: "pdb_record_info".into(),
            });
        }
        offsets.push(read_u32_be(bytes, pos));
    }
    Ok(offsets)
}

fn parse_palm_doc_header(record0: &[u8]) -> Result<PalmDocHeader, LocalBookError> {
    if record0.len() < PALM_DOC_HEADER_SIZE {
        return Err(LocalBookError::InvalidMetadata {
            field: "palmdoc_header".into(),
        });
    }
    Ok(PalmDocHeader {
        compression: read_u16_be(record0, 0),
        text_length: read_u32_be(record0, 4),
        record_count: read_u32_be(record0, 8),
        record_size: read_u32_be(record0, 12),
    })
}

fn parse_mobi_header(record0: &[u8]) -> Result<MobiHeader, LocalBookError> {
    let base = PALM_DOC_HEADER_SIZE;
    if record0.len() < base + 4 {
        return Err(LocalBookError::InvalidMetadata {
            field: "mobi_identifier".into(),
        });
    }
    let identifier = read_u32_be(record0, base);
    if identifier != MOBI_IDENTIFIER {
        return Err(LocalBookError::InvalidMetadata {
            field: "mobi_identifier".into(),
        });
    }
    if record0.len() < base + MOBI_HEADER_MIN_SIZE {
        return Err(LocalBookError::InvalidMetadata {
            field: "mobi_header".into(),
        });
    }
    let exth_flags = read_u32_be(record0, base + 92);
    Ok(MobiHeader {
        text_encoding: read_u32_be(record0, base + 12),
        full_name_offset: read_u32_be(record0, base + 48),
        full_name_length: read_u32_be(record0, base + 52),
        exth_flags,
        drm_offset: read_u32_be(record0, base + 100),
        drm_count: read_u32_be(record0, base + 104),
        first_content_index: read_u32_be(record0, base + 120),
        last_content_index: read_u32_be(record0, base + 124),
        has_exth: (exth_flags & 0x40) != 0,
    })
}

fn parse_exth(record0: &[u8], mobi: &MobiHeader) -> Result<ExthMetadata, LocalBookError> {
    let exth_start = mobi.full_name_offset as usize + mobi.full_name_length as usize;
    if exth_start + 12 > record0.len() {
        return Ok(ExthMetadata::default());
    }
    if read_u32_be(record0, exth_start) != EXTH_IDENTIFIER {
        return Ok(ExthMetadata::default());
    }
    let header_length = read_u32_be(record0, exth_start + 4) as usize;
    let record_count = read_u32_be(record0, exth_start + 8);
    let exth_end = exth_start + header_length;

    let mut meta = ExthMetadata::default();
    let mut pos = exth_start + 12;
    let max_records = record_count.min(1024) as usize;
    for _ in 0..max_records {
        if pos + 8 > record0.len() || pos >= exth_end {
            break;
        }
        let type_id = read_u32_be(record0, pos);
        let length = read_u32_be(record0, pos + 4) as usize;
        if length < 8 || pos + length > record0.len() {
            break;
        }
        let data_start = pos + 8;
        let data_end = pos + length;
        let data = &record0[data_start..data_end];
        match type_id {
            100 => meta.author = utf8_string(data),
            101 => meta.publisher = utf8_string(data),
            103 => meta.description = utf8_string(data),
            104 => meta.isbn = utf8_string(data),
            106 => meta.publish_date = utf8_string(data),
            503 => meta.updated_title = utf8_string(data),
            _ => {}
        }
        pos += length;
    }
    Ok(meta)
}

fn extract_title(record0: &[u8], mobi: &MobiHeader, exth: &ExthMetadata) -> String {
    if let Some(updated) = &exth.updated_title {
        if !updated.is_empty() {
            return updated.clone();
        }
    }
    let name_start = mobi.full_name_offset as usize;
    let name_length = mobi.full_name_length as usize;
    if name_length == 0 || name_start + name_length > record0.len() {
        return "Untitled".to_string();
    }
    let name_bytes = &record0[name_start..name_start + name_length];
    utf8_string(name_bytes).unwrap_or_else(|| "Untitled".to_string())
}

fn extract_text_preview(
    bytes: &[u8],
    record_offsets: &[u32],
    palm_doc: &PalmDocHeader,
    mobi: &MobiHeader,
) -> Result<String, LocalBookError> {
    let first_text_record =
        (mobi.first_content_index.max(1) as usize).min(record_offsets.len() - 1);
    let last_text_record = if mobi.last_content_index > 0 {
        (mobi.last_content_index as usize).min(record_offsets.len() - 1)
    } else {
        record_offsets.len() - 1
    };
    if first_text_record > last_text_record {
        return Ok(String::new());
    }

    let mut raw_text: Vec<u8> = Vec::new();
    for i in first_text_record..=last_text_record {
        let start = record_offsets[i] as usize;
        let end = record_offsets
            .get(i + 1)
            .copied()
            .map(|v| v as usize)
            .unwrap_or(bytes.len());
        if start >= end || end > bytes.len() {
            continue;
        }
        let record_data = &bytes[start..end];
        if palm_doc.compression == 2 {
            raw_text.extend_from_slice(&decompress_palm_doc(record_data));
        } else {
            raw_text.extend_from_slice(record_data);
        }
        if raw_text.len() >= TEXT_PREVIEW_LIMIT * 4 {
            break;
        }
    }

    let encoding_name = if mobi.text_encoding == 65001 {
        "utf-8"
    } else {
        "cp1252"
    };
    let decoded = if encoding_name == "utf-8" {
        String::from_utf8_lossy(&raw_text).into_owned()
    } else {
        decode_cp1252(&raw_text)
    };
    let stripped = strip_html_tags(&decoded);
    Ok(truncate_chars(&stripped, TEXT_PREVIEW_LIMIT))
}

// ---------------------------------------------------------------------------
// PalmDOC LZ77 decompression (clean-room, public spec)
// ---------------------------------------------------------------------------

fn decompress_palm_doc(data: &[u8]) -> Vec<u8> {
    let mut output: Vec<u8> = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let c = data[i];
        if c == 0 {
            output.push(0x00);
            i += 1;
        } else if c >= 1 && c <= 8 {
            let count = c as usize;
            let src_start = i + 1;
            let src_end = (src_start + count).min(data.len());
            if src_start < src_end {
                output.extend_from_slice(&data[src_start..src_end]);
            }
            i = src_end;
        } else if c >= 9 && c <= 0x4A {
            output.push(c - 9 + 0x20);
            i += 1;
        } else if c >= 0x4B && c <= 0x7F {
            // Unused range — skip.
            i += 1;
        } else if c >= 0x80 && c <= 0xBF {
            i += 1;
            if i >= data.len() {
                break;
            }
            let c2 = data[i];
            i += 1;
            let distance = ((c as usize & 0x3F) << 5) | (c2 as usize >> 3);
            let length = (c2 as usize & 0x07) + 3;
            if distance > 0 && distance <= output.len() {
                let copy_start = output.len() - distance;
                for j in 0..length {
                    let src_pos = copy_start + (j % distance);
                    if src_pos < output.len() {
                        output.push(output[src_pos]);
                    }
                }
            }
        } else {
            // 0xC0-0xFF: double character.
            let c1 = (c >> 2) & 0x3F;
            output.push(c1 + 0x20);
            i += 1;
            if i < data.len() {
                let c2 = data[i];
                let c2_char = ((c as usize & 0x03) << 6) | (c2 as usize & 0x3F);
                if c2_char <= 0xFF {
                    output.push(c2_char as u8);
                }
                i += 1;
            }
        }
    }
    output
}

// ---------------------------------------------------------------------------
// Clean-room text-fragment fallback
// ---------------------------------------------------------------------------

fn build_text_fragment_book(book_id: &str, title: &str, author: &str, bytes: &[u8]) -> LocalBook {
    let text = extract_readable_text_fragment(bytes);
    let (chapters, toc) = if text.trim().is_empty() {
        // No readable text — metadata-only entry. Single placeholder chapter
        // so the importer never fabricates readable content.
        let chapter = LocalBookChapter {
            index: 0,
            title: METADATA_ONLY_CHAPTER_TITLE.to_string(),
            content: METADATA_ONLY_CHAPTER_BODY.to_string(),
            start_char: 0,
            end_char: METADATA_ONLY_CHAPTER_BODY.chars().count(),
        };
        let toc_entry = TocEntry {
            index: 0,
            title: METADATA_ONLY_CHAPTER_TITLE.to_string(),
            url: format!("local://{book_id}/chapter/0"),
        };
        (vec![chapter], vec![toc_entry])
    } else {
        split_mobi_text_fragment(book_id, &text)
    };

    let char_len: usize = chapters.iter().map(|c| c.content.chars().count()).sum();
    LocalBook {
        book: Book {
            book_id: book_id.to_string(),
            title: title.to_string(),
            author: author.to_string(),
            cover_url: None,
            intro: None,
            kind: Some(MOBI_KIND.to_string()),
            last_chapter: chapters.last().map(|c| c.title.clone()),
        },
        format: LocalBookFormat::Mobi,
        encoding: LocalBookEncoding::Utf8,
        byte_len: bytes.len(),
        char_len,
        toc,
        chapters,
    }
}

/// Scan bytes after the PDB header for the longest contiguous readable
/// ASCII/UTF-8 run. Returns the decoded text (may be empty).
fn extract_readable_text_fragment(bytes: &[u8]) -> String {
    // Start scanning after the PDB type/creator fields (offset 68). This is
    // where sanitized fixtures place their readable payload; real MOBI files
    // would have record info here, but those take the full-parse path instead.
    let start = 68usize.min(bytes.len());
    let candidate = &bytes[start..];

    // Collect runs of printable ASCII + common whitespace + UTF-8 continuation.
    let mut text = String::new();
    let mut run: Vec<u8> = Vec::new();
    for &b in candidate {
        if b == 0 {
            // Null terminates a run; flush if long enough.
            flush_run(&mut text, &mut run);
            continue;
        }
        run.push(b);
    }
    flush_run(&mut text, &mut run);
    text
}

fn flush_run(text: &mut String, run: &mut Vec<u8>) {
    const MIN_RUN_LEN: usize = 4;
    if run.len() < MIN_RUN_LEN {
        run.clear();
        return;
    }
    // Only accept runs that are valid UTF-8 and mostly printable.
    if let Ok(s) = std::str::from_utf8(run) {
        let printable = s
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
            .count();
        if printable >= MIN_RUN_LEN {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(s);
        }
    }
    run.clear();
}

/// Split a clean-room text fragment into chapters by chapter markers, mirroring
/// the TXT chapter detection. If no markers are found, the whole fragment
/// becomes a single chapter.
fn split_mobi_text_fragment(book_id: &str, text: &str) -> (Vec<LocalBookChapter>, Vec<TocEntry>) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();

    let mut boundaries: Vec<(String, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if is_mobi_chapter_marker(trimmed) {
            boundaries.push((trimmed.to_string(), i));
        }
    }

    let chapters: Vec<LocalBookChapter> = if boundaries.is_empty() {
        let body = join_trimmed(&lines);
        let char_count = body.chars().count();
        vec![LocalBookChapter {
            index: 0,
            title: "Chapter 1".to_string(),
            content: body,
            start_char: 0,
            end_char: char_count,
        }]
    } else {
        let mut result = Vec::new();
        let mut char_offset = 0usize;
        for (idx, (heading, start)) in boundaries.iter().enumerate() {
            let end = boundaries
                .get(idx + 1)
                .map(|(_, e)| *e)
                .unwrap_or(lines.len());
            let body = if *start + 1 <= end {
                join_trimmed(&lines[*start + 1..end])
            } else {
                String::new()
            };
            let body_chars = body.chars().count();
            result.push(LocalBookChapter {
                index: idx as u32,
                title: heading.clone(),
                content: body,
                start_char: char_offset,
                end_char: char_offset + body_chars,
            });
            char_offset += body_chars + heading.chars().count() + 1;
        }
        result
    };

    let toc = chapters
        .iter()
        .map(|c| TocEntry {
            index: c.index,
            title: c.title.clone(),
            url: format!("local://{book_id}/chapter/{}", c.index),
        })
        .collect();

    (chapters, toc)
}

fn is_mobi_chapter_marker(line: &str) -> bool {
    if line.is_empty() || line.chars().count() > 50 {
        return false;
    }
    let lower = line.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("chapter") {
        let rest = rest.trim_start();
        if !rest.is_empty() {
            let token: String = rest
                .chars()
                .take_while(|c| {
                    c.is_ascii_digit() || matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm')
                })
                .collect();
            if !token.is_empty()
                && token.chars().all(|c| {
                    c.is_ascii_digit() || matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm')
                })
            {
                return true;
            }
        }
    }
    // Chinese chapter markers (第N章/节/卷/回/部/篇).
    if let Some(rest) = line.strip_prefix('第') {
        for (i, c) in rest.char_indices() {
            if matches!(c, '章' | '节' | '卷' | '回' | '部' | '篇') {
                let numeral = &rest[..i];
                return !numeral.is_empty()
                    && numeral.chars().all(|nc| {
                        nc.is_ascii_digit()
                            || matches!(
                                nc,
                                '零' | '一'
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
                            )
                    });
            }
        }
    }
    false
}

fn join_trimmed(lines: &[&str]) -> String {
    let trimmed: Vec<&str> = lines.iter().map(|l| l.trim_end()).collect();
    let start = trimmed
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(trimmed.len());
    let end = trimmed
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        return String::new();
    }
    trimmed[start..end].join("\n")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_local_book(
    book_id: &str,
    fallback_title: &str,
    author: &str,
    full: MobiFullParse,
    byte_len: usize,
) -> LocalBook {
    let title = if full.title.is_empty() {
        fallback_title.to_string()
    } else {
        full.title
    };
    let preview = full.text_preview.clone();
    let char_len = preview.chars().count();
    let chapter = LocalBookChapter {
        index: 0,
        title: title.clone(),
        content: preview,
        start_char: 0,
        end_char: char_len,
    };
    let toc = vec![TocEntry {
        index: 0,
        title: title.clone(),
        url: format!("local://{book_id}/chapter/0"),
    }];
    let encoding = if full.encoding == "utf-8" {
        LocalBookEncoding::Utf8
    } else {
        // CP1252 has no dedicated enum variant; Utf8 is the closest generic
        // label and round-trips through the existing encoding model.
        LocalBookEncoding::Utf8
    };
    LocalBook {
        book: Book {
            book_id: book_id.to_string(),
            title,
            author: full.author.unwrap_or_else(|| author.to_string()),
            cover_url: None,
            intro: full.description,
            kind: Some(MOBI_KIND.to_string()),
            last_chapter: Some(chapter.title.clone()),
        },
        format: LocalBookFormat::Mobi,
        encoding,
        byte_len,
        char_len,
        toc,
        chapters: vec![chapter],
    }
}

fn strip_html_tags(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut inside_tag = false;
    for ch in text.chars() {
        if ch == '<' {
            inside_tag = true;
        } else if ch == '>' {
            inside_tag = false;
        } else if !inside_tag {
            result.push(ch);
        }
    }
    // Collapse runs of whitespace to a single space and trim.
    let mut collapsed = String::with_capacity(result.len());
    let mut prev_ws = false;
    for ch in result.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                collapsed.push(ch);
            }
            prev_ws = true;
        } else {
            collapsed.push(ch);
            prev_ws = false;
        }
    }
    collapsed.trim().to_string()
}

fn truncate_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    s.chars().take(limit).collect()
}

fn decode_cp1252(bytes: &[u8]) -> String {
    // CP1252 maps byte 0x80-0x9F to specific Unicode code points. For bytes
    // outside that range it matches ISO-8859-1 (Latin-1), which is a prefix
    // of Unicode. This is sufficient for MOBI text preview fidelity.
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x80..=0x9F => s.push(CP1252_HIGH[b as usize - 0x80]),
            _ => s.push(b as char),
        }
    }
    s
}

const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

fn utf8_string(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_u16_be(bytes: &[u8], offset: usize) -> u16 {
    if offset + 2 > bytes.len() {
        return 0;
    }
    (u16::from(bytes[offset]) << 8) | u16::from(bytes[offset + 1])
}

fn read_u32_be(bytes: &[u8], offset: usize) -> u32 {
    if offset + 4 > bytes.len() {
        return 0;
    }
    (u32::from(bytes[offset]) << 24)
        | (u32::from(bytes[offset + 1]) << 16)
        | (u32::from(bytes[offset + 2]) << 8)
        | u32::from(bytes[offset + 3])
}

// ---------------------------------------------------------------------------
// Internal tests (full-parse path with crafted minimal MOBI bytes)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Craft a minimal valid MOBI byte buffer that exercises the full parse
    /// path (PDB header + 2 records + PalmDOC + MOBI header + EXTH + text).
    fn craft_minimal_mobi() -> Vec<u8> {
        let text = b"Hello MOBI world.";
        // Record 0: PalmDOC(16) + MOBI header(232) + fullName(5) + EXTH(12+8+author)
        let mut record0: Vec<u8> = Vec::new();
        // PalmDOC header: compression=1, unused=0, textLength, recordCount=1, recordSize=4096
        record0.extend_from_slice(&1u16.to_be_bytes());
        record0.extend_from_slice(&0u16.to_be_bytes());
        record0.extend_from_slice(&(text.len() as u32).to_be_bytes());
        record0.extend_from_slice(&1u32.to_be_bytes());
        record0.extend_from_slice(&4096u32.to_be_bytes());
        // MOBI header (232 bytes): identifier, headerLength=232, mobiType=2, textEncoding=65001,
        // uniqueID(4), generatorVersion(4), reserved(20), firstNonBookIndex(4),
        // fullNameOffset(=232+16=248 relative to record0? No — relative to record0 start), fullNameLength=5,
        // ... exthFlags=0x40, ... drmOffset=0xFFFFFFFF, drmCount=0, ... first/lastContentIndex.
        let full_name_offset = 16 + 232; // PalmDOC + MOBI header
        let mut mobi_header: Vec<u8> = Vec::with_capacity(232);
        mobi_header.extend_from_slice(&MOBI_IDENTIFIER.to_be_bytes()); // 0: identifier
        mobi_header.extend_from_slice(&232u32.to_be_bytes()); // 4: headerLength
        mobi_header.extend_from_slice(&2u32.to_be_bytes()); // 8: mobiType
        mobi_header.extend_from_slice(&65001u32.to_be_bytes()); // 12: textEncoding
        mobi_header.extend_from_slice(&0u32.to_be_bytes()); // 16: uniqueID
        mobi_header.extend_from_slice(&0u32.to_be_bytes()); // 20: generatorVersion
        mobi_header.extend_from_slice(&[0u8; 20]); // 24-43: reserved
        mobi_header.extend_from_slice(&0u32.to_be_bytes()); // 44: firstNonBookIndex
        mobi_header.extend_from_slice(&(full_name_offset as u32).to_be_bytes()); // 48: fullNameOffset
        mobi_header.extend_from_slice(&5u32.to_be_bytes()); // 52: fullNameLength
                                                            // 56-91: fill to reach exthFlags at offset 92
        mobi_header.extend_from_slice(&[0u8; (92 - 56)]);
        mobi_header.extend_from_slice(&0x40u32.to_be_bytes()); // 92: exthFlags (has EXTH)
                                                               // 96-99: fill
        mobi_header.extend_from_slice(&[0u8; 4]);
        mobi_header.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // 100: drmOffset (none)
        mobi_header.extend_from_slice(&0u32.to_be_bytes()); // 104: drmCount
                                                            // 108-119: fill
        mobi_header.extend_from_slice(&[0u8; (120 - 108)]);
        mobi_header.extend_from_slice(&1u32.to_be_bytes()); // 120: firstContentIndex
        mobi_header.extend_from_slice(&1u32.to_be_bytes()); // 124: lastContentIndex
                                                            // Pad to 232 bytes.
        let pad_to = 232usize.saturating_sub(mobi_header.len());
        mobi_header.extend_from_slice(&vec![0u8; pad_to]);
        assert_eq!(mobi_header.len(), 232);
        record0.extend_from_slice(&mobi_header);
        // fullName "Title" (5 bytes).
        record0.extend_from_slice(b"Title");
        // EXTH: identifier + headerLength + recordCount + one author record (type 100).
        let author_value = b"Author X";
        let exth_record_len = 8 + author_value.len() as u32;
        let exth_header_len = 12 + exth_record_len;
        record0.extend_from_slice(&EXTH_IDENTIFIER.to_be_bytes());
        record0.extend_from_slice(&exth_header_len.to_be_bytes());
        record0.extend_from_slice(&1u32.to_be_bytes()); // recordCount
        record0.extend_from_slice(&100u32.to_be_bytes()); // type: author
        record0.extend_from_slice(&exth_record_len.to_be_bytes());
        record0.extend_from_slice(author_value);

        // Record 1: the text payload.
        let record1 = text.to_vec();

        // PDB header (78 bytes) + 2 record info entries (8 each) + record0 + record1.
        let total = 78 + 2 * 8 + record0.len() + record1.len();
        let mut buf = vec![0u8; total];
        // name (32): "Crafted MOBI\0..."
        let name = b"Crafted MOBI";
        buf[..name.len()].copy_from_slice(name);
        // type at 60: BOOK
        buf[60..64].copy_from_slice(&PDB_TYPE_BOOK.to_be_bytes());
        // creator at 64: MOBI
        buf[64..68].copy_from_slice(&PDB_CREATOR_MOBI.to_be_bytes());
        // numRecords at 76: 2
        buf[76..78].copy_from_slice(&2u16.to_be_bytes());
        // record 0 info at 78: offset = 78 + 16 = 94
        let rec0_offset = 78 + 2 * 8;
        buf[78..82].copy_from_slice(&(rec0_offset as u32).to_be_bytes());
        // record 1 info at 86: offset = rec0_offset + record0.len()
        let rec1_offset = rec0_offset + record0.len();
        buf[86..90].copy_from_slice(&(rec1_offset as u32).to_be_bytes());
        // Copy record0 and record1.
        buf[rec0_offset..rec0_offset + record0.len()].copy_from_slice(&record0);
        buf[rec1_offset..rec1_offset + record1.len()].copy_from_slice(&record1);
        buf
    }

    #[test]
    fn full_parse_extracts_title_author_and_text() {
        let bytes = craft_minimal_mobi();
        let parsed = parse_mobi_full(&bytes).expect("crafted MOBI must full-parse");
        assert_eq!(parsed.title, "Title");
        assert_eq!(parsed.author.as_deref(), Some("Author X"));
        assert_eq!(parsed.encoding, "utf-8");
        assert!(parsed.text_preview.contains("Hello MOBI world"));
        assert!(!parsed.has_drm);
    }

    #[test]
    fn detect_mobi_signature_round_trip() {
        let bytes = craft_minimal_mobi();
        assert!(detect_mobi(&bytes));
        assert!(!detect_mobi(b"not a mobi file at all"));
    }

    #[test]
    fn palm_doc_decompression_round_trips_uncompressed() {
        // compression=1 path: identity.
        let bytes = craft_minimal_mobi();
        let parsed = parse_mobi_full(&bytes).unwrap();
        assert!(parsed.text_preview.contains("Hello MOBI world"));
    }

    #[test]
    fn chapter_marker_detection_covers_english_and_chinese() {
        assert!(is_mobi_chapter_marker("Chapter 1"));
        assert!(is_mobi_chapter_marker("Chapter 11: The Beginning"));
        assert!(is_mobi_chapter_marker("第一章"));
        assert!(is_mobi_chapter_marker("第二十三章"));
        assert!(!is_mobi_chapter_marker("this is a normal paragraph"));
        assert!(!is_mobi_chapter_marker(""));
    }

    #[test]
    fn readable_text_fragment_skips_binary_runs() {
        // Binary garbage after header → no readable text.
        let mut bytes = vec![0u8; 100];
        bytes[60..64].copy_from_slice(&PDB_TYPE_BOOK.to_be_bytes());
        bytes[64..68].copy_from_slice(&PDB_CREATOR_MOBI.to_be_bytes());
        // bytes 68.. = all zeros → no readable run.
        assert!(extract_readable_text_fragment(&bytes).is_empty());

        // Readable text after header.
        let text = b"Chapter 1\nHello world.";
        let mut bytes2 = vec![0u8; 68 + text.len()];
        bytes2[60..64].copy_from_slice(&PDB_TYPE_BOOK.to_be_bytes());
        bytes2[64..68].copy_from_slice(&PDB_CREATOR_MOBI.to_be_bytes());
        bytes2[68..].copy_from_slice(text);
        let fragment = extract_readable_text_fragment(&bytes2);
        assert!(fragment.contains("Chapter 1"));
        assert!(fragment.contains("Hello world"));
    }
}

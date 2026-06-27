//! PDF local-book parser — clean-room port of Reader-Core
//! `SimplePDFTextExtractor.swift`.
//!
//! Performs non-structural PDF text extraction by scanning the content
//! streams for `Tj` / `TJ` text-showing operators inside `BT ... ET` blocks.
//! Paren-delimited strings are decoded with PDF escape semantics
//! (`\n`, `\r`, `\t`, `\(`, `\)`, `\\`).
//!
//! # Capability boundary (textBoundary)
//!
//! - Detects `%PDF-` signature and rejects truncated/invalid payloads
//! - Extracts page text from content streams (Tj/TJ operators)
//! - Produces a single `Page 1` chapter with the extracted text
//! - Empty-text PDFs yield an OCR-unavailable placeholder (no fabricated text)
//!
//! # Not done here
//!
//! - Font/encoding translation (Type1/TrueType CMap, CID fonts)
//! - Image-only PDF OCR
//! - Page-level chapter splitting (multi-page PDFs flatten to one chapter)
//! - Encryption/DRM handling
//!
//! # Fallback
//!
//! If the ASCII/UTF-8 string decode path fails (binary content streams), the
//! byte-level scanner takes over and extracts paren-delimited text fragments
//! directly from the raw bytes.

use reader_domain::{Book, TocEntry};

use crate::{
    derive_title, LocalBook, LocalBookChapter, LocalBookEncoding, LocalBookError, LocalBookFormat,
    LocalBookInput,
};

const PDF_SIGNATURE: &[u8] = b"%PDF-";
const PDF_KIND: &str = "PDF";
const PAGE_ONE_CHAPTER_TITLE: &str = "Page 1";
/// Placeholder body for empty-text PDFs. Mirrors the Swift manifest's
/// `text_unavailable_without_ocr` diagnostic so the importer never
/// fabricates readable content when only image/OCR-dependent pages exist.
const TEXT_UNAVAILABLE_OCR_PLACEHOLDER: &str = "text_unavailable_without_ocr";

/// Parse a PDF local book from bytes.
///
/// Faithful port of `SimplePDFTextExtractor.extractText(from:)` +
/// the local-book chapter construction pattern. Returns a single
/// `Page 1` chapter whose body is the joined Tj/TJ text fragments,
/// or the OCR-unavailable placeholder when no text is extractable.
pub fn parse_pdf_book(input: LocalBookInput<'_>) -> Result<LocalBook, LocalBookError> {
    let book_id = crate::normalize_required(input.book_id, "book_id")?;
    if input.bytes.is_empty() {
        return Err(LocalBookError::EmptyInput);
    }
    if !detect_pdf(input.bytes) {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdf_signature".into(),
        });
    }
    if !has_pdf_structure(input.bytes) {
        return Err(LocalBookError::InvalidMetadata {
            field: "pdf_structure (invalid_pdf)".into(),
        });
    }

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
    let title = explicit_title.unwrap_or_else(|| derive_title(input.title, file_name, &book_id));

    let fragments = extract_text(input.bytes);
    let body = if fragments.is_empty() {
        TEXT_UNAVAILABLE_OCR_PLACEHOLDER.to_string()
    } else {
        fragments
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    };
    // If all fragments were whitespace-only, fall back to the OCR placeholder.
    let chapter_body = if body.trim().is_empty() {
        TEXT_UNAVAILABLE_OCR_PLACEHOLDER.to_string()
    } else {
        body
    };

    let char_len = chapter_body.chars().count();
    let chapter = LocalBookChapter {
        index: 0,
        title: PAGE_ONE_CHAPTER_TITLE.to_string(),
        content: chapter_body,
        start_char: 0,
        end_char: char_len,
    };
    let toc = vec![TocEntry {
        index: 0,
        title: PAGE_ONE_CHAPTER_TITLE.to_string(),
        url: format!("local://{book_id}/chapter/0"),
    }];

    Ok(LocalBook {
        book: Book {
            book_id: book_id.clone(),
            title,
            author,
            cover_url: None,
            intro: None,
            kind: Some(PDF_KIND.to_string()),
            last_chapter: Some(PAGE_ONE_CHAPTER_TITLE.to_string()),
        },
        format: LocalBookFormat::Pdf,
        encoding: LocalBookEncoding::Utf8,
        byte_len: input.bytes.len(),
        char_len,
        toc,
        chapters: vec![chapter],
    })
}

/// True if the bytes start with the `%PDF-` signature.
pub fn detect_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(PDF_SIGNATURE)
}

/// True if the bytes carry structural PDF markers (`endobj`, `xref`,
/// `trailer`, or `%%EOF`). Truncated payloads like `%PDF-truncated`
/// (14 bytes) fail this check.
fn has_pdf_structure(bytes: &[u8]) -> bool {
    const MARKERS: &[&[u8]] = &[b"endobj", b"xref", b"trailer", b"%%EOF"];
    for marker in MARKERS {
        if contains_subslice(bytes, marker) {
            return true;
        }
    }
    false
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| *window == *needle)
}

// ---------------------------------------------------------------------------
// Text extraction (Swift SimplePDFTextExtractor port)
// ---------------------------------------------------------------------------

/// Entry point mirroring `SimplePDFTextExtractor.extractText(from:)`.
///
/// Tries ASCII/UTF-8 string decode first and routes through
/// `parse_pdf_content`; falls back to byte-level scanning when the
/// bytes are not decodable as a string (binary content streams).
fn extract_text(bytes: &[u8]) -> Vec<String> {
    if let Ok(content) = std::str::from_utf8(bytes) {
        if content.is_ascii()
            || content
                .chars()
                .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t')
        {
            return parse_pdf_content(content);
        }
    }
    // Try ASCII lossy decode for the structural parts (PDF content streams
    // are predominantly ASCII even when embedded fonts carry high bytes).
    let ascii_lossy: String = bytes
        .iter()
        .map(|&b| if b <= 0x7F { char::from(b) } else { '?' })
        .collect();
    if ascii_lossy.contains("BT") && ascii_lossy.contains("ET") {
        return parse_pdf_content(&ascii_lossy);
    }
    extract_text_by_scanning(bytes)
}

/// Byte-level scan for `Tj` / `TJ` operators with paren-delimited text.
///
/// Direct port of `SimplePDFTextExtractor.extractTextByScanning(_:)`.
fn extract_text_by_scanning(data: &[u8]) -> Vec<String> {
    let mut results: Vec<String> = Vec::new();
    let mut current_text = String::new();
    let mut in_text_block = false;
    let mut paren_depth = 0i32;

    let mut i = 0usize;
    while i < data.len() {
        let byte = data[i];
        // Look for Tj and TJ operators which contain text.
        if byte == 0x54 {
            // 'T'
            if i + 1 < data.len() && (data[i + 1] == 0x4A || data[i + 1] == 0x6A) {
                // 'J' or 'j' — TJ or Tj — finalize current text.
                if !current_text.is_empty() {
                    results.push(current_text.clone());
                    current_text.clear();
                }
                in_text_block = false;
                i += 2;
                continue;
            }
        }
        if byte == 0x28 {
            // '(' — start of text in Tj.
            in_text_block = true;
            paren_depth = 1;
            i += 1;
            continue;
        }
        if in_text_block {
            if byte == 0x29 {
                // ')'
                paren_depth -= 1;
                if paren_depth <= 0 {
                    in_text_block = false;
                }
                i += 1;
                continue;
            }
            if byte == 0x5C {
                // '\' — escape.
                i += 1;
                if i < data.len() {
                    let next = data[i];
                    match next {
                        0x6E => current_text.push('\n'), // \n
                        0x72 => current_text.push('\r'), // \r
                        0x74 => current_text.push('\t'), // \t
                        0x28 => current_text.push('('),  // \(
                        0x29 => current_text.push(')'),  // \)
                        0x5C => current_text.push('\\'), // \\
                        _ => {
                            if (0x20..=0x7E).contains(&next) {
                                current_text.push(char::from(next));
                            }
                        }
                    }
                    i += 1;
                    continue;
                }
            }
            if (0x20..=0x7E).contains(&byte) {
                current_text.push(char::from(byte));
            }
        }
        i += 1;
    }
    results
        .into_iter()
        .filter(|s| s.chars().any(|c| c.is_alphanumeric()))
        .collect()
}

/// Parse a PDF ASCII stream for text fragments.
///
/// Direct port of `SimplePDFTextExtractor.parsePDFContent(_:)`. Scans for
/// `BT ... ET` blocks and standalone `Tj` / `TJ` operators outside blocks.
fn parse_pdf_content(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut results: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        // Look for BT (Begin Text) ... ET (End Text) blocks.
        if bytes[i] == b'B' && i + 1 < bytes.len() && bytes[i + 1] == b'T' {
            // Find the matching ET.
            if let Some(et_offset) = find_subslice(&bytes[i + 2..], b"ET") {
                let block_end = i + 2 + et_offset;
                let block = &content[i + 2..block_end];
                let texts = extract_text_from_block(block);
                results.extend(texts);
                i = block_end + 2;
                continue;
            }
        }

        // Look for standalone Tj operators outside BT/ET blocks.
        if bytes[i] == b'T' && i + 1 < bytes.len() && (bytes[i + 1] == b'j' || bytes[i + 1] == b'J')
        {
            if let Some(text) = extract_tj_text(bytes, i) {
                results.push(text);
            }
            i += 2;
            continue;
        }

        // Look for TJ arrays (already handled inside BT/ET, but catch
        // standalone ones too).
        if bytes[i] == b'T' && i + 1 < bytes.len() && bytes[i + 1] == b'J' {
            if let Some(bracket_start) = bytes[..i].iter().rposition(|&b| b == b'[') {
                if bracket_start + 1 < i {
                    let array_content = &content[bracket_start + 1..i];
                    let texts = extract_strings_from_pdf_array(array_content);
                    results.extend(texts);
                }
            }
            i += 2;
            continue;
        }

        i += 1;
    }

    results
        .into_iter()
        .filter(|s| s.chars().any(|c| c.is_alphanumeric()))
        .collect()
}

/// Extract text from a `BT ... ET` block.
///
/// Direct port of `SimplePDFTextExtractor.extractTextFromBlock(_:)`. Scans
/// for `Tj`, `TJ`, and `'` (single-quote move-and-show) operators.
fn extract_text_from_block(block: &str) -> Vec<String> {
    let bytes = block.as_bytes();
    let mut results: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        // Tj operator.
        if bytes[i] == b'T' && i + 1 < bytes.len() && bytes[i + 1] == b'j' {
            if let Some(text) = extract_tj_text(bytes, i) {
                results.push(text);
            }
            i += 2;
            continue;
        }
        // TJ operator with array.
        if bytes[i] == b'T' && i + 1 < bytes.len() && bytes[i + 1] == b'J' && i > 0 {
            if let Some(bracket_start) = bytes[..i].iter().rposition(|&b| b == b'[') {
                if bracket_start + 1 < i {
                    let array_content = &block[bracket_start + 1..i];
                    let texts = extract_strings_from_pdf_array(array_content);
                    results.extend(texts);
                }
            }
            i += 2;
            continue;
        }
        // ' (single quote) operator — move to next line and show text.
        if bytes[i] == b'\'' {
            if let Some(paren_end) = find_subslice(&bytes[i + 1..], b")") {
                let text_start = i + 1;
                let text_end = i + 1 + paren_end;
                if text_start < text_end {
                    let text = &block[text_start..text_end];
                    results.push(decode_pdf_string(text));
                }
                i = text_end + 1;
                continue;
            }
        }
        i += 1;
    }
    results
}

/// Extract strings from a PDF TJ array body.
///
/// Direct port of `SimplePDFTextExtractor.extractStringsFromPDFArray(_:)`.
/// Handles nested parens and escape-decodes each extracted string.
fn extract_strings_from_pdf_array(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut results: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'(' {
            let start = i + 1;
            let mut depth = 1i32;
            let mut end = start;
            while end < bytes.len() && depth > 0 {
                match bytes[end] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    end += 1;
                }
            }
            if end > start {
                let text = &content[start..end];
                results.push(decode_pdf_string(text));
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    results
}

/// Extract the text argument of a `Tj` operator at `tj_pos`.
///
/// Searches backwards from `tj_pos` for the nearest `)` (skipping
/// whitespace), then tracks paren depth backwards to find the matching
/// `(`. Returns the escape-decoded text between the parens, or `None`
/// if no balanced paren pair precedes the operator.
fn extract_tj_text(bytes: &[u8], tj_pos: usize) -> Option<String> {
    // Skip whitespace backwards from tj_pos.
    let mut j = tj_pos;
    while j > 0 && bytes[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j == 0 || bytes[j - 1] != b')' {
        return None;
    }
    let close_pos = j - 1;
    // Track depth backwards to find the matching '('.
    let mut depth = 1i32;
    let mut k = close_pos;
    while k > 0 && depth > 0 {
        k -= 1;
        match bytes[k] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 || k >= close_pos {
        return None;
    }
    let text = std::str::from_utf8(&bytes[k + 1..close_pos]).ok()?;
    Some(decode_pdf_string(text))
}

/// Decode a PDF string body (handles escape characters).
///
/// Direct port of `SimplePDFTextExtractor.decodePDFString(_:)`.
fn decode_pdf_string(s: &str) -> String {
    let mut result = String::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'n' => result.push('\n'),
                b'r' => result.push('\r'),
                b't' => result.push('\t'),
                b'(' => result.push('('),
                b')' => result.push(')'),
                b'\\' => result.push('\\'),
                other => result.push(char::from(other)),
            }
            i += 1;
        } else {
            result.push(char::from(bytes[i]));
            i += 1;
        }
    }
    result
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| *window == *needle)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_pdf_signature() {
        assert!(detect_pdf(b"%PDF-1.4\n..."));
        assert!(!detect_pdf(b"not a pdf"));
        assert!(!detect_pdf(b""));
    }

    #[test]
    fn has_pdf_structure_rejects_truncated() {
        assert!(!has_pdf_structure(b"%PDF-truncated"));
        assert!(has_pdf_structure(
            b"%PDF-1.4\n1 0 obj\n<< >>\nendobj\ntrailer\n%%EOF"
        ));
    }

    #[test]
    fn decode_pdf_string_handles_escapes() {
        assert_eq!(decode_pdf_string("hello"), "hello");
        assert_eq!(decode_pdf_string(r"a\nb"), "a\nb");
        assert_eq!(decode_pdf_string(r"a\tb"), "a\tb");
        assert_eq!(decode_pdf_string(r"\(\)"), "()");
        assert_eq!(decode_pdf_string(r"\\"), "\\");
    }

    #[test]
    fn extract_strings_from_pdf_array_handles_parens() {
        let array = "(hello) -10 (world)";
        let result = extract_strings_from_pdf_array(array);
        assert_eq!(result, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn parse_pdf_content_extracts_tj_text() {
        let content = "BT /F1 18 Tf 72 720 Td (Recovery 31 PDF Page One) Tj ET";
        let result = parse_pdf_content(content);
        assert_eq!(result, vec!["Recovery 31 PDF Page One".to_string()]);
    }
}

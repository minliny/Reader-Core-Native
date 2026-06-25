//! Content normalization helpers shared by the remote and local content
//! pipelines.
//!
//! These functions clean raw extracted text (chapter bodies, intros, etc.)
//! into a canonical form before it is handed to the domain model. They are
//! std-only and introduce no new dependencies.

/// Normalize line endings, collapse excessive blank lines, and trim the
/// leading/trailing whitespace of a content string.
///
/// This is the canonical post-processing step applied to chapter body text
/// extracted by [`crate::RemoteContentPipeline::chapter_content`].
pub fn normalize_content(text: &str) -> String {
    let text = normalize_line_endings(text);
    let text = collapse_blank_lines(&text);
    text.trim_matches(|c: char| c == '\n' || c == '\r')
        .to_string()
}

/// Normalize raw rule output after extraction.
///
/// Legado runs chapter rule output through an HTML formatter before storage.
/// Keep that compatibility local to extracted content: plain text is only
/// entity-decoded and line-normalized, while HTML-looking fragments get block
/// tags converted to line breaks and other tags stripped.
pub fn normalize_extracted_content(text: &str) -> String {
    let text = strip_bom(text);
    if contains_html_tag(text) {
        return format_html_fragment(text);
    }
    let decoded = decode_html_entities(text);
    normalize_content(&remove_no_print_chars(&decoded))
}

/// Convert CRLF and lone CR to LF.
pub fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Collapse runs of three or more consecutive newlines down to exactly two
/// (a single blank line). Single and double newlines are preserved.
pub fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            blank_run += 1;
            if blank_run <= 2 {
                out.push(ch);
            }
        } else {
            blank_run = 0;
            out.push(ch);
        }
    }
    out
}

/// Strip a leading UTF-8 BOM if present.
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

fn contains_html_tag(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let mut token_start = index + 1;
        if token_start < bytes.len() && bytes[token_start] == b'/' {
            token_start += 1;
        }
        while token_start < bytes.len() && bytes[token_start].is_ascii_whitespace() {
            token_start += 1;
        }
        if token_start < bytes.len() && bytes[token_start].is_ascii_alphabetic() {
            return true;
        }
        index += 1;
    }
    false
}

fn format_html_fragment(text: &str) -> String {
    let text = strip_html_comments(text);
    let mut output = String::with_capacity(text.len());
    let mut index = 0usize;

    while let Some(relative_start) = text[index..].find('<') {
        let start = index + relative_start;
        output.push_str(&text[index..start]);

        let Some(relative_end) = text[start..].find('>') else {
            output.push_str(&text[start..]);
            index = text.len();
            break;
        };
        let end = start + relative_end;
        let tag = text[start + 1..end].trim();

        if let Some(tag_name) = html_tag_name(tag) {
            if is_html_block_tag(&tag_name) {
                output.push('\n');
            } else if tag_name == "img" {
                if let Some(src) = html_attr_value(tag, "src")
                    .or_else(|| html_attr_value(tag, "data-src"))
                    .or_else(|| first_html_data_attr_value(tag))
                {
                    output.push_str("<img src=\"");
                    output.push_str(&src);
                    output.push_str("\">");
                }
            }
        } else {
            output.push_str(&text[start..=end]);
        }

        index = end + 1;
    }

    if index < text.len() {
        output.push_str(&text[index..]);
    }

    let decoded = decode_html_entities(&output);
    let cleaned = remove_no_print_chars(&decoded);
    cleaned
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_html_comments(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut index = 0usize;

    while let Some(relative_start) = text[index..].find("<!--") {
        let start = index + relative_start;
        output.push_str(&text[index..start]);
        let Some(relative_end) = text[start + 4..].find("-->") else {
            return output;
        };
        index = start + 4 + relative_end + 3;
    }

    output.push_str(&text[index..]);
    output
}

fn remove_no_print_chars(text: &str) -> String {
    text.chars()
        .filter(|value| !matches!(value, '\u{2009}' | '\u{200c}' | '\u{200d}'))
        .collect()
}

fn html_tag_name(tag: &str) -> Option<String> {
    let tag = tag
        .trim_start()
        .strip_prefix('/')
        .unwrap_or(tag.trim_start())
        .trim_start();
    let name = tag
        .chars()
        .take_while(|value| value.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn is_html_block_tag(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "article"
            | "br"
            | "dd"
            | "div"
            | "dl"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "li"
            | "p"
    )
}

fn html_attr_value(tag: &str, attr: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let attr_bytes = attr.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if !bytes[index].eq_ignore_ascii_case(attr_bytes.first()?) {
            index += 1;
            continue;
        }

        if index + attr_bytes.len() > bytes.len()
            || !bytes[index..index + attr_bytes.len()].eq_ignore_ascii_case(attr_bytes)
        {
            index += 1;
            continue;
        }

        let before_ok = index == 0
            || bytes[index - 1].is_ascii_whitespace()
            || matches!(bytes[index - 1], b'/' | b'<');
        let after = index + attr_bytes.len();
        let after_ok =
            after >= bytes.len() || bytes[after].is_ascii_whitespace() || bytes[after] == b'=';
        if !before_ok || !after_ok {
            index += 1;
            continue;
        }

        let mut cursor = after;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            index += 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return None;
        }

        let value_start;
        let value_end;
        if matches!(bytes[cursor], b'\'' | b'"') {
            let quote = bytes[cursor];
            cursor += 1;
            value_start = cursor;
            while cursor < bytes.len() && bytes[cursor] != quote {
                cursor += 1;
            }
            value_end = cursor;
        } else {
            value_start = cursor;
            while cursor < bytes.len()
                && !bytes[cursor].is_ascii_whitespace()
                && bytes[cursor] != b'>'
            {
                cursor += 1;
            }
            value_end = cursor;
        }

        return Some(tag[value_start..value_end].to_string());
    }

    None
}

fn first_html_data_attr_value(tag: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && bytes[cursor] != b'='
            && bytes[cursor] != b'>'
            && bytes[cursor] != b'/'
        {
            cursor += 1;
        }
        if name_start == cursor {
            cursor += 1;
            continue;
        }

        let name = &tag[name_start..cursor];
        if name.len() > "data-".len()
            && name[..5].eq_ignore_ascii_case("data-")
            && !name.eq_ignore_ascii_case("data-src")
        {
            let value = html_attr_value(tag, name)?;
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn decode_html_entities(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut index = 0usize;

    while let Some(relative_start) = text[index..].find('&') {
        let start = index + relative_start;
        output.push_str(&text[index..start]);

        let Some(relative_end) = text[start..].find(';') else {
            output.push_str(&text[start..]);
            return output;
        };
        let end = start + relative_end;
        let entity = &text[start + 1..end];

        if entity.len() > 32 {
            output.push_str(&text[start..=end]);
        } else if let Some(decoded) = decode_html_entity(entity) {
            output.push_str(&decoded);
        } else {
            output.push_str(&text[start..=end]);
        }
        index = end + 1;
    }

    output.push_str(&text[index..]);
    output
}

fn decode_html_entity(entity: &str) -> Option<String> {
    let decoded = match entity {
        "nbsp" | "ensp" | "emsp" => " ".to_string(),
        "thinsp" | "zwnj" | "zwj" => String::new(),
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" | "#39" => "'".to_string(),
        "lsquo" => "\u{2018}".to_string(),
        "rsquo" => "\u{2019}".to_string(),
        "ldquo" => "\u{201c}".to_string(),
        "rdquo" => "\u{201d}".to_string(),
        "ndash" => "\u{2013}".to_string(),
        "mdash" => "\u{2014}".to_string(),
        "hellip" => "\u{2026}".to_string(),
        "middot" => "\u{00b7}".to_string(),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            let codepoint = u32::from_str_radix(&entity[2..], 16).ok()?;
            char::from_u32(codepoint)?.to_string()
        }
        _ if entity.starts_with('#') => {
            let codepoint = entity[1..].parse::<u32>().ok()?;
            char::from_u32(codepoint)?.to_string()
        }
        _ => return None,
    };

    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_crlf_and_cr_to_lf() {
        assert_eq!(normalize_line_endings("a\r\nb\rc\n"), "a\nb\nc\n");
    }

    #[test]
    fn collapses_three_plus_newlines_to_two() {
        assert_eq!(collapse_blank_lines("a\n\n\n\nb"), "a\n\nb");
        assert_eq!(collapse_blank_lines("a\n\nb"), "a\n\nb");
        assert_eq!(collapse_blank_lines("a\nb"), "a\nb");
    }

    #[test]
    fn normalize_content_trims_edges_and_collapses_blanks() {
        let raw = "\n\n  para one  \n\n\n\n  para two  \n\n";
        let normalized = normalize_content(raw);
        assert_eq!(normalized, "  para one  \n\n  para two  ");
    }

    #[test]
    fn strip_bom_removes_leading_bom() {
        assert_eq!(strip_bom("\u{FEFF}hello"), "hello");
        assert_eq!(strip_bom("hello"), "hello");
    }

    #[test]
    fn normalize_content_handles_empty_string() {
        assert_eq!(normalize_content(""), "");
    }

    #[test]
    fn extracted_html_fragments_strip_tags_and_decode_entities() {
        let html = "<p>First&nbsp;&amp; <em>bold</em></p><p>Second<br/>line</p>";

        assert_eq!(
            normalize_extracted_content(html),
            "First & bold\nSecond\nline"
        );
    }

    #[test]
    fn extracted_content_removes_legado_no_print_characters() {
        let text = "a\u{2009}b\u{200c}c\u{200d}d";

        assert_eq!(normalize_extracted_content(text), "abcd");
    }

    #[test]
    fn extracted_html_keeps_img_data_attributes_like_legado() {
        let html = r#"<p>Before</p><img data-original="/images/1.jpg"><p>After</p>"#;

        assert_eq!(
            normalize_extracted_content(html),
            "Before\n<img src=\"/images/1.jpg\">\nAfter"
        );
    }

    #[test]
    fn extracted_content_decodes_common_html4_punctuation_entities() {
        let text = "&ldquo;Title&rdquo;&mdash;author";

        assert_eq!(
            normalize_extracted_content(text),
            "\u{201c}Title\u{201d}\u{2014}author"
        );
    }
}

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
}

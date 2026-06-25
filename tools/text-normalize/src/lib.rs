//! Standalone dev-time text normalization for benchmark chapter-content
//! comparison.
//!
//! This crate is intentionally **not** integrated with `reader-content`. It
//! exists as a development-time tool to canonicalize extracted chapter text
//! before hashing/diffing so that formatting noise (CRLF, full-width glyphs,
//! HTML entities, stray whitespace) does not produce false mismatches.
//!
//! See `Options` for the configurable knobs and `Options::conservative` /
//! `Options::lenient` for the two preset modes.

use unicode_normalization::UnicodeNormalization;

/// Normalization mode preset. Maps to an `Options` snapshot via
/// [`Mode::to_options`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Minimal, structure-preserving canonicalization.
    Conservative,
    /// Aggressive canonicalization for fuzzy comparison.
    Lenient,
}

/// Full-width / half-width conversion strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthStrategy {
    /// Leave full-width characters untouched.
    Keep,
    /// Convert full-width ASCII letters / digits / punctuation and the
    /// ideographic space to their half-width equivalents.
    ToHalf,
}

/// Unicode normalization form to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnicodeForm {
    /// No Unicode normalization.
    None,
    /// NFC (canonical composition).
    Nfc,
    /// NFKC (compatibility composition — also folds full-width ASCII).
    Nfkc,
}

/// Configurable normalization pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub strip_bom: bool,
    pub unicode_form: UnicodeForm,
    pub width_strategy: WidthStrategy,
    pub normalize_newlines: bool,
    pub collapse_blank_lines: bool,
    pub collapse_whitespace: bool,
    pub decode_html_entities: bool,
    pub trim: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self::conservative()
    }
}

impl Options {
    /// Conservative preset: NFC, CRLF→LF, blank-line collapse, HTML entity
    /// decode, BOM strip, edge trim (newlines only). Full-width characters and
    /// inline whitespace are preserved.
    pub fn conservative() -> Self {
        Options {
            strip_bom: true,
            unicode_form: UnicodeForm::Nfc,
            width_strategy: WidthStrategy::Keep,
            normalize_newlines: true,
            collapse_blank_lines: true,
            collapse_whitespace: false,
            decode_html_entities: true,
            trim: true,
        }
    }

    /// Lenient preset: NFKC, full-width→half-width, all whitespace runs
    /// (including newlines) collapsed to a single space, HTML entity decode,
    /// BOM strip, full trim. Good for fuzzy equality.
    pub fn lenient() -> Self {
        Options {
            strip_bom: true,
            unicode_form: UnicodeForm::Nfkc,
            width_strategy: WidthStrategy::ToHalf,
            normalize_newlines: true,
            collapse_blank_lines: true,
            collapse_whitespace: true,
            decode_html_entities: true,
            trim: true,
        }
    }
}

impl Mode {
    pub fn to_options(self) -> Options {
        match self {
            Mode::Conservative => Options::conservative(),
            Mode::Lenient => Options::lenient(),
        }
    }
}

/// Normalization result: the canonical text plus a content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    pub text: String,
    /// FNV-1a 64-bit hash of the normalized text, as 16 lowercase hex chars.
    pub hash: String,
}

/// Normalize `text` according to `opts`, returning the canonical text and its
/// FNV-1a 64-bit hash.
///
/// Pipeline order (each step gated by the corresponding `Options` flag):
/// BOM strip → Unicode form → width strategy → HTML entity decode →
/// newline normalization → blank-line collapse → whitespace collapse → trim.
pub fn normalize(text: &str, opts: &Options) -> Normalized {
    let text = if opts.strip_bom {
        strip_bom(text)
    } else {
        text
    };
    let text = apply_unicode_form(text, opts.unicode_form);
    let text = apply_width_strategy(&text, opts.width_strategy);
    let text = if opts.decode_html_entities {
        decode_html_entities(&text)
    } else {
        text
    };
    let text = if opts.normalize_newlines {
        normalize_line_endings(&text)
    } else {
        text
    };
    let text = if opts.collapse_blank_lines {
        collapse_blank_lines(&text)
    } else {
        text
    };
    let text = if opts.collapse_whitespace {
        collapse_whitespace_runs(&text)
    } else {
        text
    };
    let text = if opts.trim {
        trim_edges(&text, opts.collapse_whitespace)
    } else {
        text
    };

    let hash = hash_text(&text);
    Normalized { text, hash }
}

/// Compute the FNV-1a 64-bit hash of `text` as 16 lowercase hex characters.
///
/// FNV-1a is non-cryptographic but deterministic and collision-resistant enough
/// for benchmark text comparison; identical normalized text always yields an
/// identical hash.
pub fn hash_text(text: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

// ---------------------------------------------------------------------------
// Pipeline steps
// ---------------------------------------------------------------------------

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

fn apply_unicode_form(text: &str, form: UnicodeForm) -> String {
    match form {
        UnicodeForm::None => text.to_string(),
        UnicodeForm::Nfc => text.nfc().collect(),
        UnicodeForm::Nfkc => text.nfkc().collect(),
    }
}

fn apply_width_strategy(text: &str, strategy: WidthStrategy) -> String {
    match strategy {
        WidthStrategy::Keep => text.to_string(),
        WidthStrategy::ToHalf => text
            .chars()
            .map(|ch| {
                let code = ch as u32;
                match code {
                    0x3000 => '\u{0020}', // ideographic space -> ASCII space
                    0xFF01..=0xFF5E => char::from_u32(code - 0xFEE0).unwrap_or(ch),
                    _ => ch,
                }
            })
            .collect(),
    }
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Collapse runs of three or more consecutive newlines down to exactly two.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newline_run = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push(ch);
            }
        } else {
            newline_run = 0;
            out.push(ch);
        }
    }
    out
}

/// Collapse every maximal run of whitespace characters to a single ASCII space.
fn collapse_whitespace_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(ch);
            in_run = false;
        }
    }
    out
}

/// Trim leading/trailing whitespace. When `full` is true, all whitespace is
/// trimmed; otherwise only `\n` and `\r` are trimmed (inline spaces on the
/// first/last line are preserved).
fn trim_edges(text: &str, full: bool) -> String {
    if full {
        text.trim().to_string()
    } else {
        text.trim_matches(|c: char| c == '\n' || c == '\r')
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// HTML entity decoding
// ---------------------------------------------------------------------------

fn decode_html_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp_rel) = rest.find('&') {
        out.push_str(&rest[..amp_rel]);
        let after_amp = &rest[amp_rel..];
        match after_amp.find(';') {
            None => {
                out.push_str(after_amp);
                return out;
            }
            Some(semi_rel) => {
                let entity = &after_amp[1..semi_rel];
                if entity.len() > 32 {
                    out.push_str(&after_amp[..=semi_rel]);
                } else if let Some(decoded) = decode_entity(entity) {
                    out.push_str(&decoded);
                } else {
                    out.push_str(&after_amp[..=semi_rel]);
                }
                rest = &after_amp[semi_rel + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<String> {
    let decoded = match entity {
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ".to_string(),
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
    fn fnv1a_known_vector() {
        // FNV-1a 64-bit of "foobar" (reference vector from the FNV spec).
        assert_eq!(hash_text("foobar"), "85944171f73967e8");
    }

    #[test]
    fn fullwidth_range_boundaries() {
        assert_eq!(apply_width_strategy("！", WidthStrategy::ToHalf), "!");
        assert_eq!(apply_width_strategy("～", WidthStrategy::ToHalf), "~");
        assert_eq!(apply_width_strategy("　", WidthStrategy::ToHalf), " ");
    }
}

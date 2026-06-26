//! Integration tests for the standalone text-normalization tool.
//!
//! Covers the required surface area: Chinese punctuation / full-width glyphs,
//! whitespace handling, HTML entity decoding, CRLF/LF normalization,
//! conservative vs lenient modes, Unicode normalization, and hash determinism.

use reader_text_normalize::{hash_text, normalize, Mode, Options, UnicodeForm, WidthStrategy};

// ---------------------------------------------------------------------------
// CRLF / LF normalization
// ---------------------------------------------------------------------------

#[test]
fn conservative_normalizes_crlf_and_cr_to_lf() {
    let out = normalize("a\r\nb\rc", &Options::conservative());
    assert_eq!(out.text, "a\nb\nc");
}

#[test]
fn conservative_crlf_and_lf_produce_identical_output() {
    let crlf = normalize("line1\r\nline2\r\nline3", &Options::conservative());
    let lf = normalize("line1\nline2\nline3", &Options::conservative());
    assert_eq!(crlf.text, lf.text);
    assert_eq!(crlf.hash, lf.hash);
}

#[test]
fn lenient_collapses_all_newlines_to_single_spaces() {
    let out = normalize("a\r\nb\rc\nd", &Options::lenient());
    assert_eq!(out.text, "a b c d");
}

// ---------------------------------------------------------------------------
// HTML entity decoding
// ---------------------------------------------------------------------------

#[test]
fn conservative_decodes_common_named_entities() {
    let out = normalize("a&nbsp;b&amp;c&lt;d&gt;e&quot;f", &Options::conservative());
    assert_eq!(out.text, "a b&c<d>e\"f");
}

#[test]
fn decodes_decimal_numeric_entities() {
    let out = normalize("&#65;&#66;&#67;", &Options::conservative());
    assert_eq!(out.text, "ABC");
}

#[test]
fn decodes_hex_numeric_entities_case_insensitive() {
    let out = normalize("&#x41;&#X42;", &Options::conservative());
    assert_eq!(out.text, "AB");
}

#[test]
fn decodes_curly_quote_entities_used_in_chinese_typography() {
    let out = normalize("&ldquo;引文&rdquo;&mdash;作者", &Options::conservative());
    assert_eq!(out.text, "\u{201c}引文\u{201d}\u{2014}作者");
}

#[test]
fn leaves_unknown_entities_intact() {
    let out = normalize("a&notarealentity;b", &Options::conservative());
    assert_eq!(out.text, "a&notarealentity;b");
}

#[test]
fn lenient_also_decodes_entities_before_whitespace_collapse() {
    // &nbsp; decodes to a space, then lenient collapses the run.
    let out = normalize("a&nbsp;&nbsp;b", &Options::lenient());
    assert_eq!(out.text, "a b");
}

// ---------------------------------------------------------------------------
// Chinese punctuation / full-width vs half-width
// ---------------------------------------------------------------------------

#[test]
fn conservative_keeps_fullwidth_ascii_letters_and_digits() {
    let out = normalize("ＡＢＣ１２３", &Options::conservative());
    assert_eq!(out.text, "ＡＢＣ１２３");
}

#[test]
fn lenient_converts_fullwidth_ascii_letters_and_digits_to_halfwidth() {
    let out = normalize("ＡＢＣ１２３", &Options::lenient());
    assert_eq!(out.text, "ABC123");
}

#[test]
fn lenient_converts_fullwidth_space_to_halfwidth() {
    let out = normalize("a　b", &Options::lenient());
    assert_eq!(out.text, "a b");
}

#[test]
fn width_to_half_converts_fullwidth_ascii_punctuation() {
    let opts = Options {
        width_strategy: WidthStrategy::ToHalf,
        ..Options::conservative()
    };
    let out = normalize("！＃＄％＆", &opts);
    assert_eq!(out.text, "!#$%&");
}

#[test]
fn width_keep_preserves_fullwidth_punctuation() {
    let opts = Options {
        width_strategy: WidthStrategy::Keep,
        unicode_form: UnicodeForm::None,
        ..Options::conservative()
    };
    let out = normalize("！＃", &opts);
    assert_eq!(out.text, "！＃");
}

#[test]
fn lenient_nfkc_folds_ligatures() {
    // NFKC decomposes the fi ligature (U+FB01) to "fi".
    let out = normalize("oﬃce", &Options::lenient());
    assert_eq!(out.text, "office");
}

// ---------------------------------------------------------------------------
// Whitespace handling
// ---------------------------------------------------------------------------

#[test]
fn conservative_preserves_inline_whitespace_runs() {
    let out = normalize("hello   world", &Options::conservative());
    assert_eq!(out.text, "hello   world");
}

#[test]
fn lenient_collapses_inline_whitespace_runs_to_single_space() {
    let out = normalize("hello \t \t world", &Options::lenient());
    assert_eq!(out.text, "hello world");
}

#[test]
fn conservative_collapses_three_plus_blank_lines_to_two() {
    let out = normalize("a\n\n\n\n\nb", &Options::conservative());
    assert_eq!(out.text, "a\n\nb");
}

#[test]
fn conservative_keeps_single_and_double_newlines() {
    assert_eq!(normalize("a\nb", &Options::conservative()).text, "a\nb");
    assert_eq!(normalize("a\n\nb", &Options::conservative()).text, "a\n\nb");
}

#[test]
fn conservative_trims_leading_and_trailing_newlines_only() {
    // trim removes edges; inline spaces within the first/last line are kept.
    let out = normalize("\n\n  hello  \n\n", &Options::conservative());
    assert_eq!(out.text, "  hello  ");
}

#[test]
fn lenient_trims_all_edges() {
    let out = normalize("   hello world   ", &Options::lenient());
    assert_eq!(out.text, "hello world");
}

// ---------------------------------------------------------------------------
// BOM stripping
// ---------------------------------------------------------------------------

#[test]
fn conservative_strips_leading_bom() {
    let out = normalize("\u{FEFF}hello", &Options::conservative());
    assert_eq!(out.text, "hello");
}

#[test]
fn lenient_strips_leading_bom() {
    let out = normalize("\u{FEFF}hello world", &Options::lenient());
    assert_eq!(out.text, "hello world");
}

// ---------------------------------------------------------------------------
// Unicode normalization
// ---------------------------------------------------------------------------

#[test]
fn conservative_applies_nfc_to_combining_diacritics() {
    // NFD: 'e' + combining acute (U+0301) -> NFC: é (U+00E9)
    let nfd = "e\u{0301}";
    let out = normalize(nfd, &Options::conservative());
    assert_eq!(out.text, "é");
    assert_eq!(out.text.chars().count(), 1);
}

#[test]
fn no_unicode_form_leaves_combining_marks_alone() {
    let opts = Options {
        unicode_form: UnicodeForm::None,
        ..Options::conservative()
    };
    let nfd = "e\u{0301}";
    let out = normalize(nfd, &opts);
    assert_eq!(out.text, "e\u{0301}");
}

// ---------------------------------------------------------------------------
// Hash
// ---------------------------------------------------------------------------

#[test]
fn hash_is_deterministic_for_identical_input() {
    let a = normalize("chapter text", &Options::conservative());
    let b = normalize("chapter text", &Options::conservative());
    assert_eq!(a.hash, b.hash);
}

#[test]
fn hash_differs_for_different_text() {
    let a = normalize("hello", &Options::conservative());
    let b = normalize("world", &Options::conservative());
    assert_ne!(a.hash, b.hash);
}

#[test]
fn hash_is_16_lowercase_hex_chars() {
    let out = normalize("anything", &Options::conservative());
    assert_eq!(out.hash.len(), 16);
    assert!(out
        .hash
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn hash_matches_direct_hash_text_call() {
    let out = normalize("payload", &Options::conservative());
    assert_eq!(out.hash, hash_text(&out.text));
}

#[test]
fn fnv1a_empty_string_hash_is_offset_basis() {
    // FNV-1a 64-bit of the empty string is the offset basis.
    assert_eq!(hash_text(""), "cbf29ce484222325");
}

#[test]
fn same_normalized_text_yields_same_hash_across_modes() {
    // "hello" is untouched by both modes, so hashes must match.
    let c = normalize("hello", &Options::conservative());
    let l = normalize("hello", &Options::lenient());
    assert_eq!(c.text, l.text);
    assert_eq!(c.hash, l.hash);
}

// ---------------------------------------------------------------------------
// Mode preset mapping
// ---------------------------------------------------------------------------

#[test]
fn mode_to_options_matches_presets() {
    assert_eq!(Mode::Conservative.to_options(), Options::conservative());
    assert_eq!(Mode::Lenient.to_options(), Options::lenient());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_string_normalizes_to_empty() {
    let out = normalize("", &Options::conservative());
    assert_eq!(out.text, "");
    assert_eq!(out.hash, hash_text(""));
}

#[test]
fn whitespace_only_conservative_collapses_to_empty() {
    let out = normalize("\n\n\n", &Options::conservative());
    assert_eq!(out.text, "");
}

#[test]
fn whitespace_only_lenient_collapses_to_empty() {
    let out = normalize("  \n\t \r\n", &Options::lenient());
    assert_eq!(out.text, "");
}

#[test]
fn combined_pipeline_chinese_text_with_noise() {
    // A realistic chapter snippet with full-width chars, entity, CRLF, extra
    // blank lines — canonicalized by conservative mode.
    let raw = "\u{FEFF}第一回\u{3000}ＡＢＣ\r\n\r\n\r\n第二段&nbsp;内容";
    let out = normalize(raw, &Options::conservative());
    assert_eq!(out.text, "第一回\u{3000}ＡＢＣ\n\n第二段 内容");
}

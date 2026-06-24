//! Reader-Core local-book parsing — TXT / EPUB / encoding detection.
//!
//! This crate owns the local-content parsing line: it turns raw decoded text
//! into structured data consumable by the existing domain model
//! ([`Book`](reader_domain::Book), [`TocEntry`](reader_domain::TocEntry), and
//! chapter body `String`s).
//!
//! ## V1 scope
//!
//! TXT parsing is implemented in [`txt`] with std-only pattern matching (no
//! regex dependency). It covers:
//!
//! - Book metadata extraction (title from the first non-heading line).
//! - Chapter title identification (Chinese `第N章/节/卷/回/部/篇`, English
//!   `Chapter N`, and special headings like `楔子` / `序章` / `番外`).
//! - Chapter body reading with normalized line endings.
//! - Edge cases: empty files, no-heading files, duplicate headings, abnormal
//!   line breaks (CRLF / lone CR / mixed), and UTF-8 BOM.
//!
//! ## Known gaps (not addressed in V1)
//!
//! The following would require new crate dependencies and are recorded here
//! rather than silently introduced:
//!
//! - **Encoding detection** — GBK / GB18030 / Big5 → UTF-8 transcoding. A crate
//!   such as `encoding_rs` or `chardetng` is needed; callers must currently
//!   supply already-decoded `&str`.
//! - **Regex-based heading rules** — the current detector uses hand-written
//!   pattern matching. User-configurable regex heading rules would need the
//!   `regex` crate.
//! - **EPUB** — ZIP unpacking + OPF/NCX parsing (`zip` + XML crates).

pub mod txt;

pub use txt::{parse_txt, parse_txt_with_options, ParsedTxt, TxtChapter, TxtParseOptions};

/// Errors raised by local-book parsing.
///
/// TXT parsing is infallible (any `&str` is valid TXT), so this enum is
/// reserved for future formats (EPUB, encoding) and is intentionally empty in
/// V1. It is defined so callers can depend on a stable error type ahead of
/// those additions.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalBookError {
    /// The input was empty or could not be decoded.
    Empty,
    /// A future format (EPUB, etc.) is not yet supported.
    Unsupported { format: String },
    /// A structural parse failure in a future format.
    Parse { message: String },
}

impl std::fmt::Display for LocalBookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalBookError::Empty => write!(f, "empty local book input"),
            LocalBookError::Unsupported { format } => {
                write!(f, "unsupported local book format: {format}")
            }
            LocalBookError::Parse { message } => {
                write!(f, "local book parse error: {message}")
            }
        }
    }
}

impl std::error::Error for LocalBookError {}

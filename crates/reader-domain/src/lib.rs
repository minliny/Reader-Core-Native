//! Reader-Core domain models — Book / Chapter / Source / Progress.
//!
//! Minimal V1 models for the remote-reading vertical. These are intentionally
//! small: they carry just enough structure for the import → search → detail →
//! toc → chapter → progress pipeline to round-trip through the JSON protocol.
//! Legado parity is explicitly out of scope for V1.

use serde::{Deserialize, Serialize};

/// A remote book source definition (the "import source" payload).
///
/// `rules` describe how to extract books from a source's HTML/JSON responses.
/// They are stored verbatim as JSON so the rule engine can deserialize them
/// into [`RuleStep`](../reader_rule/enum.RuleStep.html) instances on demand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    /// Stable identifier assigned at import time.
    pub source_id: String,
    /// Human-readable source name.
    pub name: String,
    /// Base URL of the source (informational only in V1 — no live network).
    #[serde(default)]
    pub base_url: String,
    /// Extraction rules keyed by pipeline stage. Each value is a JSON array of
    /// rule steps understood by `reader-rule`.
    pub rules: SourceRules,
}

/// Per-stage extraction rules for a [`Source`].
///
/// Each field is a JSON array of `RuleStep` objects. Empty arrays are allowed
/// (the stage simply yields nothing).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRules {
    /// Extract a list of books from a search response.
    #[serde(default)]
    pub search: serde_json::Value,
    /// Extract detail metadata for a single book.
    #[serde(default)]
    pub detail: serde_json::Value,
    /// Extract the table of contents (chapter list).
    #[serde(default)]
    pub toc: serde_json::Value,
    /// Extract the chapter body text.
    #[serde(default)]
    pub chapter: serde_json::Value,
}

/// A book discovered via search or detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Book {
    /// Source-relative book identifier (may be a URL or path fragment).
    #[serde(default)]
    pub book_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    /// Optional cover URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Optional intro/summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    /// Optional category/kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional last-chapter hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
}

/// A single table-of-contents entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TocEntry {
    /// 0-based index within the toc.
    pub index: u32,
    #[serde(default)]
    pub title: String,
    /// Source-relative chapter URL/path.
    #[serde(default)]
    pub url: String,
}

/// Minimal reading progress / state for a book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingProgress {
    pub book_id: String,
    /// Index of the chapter the reader is currently on.
    #[serde(default)]
    pub chapter_index: u32,
    /// Scroll/char offset within the current chapter (0-based).
    #[serde(default)]
    pub chapter_offset: u64,
    /// Fraction read in the current chapter, 0.0..=1.0.
    #[serde(default)]
    pub chapter_progress: f64,
}

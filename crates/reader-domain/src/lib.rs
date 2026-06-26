//! Reader-Core domain models — Book / Chapter / Source / Progress.
//!
//! Minimal V1 models for the remote-reading vertical. These are intentionally
//! small: they carry just enough structure for the import → search → detail →
//! toc → chapter → progress pipeline to round-trip through the JSON protocol.
//! Legado BookSource compatibility lives beside the V1 execution model: raw
//! Legado rule strings are preserved first, then mapped into execution rules by
//! later migration stages.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
    /// Optional raw Legado BookSource payload. This is preserved independently
    /// from V1 `rules` so DSL migration can be staged without data loss.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub book_source: Value,
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
    pub search: Value,
    /// Extract a discovery/explore page.
    #[serde(default)]
    pub explore: Value,
    /// Extract detail metadata for a single book.
    #[serde(default)]
    pub detail: Value,
    /// Extract the table of contents (chapter list).
    #[serde(default)]
    pub toc: Value,
    /// Extract the chapter body text.
    #[serde(default)]
    pub chapter: Value,
}

/// Compatibility model for a Legado BookSource JSON document.
///
/// This intentionally mirrors the legacy field names and keeps unknown fields.
/// It is not the execution model. Its first responsibility is lossless import
/// and export of BookSource metadata and raw rule DSL strings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegadoBookSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_type: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_url_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_order: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js_lib: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js_lib_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrent_rate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_review: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_check_js: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_js: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_button: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_listener: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_explore: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_explore_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_rule: Option<ExploreRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_review: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_review_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_rule: Option<ReviewRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_rule: Option<SearchRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_book_info: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_info_rule: Option<BookInfoRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_toc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_rule: Option<TocRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rule: Option<ContentRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_explore: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_rule: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_ui: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_cookie_jar: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_decode_js: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_view: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<bool>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

pub type BookSourceCompat = LegadoBookSource;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceSemantics {
    pub source_id: String,
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_url: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub enabled_explore: bool,
    pub rules: BookSourcePipelineRules,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourcePipelineRules {
    pub search: BookSourceSearchSemantics,
    pub explore: BookSourceExploreSemantics,
    pub detail: BookSourceDetailSemantics,
    pub toc: BookSourceTocSemantics,
    pub content: BookSourceContentSemantics,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceSearchSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceExploreSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceDetailSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceTocSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceContentSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_regex: Option<String>,
}

impl Source {
    pub fn legado_book_source(&self) -> Option<LegadoBookSource> {
        if self.book_source.is_null() {
            return None;
        }
        serde_json::from_value(self.book_source.clone()).ok()
    }

    pub fn book_source_semantics(&self) -> Option<BookSourceSemantics> {
        self.legado_book_source().map(|book_source| {
            BookSourceSemantics::from_legado(
                &self.source_id,
                Some(&self.name),
                Some(&self.base_url),
                &book_source,
            )
        })
    }
}

impl BookSourceSemantics {
    pub fn from_legado(
        source_id: &str,
        fallback_name: Option<&str>,
        fallback_base_url: Option<&str>,
        source: &LegadoBookSource,
    ) -> Self {
        let search = normalize_search_semantics(source);
        let explore = normalize_explore_semantics(source);
        let detail = normalize_detail_semantics(source);
        let toc = normalize_toc_semantics(source);
        let content = normalize_content_semantics(source);
        Self {
            source_id: first_non_empty_str(&[Some(source_id)])
                .unwrap_or_else(|| "booksource".to_string()),
            name: first_non_empty_str(&[source.book_source_name.as_deref(), fallback_name])
                .unwrap_or_default(),
            base_url: first_non_empty_str(&[source.book_source_url.as_deref(), fallback_base_url])
                .unwrap_or_default(),
            search_url: clean_string(source.search_url.as_deref()),
            explore_url: clean_string(source.explore_url.as_deref()),
            enabled: source.enabled.unwrap_or(true),
            enabled_explore: source.enabled_explore.unwrap_or(false),
            rules: BookSourcePipelineRules {
                search,
                explore,
                detail,
                toc,
                content,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_key_word: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_fields: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookInfoRule {
    #[serde(default, rename = "init", skip_serializing_if = "Option::is_none")]
    pub r#init: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_re_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_urls: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_volume: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_vip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_pay: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_toc_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_update_js: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_js: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_content_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_js: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_decode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pay_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_check_js: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_screen: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_rating: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
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

fn normalize_search_semantics(source: &LegadoBookSource) -> BookSourceSearchSemantics {
    let structured = source.search_rule.as_ref();
    let legacy_fields = [
        source.rule_search_name.as_deref(),
        source.rule_search_author.as_deref(),
        source.rule_search_url.as_deref(),
    ]
    .iter()
    .any(|value| clean_string(*value).is_some());
    BookSourceSearchSemantics {
        raw: clean_string(source.rule_search.as_deref()),
        list: structured
            .and_then(|rule| clean_string(rule.book_list.as_deref()))
            .or_else(|| {
                if legacy_fields {
                    clean_string(source.rule_search.as_deref())
                } else {
                    None
                }
            }),
        name: structured
            .and_then(|rule| clean_string(rule.name.as_deref()))
            .or_else(|| clean_string(source.rule_search_name.as_deref())),
        author: structured
            .and_then(|rule| clean_string(rule.author.as_deref()))
            .or_else(|| clean_string(source.rule_search_author.as_deref())),
        intro: structured.and_then(|rule| clean_string(rule.intro.as_deref())),
        kind: structured.and_then(|rule| clean_string(rule.kind.as_deref())),
        last_chapter: structured.and_then(|rule| clean_string(rule.last_chapter.as_deref())),
        update_time: structured.and_then(|rule| clean_string(rule.update_time.as_deref())),
        detail_url: structured
            .and_then(|rule| clean_string(rule.book_url.as_deref()))
            .or_else(|| clean_string(source.rule_search_url.as_deref())),
        cover_url: structured.and_then(|rule| clean_string(rule.cover_url.as_deref())),
        word_count: structured.and_then(|rule| clean_string(rule.word_count.as_deref())),
    }
}

fn normalize_explore_semantics(source: &LegadoBookSource) -> BookSourceExploreSemantics {
    let structured = source.explore_rule.as_ref();
    BookSourceExploreSemantics {
        raw: clean_string(source.rule_explore.as_deref()),
        list: structured
            .and_then(|rule| clean_string(rule.book_list.as_deref()))
            .or_else(|| clean_string(source.rule_explore.as_deref())),
        name: structured.and_then(|rule| clean_string(rule.name.as_deref())),
        author: structured.and_then(|rule| clean_string(rule.author.as_deref())),
        intro: structured.and_then(|rule| clean_string(rule.intro.as_deref())),
        kind: structured.and_then(|rule| clean_string(rule.kind.as_deref())),
        last_chapter: structured.and_then(|rule| clean_string(rule.last_chapter.as_deref())),
        detail_url: structured.and_then(|rule| clean_string(rule.book_url.as_deref())),
        cover_url: structured.and_then(|rule| clean_string(rule.cover_url.as_deref())),
        word_count: structured.and_then(|rule| clean_string(rule.word_count.as_deref())),
        screen: structured.and_then(|rule| clean_string(rule.explore_screen.as_deref())),
    }
}

fn normalize_detail_semantics(source: &LegadoBookSource) -> BookSourceDetailSemantics {
    let structured = source.book_info_rule.as_ref();
    BookSourceDetailSemantics {
        raw: clean_string(source.rule_book_info.as_deref()),
        init: structured
            .and_then(|rule| clean_string(rule.r#init.as_deref()))
            .or_else(|| clean_string(source.rule_book_info.as_deref())),
        name: structured.and_then(|rule| clean_string(rule.name.as_deref())),
        author: structured.and_then(|rule| clean_string(rule.author.as_deref())),
        intro: structured.and_then(|rule| clean_string(rule.intro.as_deref())),
        kind: structured.and_then(|rule| clean_string(rule.kind.as_deref())),
        last_chapter: structured.and_then(|rule| clean_string(rule.last_chapter.as_deref())),
        update_time: structured.and_then(|rule| clean_string(rule.update_time.as_deref())),
        cover_url: structured.and_then(|rule| clean_string(rule.cover_url.as_deref())),
        toc_url: structured.and_then(|rule| clean_string(rule.toc_url.as_deref())),
        word_count: structured.and_then(|rule| clean_string(rule.word_count.as_deref())),
    }
}

fn normalize_toc_semantics(source: &LegadoBookSource) -> BookSourceTocSemantics {
    let structured = source.toc_rule.as_ref();
    let structured_fields = structured.is_some_and(|rule| {
        [
            rule.chapter_name.as_deref(),
            rule.chapter_url.as_deref(),
            rule.next_toc_url.as_deref(),
        ]
        .iter()
        .any(|value| clean_string(*value).is_some())
    });
    BookSourceTocSemantics {
        raw: clean_string(source.rule_toc.as_deref()),
        list: structured
            .and_then(|rule| clean_string(rule.chapter_list.as_deref()))
            .or_else(|| {
                if structured_fields {
                    clean_string(source.rule_toc.as_deref())
                } else {
                    None
                }
            }),
        name: structured.and_then(|rule| clean_string(rule.chapter_name.as_deref())),
        url: structured.and_then(|rule| clean_string(rule.chapter_url.as_deref())),
        next_url: structured.and_then(|rule| clean_string(rule.next_toc_url.as_deref())),
    }
}

fn normalize_content_semantics(source: &LegadoBookSource) -> BookSourceContentSemantics {
    let structured = source.content_rule.as_ref();
    BookSourceContentSemantics {
        raw: clean_string(source.rule_content.as_deref()),
        content: structured
            .and_then(|rule| clean_string(rule.content.as_deref()))
            .or_else(|| clean_string(source.rule_content.as_deref())),
        title: structured.and_then(|rule| clean_string(rule.title.as_deref())),
        next_url: structured.and_then(|rule| clean_string(rule.next_content_url.as_deref())),
        source_regex: structured.and_then(|rule| clean_string(rule.source_regex.as_deref())),
        replace_regex: structured.and_then(|rule| clean_string(rule.replace_regex.as_deref())),
    }
}

fn first_non_empty_str(values: &[Option<&str>]) -> Option<String> {
    values.iter().find_map(|value| clean_string(*value))
}

fn clean_string(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGADO_BOOK_SOURCE: &str = r#"
{
  "bookSourceName": "Legado Compat Source",
  "bookSourceUrl": "https://books.example.test",
  "bookSourceGroup": "compat",
  "searchUrl": "/search?q={{key}}",
  "ruleSearch": "div.list&&div.item;div.name&&a@text",
  "ruleBookInfo": "div.detail",
  "ruleToc": "div.chapter&&a@href",
  "ruleContent": "div.content@html",
  "searchRule": {
    "bookList": "div.list&&div.item",
    "name": "div.name&&a@text",
    "author": "span.author@text",
    "bookUrl": "div.name&&a@href"
  },
  "bookInfoRule": {
    "init": "@js:java.ajax(source.bookSourceUrl)",
    "name": "h1@text",
    "tocUrl": "a.toc@href"
  },
  "tocRule": {
    "chapterList": "div.chapter&&a",
    "chapterName": "a@text",
    "chapterUrl": "a@href"
  },
  "contentRule": {
    "content": "div.content@html",
    "nextContentUrl": "a.next@href",
    "loginCheckJs": "result.includes('login')"
  },
  "enabled": true,
  "enabledExplore": false,
  "enabledCookieJar": true,
  "header": {
    "User-Agent": "ReaderCoreTest"
  },
  "futureLegadoField": {
    "nested": true
  }
}
"#;

    #[test]
    fn legado_book_source_decodes_legacy_fields_and_raw_rules() {
        let source: LegadoBookSource =
            serde_json::from_str(LEGADO_BOOK_SOURCE).expect("fixture should decode");

        assert_eq!(
            source.book_source_name.as_deref(),
            Some("Legado Compat Source")
        );
        assert_eq!(
            source.rule_search.as_deref(),
            Some("div.list&&div.item;div.name&&a@text")
        );
        assert_eq!(
            source
                .search_rule
                .as_ref()
                .and_then(|rule| rule.book_list.as_deref()),
            Some("div.list&&div.item")
        );
        assert_eq!(
            source
                .book_info_rule
                .as_ref()
                .and_then(|rule| rule.r#init.as_deref()),
            Some("@js:java.ajax(source.bookSourceUrl)")
        );
        assert_eq!(
            source
                .content_rule
                .as_ref()
                .and_then(|rule| rule.login_check_js.as_deref()),
            Some("result.includes('login')")
        );
        assert_eq!(source.enabled_cookie_jar, Some(true));
        assert_eq!(
            source
                .header
                .as_ref()
                .and_then(|header| header["User-Agent"].as_str()),
            Some("ReaderCoreTest")
        );
        assert_eq!(
            source.extra["futureLegadoField"],
            serde_json::json!({ "nested": true })
        );
    }

    #[test]
    fn legado_book_source_round_trips_without_rewriting_rule_dsl() {
        let original: Value =
            serde_json::from_str(LEGADO_BOOK_SOURCE).expect("fixture should be json");
        let source: BookSourceCompat =
            serde_json::from_value(original.clone()).expect("fixture should decode");
        let encoded = serde_json::to_value(source).expect("source should encode");

        for field in [
            "ruleSearch",
            "ruleBookInfo",
            "ruleToc",
            "ruleContent",
            "searchRule",
            "bookInfoRule",
            "tocRule",
            "contentRule",
            "header",
            "futureLegadoField",
        ] {
            assert_eq!(
                encoded[field], original[field],
                "{field} should be preserved"
            );
        }

        assert!(
            encoded.get("baseUrl").is_none(),
            "compat encoding must not invent V1 fields"
        );
    }

    #[test]
    fn legado_book_source_normalizes_structured_and_legacy_rule_aliases() {
        let source: LegadoBookSource =
            serde_json::from_str(LEGADO_BOOK_SOURCE).expect("fixture should decode");
        let semantics = BookSourceSemantics::from_legado(
            "legado-compat-source",
            Some("Fallback Name"),
            Some("https://fallback.example.test"),
            &source,
        );

        assert_eq!(semantics.source_id, "legado-compat-source");
        assert_eq!(semantics.name, "Legado Compat Source");
        assert_eq!(semantics.base_url, "https://books.example.test");
        assert_eq!(semantics.search_url.as_deref(), Some("/search?q={{key}}"));
        assert_eq!(
            semantics.rules.search.raw.as_deref(),
            Some("div.list&&div.item;div.name&&a@text")
        );
        assert_eq!(
            semantics.rules.search.list.as_deref(),
            Some("div.list&&div.item")
        );
        assert_eq!(
            semantics.rules.search.name.as_deref(),
            Some("div.name&&a@text")
        );
        assert_eq!(
            semantics.rules.search.detail_url.as_deref(),
            Some("div.name&&a@href")
        );
        assert_eq!(
            semantics.rules.detail.init.as_deref(),
            Some("@js:java.ajax(source.bookSourceUrl)")
        );
        assert_eq!(
            semantics.rules.detail.toc_url.as_deref(),
            Some("a.toc@href")
        );
        assert_eq!(semantics.rules.toc.list.as_deref(), Some("div.chapter&&a"));
        assert_eq!(semantics.rules.toc.url.as_deref(), Some("a@href"));
        assert_eq!(
            semantics.rules.content.content.as_deref(),
            Some("div.content@html")
        );
        assert_eq!(
            semantics.rules.content.next_url.as_deref(),
            Some("a.next@href")
        );
    }
}

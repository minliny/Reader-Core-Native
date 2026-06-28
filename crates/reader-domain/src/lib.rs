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
    pub concurrent_rate: Option<Value>,
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
    // rule_explore/rule_review 接受两种形态(与 rule_search 同模式):
    // - 字符串(legacy/混合格式)
    // - 对象(真实 Legado 导出格式,见 BookSource.kt ruleExplore: ExploreRule)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_explore: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_explore_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_rule: Option<ExploreRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_review: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_review_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_rule: Option<ReviewRule>,
    // rule_search/rule_book_info/rule_toc/rule_content 接受两种形态:
    // - 字符串(legacy/混合格式,如 LEGADO_BOOK_SOURCE fixture)
    // - 对象(真实 Legado 导出格式,见 BookSource.kt ruleSearch: SearchRule)
    // 用 Option<Value> 天然兼容两种,后续在 normalize_*_semantics 展开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_rule: Option<SearchRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_search_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_book_info: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_info_rule: Option<BookInfoRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_toc: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_rule: Option<TocRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<Value>,
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
        serde_json::from_value(self.book_source.clone())
            .map_err(|err| {
                // 不再静默吞错误:记录诊断到 stderr,让 release blocker 可见。
                // reader-domain 是纯 domain crate,无 tracing/log 依赖,用 eprintln!。
                eprintln!(
                    "[reader-domain] legado book source deserialization failed: source_id={} error={}",
                    self.source_id, err
                );
                err
            })
            .ok()
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
    // structured 优先用 search_rule 字段(canonical fixture 格式),
    // 其次用 rule_search 对象形态(真实 Legado 导出格式)。
    let structured_owned = source
        .search_rule
        .clone()
        .or_else(|| rule_value_as_structured::<SearchRule>(&source.rule_search));
    let structured = structured_owned.as_ref();
    let legacy_fields = [
        source.rule_search_name.as_deref(),
        source.rule_search_author.as_deref(),
        source.rule_search_url.as_deref(),
    ]
    .iter()
    .any(|value| clean_string(*value).is_some());
    let raw_str = rule_value_as_string(&source.rule_search);
    BookSourceSearchSemantics {
        raw: clean_string(raw_str),
        list: structured
            .and_then(|rule| clean_string(rule.book_list.as_deref()))
            .or_else(|| {
                if legacy_fields {
                    clean_string(raw_str)
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
    let structured_owned = source
        .explore_rule
        .clone()
        .or_else(|| rule_value_as_structured::<ExploreRule>(&source.rule_explore));
    let structured = structured_owned.as_ref();
    let raw_str = rule_value_as_string(&source.rule_explore);
    BookSourceExploreSemantics {
        raw: clean_string(raw_str),
        list: structured
            .and_then(|rule| clean_string(rule.book_list.as_deref()))
            .or_else(|| clean_string(raw_str)),
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
    let structured_owned = source
        .book_info_rule
        .clone()
        .or_else(|| rule_value_as_structured::<BookInfoRule>(&source.rule_book_info));
    let structured = structured_owned.as_ref();
    let raw_str = rule_value_as_string(&source.rule_book_info);
    BookSourceDetailSemantics {
        raw: clean_string(raw_str),
        init: structured
            .and_then(|rule| clean_string(rule.r#init.as_deref()))
            .or_else(|| clean_string(raw_str)),
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
    let structured_owned = source
        .toc_rule
        .clone()
        .or_else(|| rule_value_as_structured::<TocRule>(&source.rule_toc));
    let structured = structured_owned.as_ref();
    let structured_fields = structured.is_some_and(|rule| {
        [
            rule.chapter_name.as_deref(),
            rule.chapter_url.as_deref(),
            rule.next_toc_url.as_deref(),
        ]
        .iter()
        .any(|value| clean_string(*value).is_some())
    });
    let raw_str = rule_value_as_string(&source.rule_toc);
    BookSourceTocSemantics {
        raw: clean_string(raw_str),
        list: structured
            .and_then(|rule| clean_string(rule.chapter_list.as_deref()))
            .or_else(|| {
                if structured_fields {
                    clean_string(raw_str)
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
    let structured_owned = source
        .content_rule
        .clone()
        .or_else(|| rule_value_as_structured::<ContentRule>(&source.rule_content));
    let structured = structured_owned.as_ref();
    let raw_str = rule_value_as_string(&source.rule_content);
    BookSourceContentSemantics {
        raw: clean_string(raw_str),
        content: structured
            .and_then(|rule| clean_string(rule.content.as_deref()))
            .or_else(|| clean_string(raw_str)),
        title: structured.and_then(|rule| clean_string(rule.title.as_deref())),
        next_url: structured.and_then(|rule| clean_string(rule.next_content_url.as_deref())),
        source_regex: structured.and_then(|rule| clean_string(rule.source_regex.as_deref())),
        replace_regex: structured.and_then(|rule| clean_string(rule.replace_regex.as_deref())),
    }
}

fn first_non_empty_str(values: &[Option<&str>]) -> Option<String> {
    values.iter().find_map(|value| clean_string(*value))
}

/// 从 `Option<Value>` 中取字符串形态(legacy/混合格式 fixture)。
/// 对象形态返回 None,调用方应改用 `rule_value_as_structured`。
fn rule_value_as_string(v: &Option<Value>) -> Option<&str> {
    v.as_ref().and_then(|val| val.as_str())
}

/// 从 `Option<Value>` 中取对象形态,反序列化为对应 Rule struct
/// (真实 Legado 导出格式,见 BookSource.kt ruleSearch: SearchRule)。
/// 字符串形态或解析失败返回 None。
fn rule_value_as_structured<T: serde::de::DeserializeOwned>(v: &Option<Value>) -> Option<T> {
    v.as_ref()
        .and_then(|val| {
            if val.is_object() {
                Some(val.clone())
            } else {
                None
            }
        })
        .and_then(|val| serde_json::from_value(val).ok())
}

fn clean_string(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

// ===========================================================================
// Independent configurable entities (S5 closure)
//
// Aligned against Legado `app/src/main/java/io/legado/app/data/entities/`
// {ReplaceRule,DictRule,TxtTocRule,Bookmark}.kt and Swift `Reader-Core`:
// DictRule.swift + ReplaceRuleEngine.swift (ReaderCoreManagedReplaceRule + scope).
// Per charter red line 3: Swift Core 也缺的能力（TxtTocRule/Bookmark）对照 Legado
// 新建；Swift 已有的（DictRule/ReplaceRule）迁移保真 + 补齐到 Legado 字段。
// ===========================================================================

fn default_true() -> bool {
    true
}

fn default_serial_number() -> i32 {
    -1
}

fn default_timeout_ms() -> i64 {
    3000
}

/// TXT 文件目录识别规则（对照 Legado `TxtTocRule.kt`）。
///
/// Swift Reader-Core 不存在该实体，对照 Legado 新建。`rule` 是正则字符串，
/// 用于在本地 TXT 全文上匹配章节标题（Legado `TextFile.kt:440-461` 的择优算法
/// 在 reader-local-book 消费侧实现）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TxtTocRule {
    /// 主键；Legado 默认 `System.currentTimeMillis()`，Rust 侧由调用方赋值。
    pub id: i64,
    #[serde(default)]
    pub name: String,
    /// 正则表达式字符串。
    #[serde(default)]
    pub rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// 排序序号；Legado 默认 -1。
    #[serde(default = "default_serial_number")]
    pub serial_number: i32,
    #[serde(default = "default_true")]
    pub enable: bool,
}

/// 书签（对照 Legado `Bookmark.kt`）。
///
/// Swift Reader-Core 仅有轻量 draft（无同步/搜索/排序），对齐 Legado 字段。
/// 主键 `time` 为创建时间戳；`(book_name, book_author)` 复合定位"哪本书的书签"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bookmark {
    /// 主键；创建时间戳（毫秒）。
    pub time: i64,
    #[serde(default)]
    pub book_name: String,
    #[serde(default)]
    pub book_author: String,
    #[serde(default)]
    pub chapter_index: i32,
    /// 章节内字符位置（滚动/阅读进度偏移）。
    #[serde(default)]
    pub chapter_pos: i32,
    #[serde(default)]
    pub chapter_name: String,
    /// 书签关联的原文片段（选中文本或当前页文本）。
    #[serde(default)]
    pub book_text: String,
    /// 用户批注内容。
    #[serde(default)]
    pub content: String,
}

/// 替换规则的目标（标题或正文）。对照 Swift `ReaderCoreManagedReplaceRuleTarget`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplaceRuleTarget {
    Title,
    Content,
}

/// 独立可配置替换规则（对照 Swift `ReaderCoreManagedReplaceRule` + Legado `ReplaceRule.kt`）。
///
/// 合并 Swift scope 过滤语义与 Legado 字段（`is_regex`/`order`/`group`）。
/// 字段 `order` 在 SQLite 列名为 `sort_order`（对照 Legado `@ColumnInfo(name = "sortOrder")`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceRule {
    /// 主键；Legado 默认 `System.currentTimeMillis()`。
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 匹配内容（正则或纯文本，由 `is_regex` 决定）。
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub replacement: String,
    /// 包含作用域 tokens（书名/书源 URL 片段，多值分隔）；空表示匹配全部。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// 是否作用于标题。Legado 默认 false。
    #[serde(default)]
    pub scope_title: bool,
    /// 是否作用于正文。Legado 默认 true。
    #[serde(default = "default_true")]
    pub scope_content: bool,
    /// 排除作用域 tokens；任一命中即不应用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_scope: Option<String>,
    #[serde(default = "default_true")]
    pub is_enabled: bool,
    /// true=正则匹配，false=纯文本替换。Legado 默认 true。
    #[serde(default = "default_true")]
    pub is_regex: bool,
    /// 正则替换超时（毫秒）。Legado 默认 3000。
    #[serde(default = "default_timeout_ms")]
    pub timeout_millisecond: i64,
    /// 排序；小的先执行。SQLite 列名 `sort_order`。
    #[serde(default)]
    pub order: i32,
}

/// 替换规则作用域评估上下文。对照 Swift `ReaderCoreReplaceRuleEvaluationContext`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceRuleEvaluationContext {
    pub book_title: String,
    pub source_name: String,
    pub source_url: String,
}

impl ReplaceRuleEvaluationContext {
    /// 全部小写化的可搜索字段（对照 Swift `searchableFields`）。
    fn searchable_fields(&self) -> [String; 3] {
        [
            self.book_title.to_lowercase(),
            self.source_name.to_lowercase(),
            self.source_url.to_lowercase(),
        ]
    }
}

/// 切分作用域 tokens：按 `,` / `;` / `|` 分割，trim + 小写，丢弃空串。
/// 对照 Swift `scopeTokens`（ReplaceRuleEngine.swift:202-210）。
pub fn scope_tokens(scope: Option<&str>) -> Vec<String> {
    let Some(scope) = scope else {
        return Vec::new();
    };
    scope
        .split([',', ';', '|'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 判断规则是否作用于指定目标。对照 Swift `matchesTarget`。
pub fn replace_rule_matches_target(rule: &ReplaceRule, target: ReplaceRuleTarget) -> bool {
    match target {
        ReplaceRuleTarget::Title => rule.scope_title,
        ReplaceRuleTarget::Content => rule.scope_content,
    }
}

/// 判断规则的作用域是否命中上下文。对照 Swift `matchesIncludeScope && !matchesExcludeScope`。
/// - include tokens 为空 → 匹配全部；
/// - 否则 ANY 语义：任一 token 是任一 searchable field 的子串即命中。
/// - exclude tokens 为空 → 不排除；
/// - 否则 ANY 语义：任一 token 命中即排除（排除优先于包含）。
pub fn replace_rule_matches_scope(rule: &ReplaceRule, ctx: &ReplaceRuleEvaluationContext) -> bool {
    let fields = ctx.searchable_fields();
    let include = scope_tokens(rule.scope.as_deref());
    let include_hit = include.is_empty()
        || include
            .iter()
            .any(|tok| fields.iter().any(|f| f.contains(tok)));
    if !include_hit {
        return false;
    }
    let exclude = scope_tokens(rule.exclude_scope.as_deref());
    if exclude.is_empty() {
        return true;
    }
    !exclude
        .iter()
        .any(|tok| fields.iter().any(|f| f.contains(tok)))
}

/// 字典搜索规则（对照 Swift `DictRule.swift` + Legado `DictRule.kt`）。
///
/// 主键为 `name`（字符串）。`url_rule` 含 `{{key}}` 占位的搜索 URL，
/// `show_rule` 为结果展示规则（空则直接返回原始响应体）。
/// 执行（网络请求 + 解析）由 Host 层完成，Core 只存取与校验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DictRule {
    /// 主键；规则名。
    pub name: String,
    #[serde(default)]
    pub url_rule: String,
    #[serde(default)]
    pub show_rule: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub sort_number: i32,
}

/// 书架分组（对照 Legado `BookGroup.kt`）。
///
/// Swift Reader-Core 不存在该实体，对照 Legado 新建。主键 `group_id`
/// 为整数 id（Legado 默认 `System.currentTimeMillis()`）。`group_name`
/// 是用户可见名称；`cover` 为分组封面 URL；`order` 控制排序（小的在前）；
/// `enable_refresh` 标记是否参与自动刷新；`show` 控制是否在书架显示。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookGroup {
    /// 主键；分组 id。Legado 默认 `System.currentTimeMillis()`。
    pub group_id: i64,
    #[serde(default)]
    pub group_name: String,
    /// 分组封面 URL（可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// 排序序号；小的在前。Legado 默认 0。
    #[serde(default)]
    pub order: i32,
    /// 是否参与自动刷新。Legado 默认 true。
    #[serde(default = "default_true")]
    pub enable_refresh: bool,
    /// 是否在书架显示。Legado 默认 true。
    #[serde(default = "default_true")]
    pub show: bool,
}

/// 阅读时长记录（对照 Legado `ReadRecord.kt`）。
///
/// Swift Reader-Core 不存在该实体，对照 Legado 新建。复合主键
/// `(device_id, book_name)`：每个设备对每本书维护一条记录。
/// `read_time` 为累计阅读时长（毫秒）；`last_read` 为上次阅读时间戳（毫秒）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRecord {
    /// 设备 id（Legado 用 `androidId`/`deviceId`）。
    #[serde(default)]
    pub device_id: String,
    /// 书名（与 Bookmark.bookName 同语义）。
    pub book_name: String,
    /// 累计阅读时长（毫秒）。
    #[serde(default)]
    pub read_time: i64,
    /// 上次阅读时间戳（毫秒）。
    #[serde(default)]
    pub last_read: i64,
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
            source.rule_search.as_ref().and_then(|v| v.as_str()),
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

    /// 真实 Legado 导出格式:ruleSearch/ruleBookInfo/ruleToc/ruleContent 是 JSON
    /// 对象(见 BookSource.kt),concurrentRate 是字符串(如 "3"/"0.5")。
    /// 修复 rb-legado-rulesearch-object-deser / rb-legado-concurrentrate-string-deser
    /// 后必须能反序列化,且 normalize_*_semantics 能从对象形态提取字段。
    #[test]
    fn legado_book_source_decodes_real_legado_object_rules_and_string_concurrent_rate() {
        let json = r#"
{
  "bookSourceName": "Real Legado Source",
  "bookSourceUrl": "https://real.example.test",
  "concurrentRate": "3",
  "ruleSearch": {
    "bookList": "class.item",
    "name": "tag.h3@tag.a@text",
    "author": "tag.p.1@tag.a@text##作者：",
    "bookUrl": "tag.h3@tag.a@href",
    "coverUrl": "tag.img@src",
    "checkKeyWord": "轮回乐园"
  },
  "ruleBookInfo": {
    "name": "tag.h1@tag.a@text",
    "author": "class.itemtxt@tag.p@tag.a@text",
    "tocUrl": "{{baseUrl}}/#dir",
    "init": "@js:java.ajax(source.bookSourceUrl)"
  },
  "ruleToc": {
    "chapterList": "id.list@tag.li",
    "chapterName": "tag.a@text",
    "chapterUrl": "tag.a@href",
    "nextTocUrl": "@xpath://div[@class='pages']//a/@href"
  },
  "ruleContent": {
    "content": "class.con@html##<div.*?>|</div>",
    "nextContentUrl": "@xpath://div[@class='prenext']//a/@href"
  }
}
"#;
        let source: LegadoBookSource =
            serde_json::from_str(json).expect("real legado format should decode");

        // concurrentRate 字符串形态被保留为 Value::String。
        assert_eq!(
            source.concurrent_rate.as_ref().and_then(|v| v.as_str()),
            Some("3")
        );
        // rule_search 是对象形态,字符串提取返回 None,但 structured 反序列化成功。
        assert!(source.rule_search.as_ref().unwrap().is_object());
        let semantics = BookSourceSemantics::from_legado(
            "real-legado-src",
            Some("Real Legado Source"),
            Some("https://real.example.test"),
            &source,
        );
        assert_eq!(semantics.rules.search.list.as_deref(), Some("class.item"));
        assert_eq!(
            semantics.rules.search.name.as_deref(),
            Some("tag.h3@tag.a@text")
        );
        assert_eq!(
            semantics.rules.search.author.as_deref(),
            Some("tag.p.1@tag.a@text##作者：")
        );
        assert_eq!(
            semantics.rules.detail.toc_url.as_deref(),
            Some("{{baseUrl}}/#dir")
        );
        assert_eq!(
            semantics.rules.detail.init.as_deref(),
            Some("@js:java.ajax(source.bookSourceUrl)")
        );
        assert_eq!(semantics.rules.toc.list.as_deref(), Some("id.list@tag.li"));
        assert_eq!(
            semantics.rules.content.content.as_deref(),
            Some("class.con@html##<div.*?>|</div>")
        );
    }

    /// concurrentRate 也接受数字形态(部分导出工具输出 int)。
    #[test]
    fn legado_book_source_concurrent_rate_accepts_int_or_string() {
        let int_json = r#"{"bookSourceUrl":"u","concurrentRate":3}"#;
        let s: LegadoBookSource = serde_json::from_str(int_json).expect("int concurrentRate");
        assert_eq!(s.concurrent_rate.as_ref().and_then(|v| v.as_i64()), Some(3));

        let str_json = r#"{"bookSourceUrl":"u","concurrentRate":"0.5"}"#;
        let s: LegadoBookSource = serde_json::from_str(str_json).expect("string concurrentRate");
        assert_eq!(
            s.concurrent_rate.as_ref().and_then(|v| v.as_str()),
            Some("0.5")
        );
    }
}

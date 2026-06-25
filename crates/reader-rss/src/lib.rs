//! Reader-Core RSS — feed parsing and subscription state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use regex::RegexBuilder;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current RSS library snapshot schema version.
pub const RSS_LIBRARY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const RSS_HEADER_IF_NONE_MATCH: &str = "If-None-Match";
pub const RSS_HEADER_IF_MODIFIED_SINCE: &str = "If-Modified-Since";
pub const RSS_HEADER_ETAG: &str = "ETag";
pub const RSS_HEADER_LAST_MODIFIED: &str = "Last-Modified";

/// Runtime capabilities surfaced by RSS source rules before host execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RssRuntimeCapability {
    CookieJar,
    Login,
    Javascript,
    WebView,
}

/// Parsed RSS/Atom feed metadata plus entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssFeed {
    pub title: String,
    pub feed_url: Option<String>,
    pub site_url: Option<String>,
    pub description: Option<String>,
    pub entries: Vec<RssEntry>,
}

/// One item from an RSS channel or Atom feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssEntry {
    /// Stable entry identity, derived from `guid`/`id`/`link`/`title`.
    pub id: String,
    pub title: String,
    pub link: Option<String>,
    pub summary: Option<String>,
    /// Raw date string from `pubDate`, `updated`, or `published`.
    pub published_at: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_fields: BTreeMap<String, serde_json::Value>,
}

/// Reader-Core subscription output item DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RssSubscriptionItem {
    pub title: String,
    pub link: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_fields: BTreeMap<String, serde_json::Value>,
}

/// Parsed JSON Feed page plus Reader-Core subscription DTO projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssJsonFeedPage {
    pub feed: RssFeed,
    pub items: Vec<RssSubscriptionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// RSS/Atom XML pagination metadata parsed without fetching the next page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssXmlPaginationPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    pub network_fetch_executed: bool,
}

/// Explore category metadata carried by the legacy RSS/Explore fixture manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreCategory {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_template: Option<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ExploreCategory>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
}

/// Offline fixture manifest used by legacy Explore/RSS conformance tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreFixtureManifest {
    pub source_id: String,
    pub source_name: String,
    pub fixture_root: String,
    #[serde(default)]
    pub categories: Vec<ExploreCategory>,
    #[serde(default)]
    pub snapshots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_result_count: Option<usize>,
    #[serde(default = "default_true")]
    pub no_network_replay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_fetch_saved_at: Option<i64>,
    #[serde(default = "default_true")]
    pub repeated_fetch_forbidden: bool,
}

/// Explore request input used by the legacy execution runtime before any
/// network access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRequest {
    pub source_id: String,
    pub source_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_title: Option<String>,
    #[serde(default = "default_explore_page")]
    pub page: u32,
    #[serde(default)]
    pub query_parameters: BTreeMap<String, String>,
}

impl ExploreRequest {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_required(&self.source_id, "source_id")?;
        validate_explore_required(&self.source_name, "source_name")?;
        validate_explore_optional(&self.category_id, "category_id")?;
        validate_explore_optional(&self.category_title, "category_title")?;
        validate_explore_optional(&self.screen_id, "screen_id")?;
        validate_explore_optional(&self.screen_title, "screen_title")?;
        if self.page == 0 {
            return Err(RssError::InvalidSubscription {
                field: "page".into(),
            });
        }
        if self
            .query_parameters
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            return Err(RssError::InvalidSubscription {
                field: "query_parameters".into(),
            });
        }
        Ok(())
    }
}

/// Minimal Explore rule fields needed for request planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRequestRule {
    pub source_id: String,
    pub source_name: String,
    #[serde(default = "default_true")]
    pub enabled_explore: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore_screen: Option<String>,
}

impl ExploreRequestRule {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_required(&self.source_id, "source_id")?;
        validate_explore_required(&self.source_name, "source_name")?;
        validate_explore_optional(&self.explore_url, "explore_url")?;
        validate_explore_optional(&self.explore_screen, "explore_screen")?;
        Ok(())
    }
}

/// Minimal non-JS Explore HTML rule fields needed for local fixture parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreHtmlParseRule {
    pub source_id: String,
    pub source_name: String,
    #[serde(default = "default_true")]
    pub enabled_explore: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_list: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page: Option<String>,
}

impl ExploreHtmlParseRule {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_required(&self.source_id, "source_id")?;
        validate_explore_required(&self.source_name, "source_name")?;
        if !self.enabled_explore {
            return Err(RssError::InvalidSubscription {
                field: "enabled_explore".into(),
            });
        }
        validate_explore_required(self.book_list.as_deref().unwrap_or_default(), "book_list")?;
        validate_explore_required(self.book_url.as_deref().unwrap_or_default(), "book_url")?;
        validate_explore_optional(&self.name, "name")?;
        validate_explore_optional(&self.author, "author")?;
        validate_explore_optional(&self.cover_url, "cover_url")?;
        validate_explore_optional(&self.intro, "intro")?;
        validate_explore_optional(&self.last_chapter, "last_chapter")?;
        validate_explore_optional(&self.kind, "kind")?;
        validate_explore_optional(&self.update_time, "update_time")?;
        validate_explore_optional(&self.tags, "tags")?;
        validate_explore_optional(&self.next_page, "next_page")
    }
}

/// One parsed Explore list item from a non-JS HTML fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreHtmlItem {
    pub id: String,
    pub title: String,
    pub book_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub raw_fields: BTreeMap<String, String>,
}

/// Parsed Explore list envelope matching the legacy local snapshot parser path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreHtmlParseResult {
    pub source_id: String,
    pub source_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_title: Option<String>,
    pub page: u32,
    pub items: Vec<ExploreHtmlItem>,
    pub total_count: usize,
    pub has_next_page: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    pub generated_at: i64,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub replayed_from_local_snapshot: bool,
}

/// Parsed Explore screen selector from legacy `exploreScreen` JSON or
/// delimited screen strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreScreen {
    #[serde(default, alias = "key", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(alias = "name", alias = "label")]
    pub title: String,
    #[serde(
        default,
        rename = "urlTemplate",
        alias = "url",
        alias = "exploreUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ExploreScreen>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
}

impl ExploreScreen {
    pub fn is_executable(&self) -> bool {
        non_empty_trimmed(self.url.as_deref()).is_some()
    }

    fn validate(&self) -> Result<(), RssError> {
        validate_explore_required(&self.title, "explore_screen.title")?;
        validate_explore_optional(&self.id, "explore_screen.id")?;
        validate_explore_optional(&self.url, "explore_screen.url")?;
        validate_explore_string_map(&self.style, "explore_screen.style")?;
        validate_explore_string_map(&self.metadata, "explore_screen.metadata")?;
        if let Some(children) = &self.children {
            for child in children {
                child.validate()?;
            }
        }
        Ok(())
    }
}

/// Request method emitted by Explore request planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExploreRequestMethod {
    GET,
    POST,
}

/// Content type hint from legacy URL DSL options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExploreExpectedContentType {
    Html,
    Json,
    Xml,
    Text,
}

/// Host capabilities needed to execute the planned request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExploreRequestCapability {
    NetworkRequest,
    CustomHeader,
    PostBody,
    Charset,
}

/// Body template emitted after variable expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRequestBodyTemplate {
    pub template: String,
}

/// Transport-free Explore request plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRequestSpec {
    pub stage: String,
    pub method: ExploreRequestMethod,
    pub url_template: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_template: Option<ExploreRequestBodyTemplate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_content_type: Option<ExploreExpectedContentType>,
    pub debug_description: String,
    #[serde(default)]
    pub capability_requirements: BTreeSet<ExploreRequestCapability>,
}

/// Explore/RSS execution mode after the host has either fetched bytes or chosen
/// a deterministic local snapshot replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExploreRssExecutionMode {
    Network,
    LocalSnapshotReplay,
    RefreshSkipped,
}

/// Pure execution summary input. Parsed items are supplied by existing Core
/// parsers; this state machine only preserves the legacy runtime envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRssExecutionSummaryRequest {
    pub mode: ExploreRssExecutionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_spec: Option<ExploreRequestSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub items: Vec<RssSubscriptionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_decision: Option<RssRefreshDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_state: Option<RssRefreshState>,
}

impl ExploreRssExecutionSummaryRequest {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_optional(&self.final_url, "final_url")?;
        validate_explore_optional(&self.next_page_url, "next_page_url")?;
        validate_explore_optional(&self.snapshot_id, "snapshot_id")?;
        if let Some(decision) = &self.refresh_decision {
            validate_rss_refresh_decision(decision)?;
        }
        if let Some(state) = &self.refresh_state {
            validate_rss_refresh_state(state)?;
        }
        for item in &self.items {
            item.validate()?;
        }
        match self.mode {
            ExploreRssExecutionMode::Network => {
                if self.request_spec.is_none()
                    || self.response_status_code.is_none()
                    || self.final_url.is_none()
                {
                    return Err(RssError::InvalidSubscription {
                        field: "network_execution".into(),
                    });
                }
                if self.response_status_code == Some(304) {
                    if !self.items.is_empty()
                        || self.next_page_url.is_some()
                        || !self
                            .refresh_state
                            .as_ref()
                            .is_some_and(|state| state.not_modified)
                    {
                        return Err(RssError::InvalidSubscription {
                            field: "network_not_modified".into(),
                        });
                    }
                } else if self
                    .refresh_state
                    .as_ref()
                    .is_some_and(|state| state.not_modified)
                {
                    return Err(RssError::InvalidSubscription {
                        field: "refresh_state.not_modified".into(),
                    });
                }
            }
            ExploreRssExecutionMode::LocalSnapshotReplay => {
                if self.request_spec.is_some()
                    || self.response_status_code.is_some()
                    || self.final_url.is_some()
                    || self.snapshot_id.is_none()
                    || self.refresh_decision.is_some()
                    || self.refresh_state.is_some()
                {
                    return Err(RssError::InvalidSubscription {
                        field: "local_snapshot_replay".into(),
                    });
                }
            }
            ExploreRssExecutionMode::RefreshSkipped => {
                if self.request_spec.is_some()
                    || self.response_status_code.is_some()
                    || self.final_url.is_some()
                    || self.next_page_url.is_some()
                    || self.snapshot_id.is_some()
                    || !self.items.is_empty()
                    || !self
                        .refresh_decision
                        .as_ref()
                        .is_some_and(|decision| !decision.should_fetch)
                    || self.refresh_state.is_none()
                {
                    return Err(RssError::InvalidSubscription {
                        field: "refresh_skipped".into(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Transport-free legacy execution envelope for Explore/RSS parsing results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRssExecutionSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_spec: Option<ExploreRequestSpec>,
    pub network_accessed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
    pub replayed_from_local_snapshot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    pub item_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_item_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_state: Option<RssRefreshState>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Result of parsing a legacy rule-based HTML RSS source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRuleParseResult {
    pub items: Vec<RssSubscriptionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
}

/// Import/export model for a legacy Reader-Core RSS source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssSourceConfig {
    pub url: String,
    pub name: Option<String>,
    pub update_interval_minutes: Option<u32>,
    pub last_fetched_at: Option<i64>,
    pub last_etag: Option<String>,
    pub last_modified: Option<String>,
    pub enabled: bool,
    pub source_icon: Option<String>,
    pub source_group: Option<String>,
    pub source_comment: Option<String>,
    pub variable_comment: Option<String>,
    pub js_lib: Option<String>,
    pub enabled_cookie_jar: Option<bool>,
    pub concurrent_rate: Option<String>,
    pub header: Option<String>,
    pub login_url: Option<String>,
    pub login_ui: Option<String>,
    pub login_check_js: Option<String>,
    pub cover_decode_js: Option<String>,
    pub sort_url: Option<String>,
    pub single_url: bool,
    pub article_style: i32,
    pub rule_articles: Option<String>,
    pub rule_next_page: Option<String>,
    pub rule_title: Option<String>,
    pub rule_pub_date: Option<String>,
    pub rule_description: Option<String>,
    pub rule_image: Option<String>,
    pub rule_link: Option<String>,
    pub rule_content: Option<String>,
    pub content_whitelist: Option<String>,
    pub content_blacklist: Option<String>,
    pub should_override_url_loading: Option<String>,
    pub style: Option<String>,
    pub enable_js: bool,
    pub load_with_base_url: bool,
    pub inject_js: Option<String>,
    pub last_update_time: Option<i64>,
    pub custom_order: Option<i64>,
    pub unknown_fields: BTreeMap<String, serde_json::Value>,
}

/// Import/export model for a legacy Reader-Core subscription source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RssSubscriptionSourceConfig {
    pub id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_interval_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<i64>,
    #[serde(default, rename = "lastETag", skip_serializing_if = "Option::is_none")]
    pub last_etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_rules: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_fields: BTreeMap<String, serde_json::Value>,
}

/// Stored subscription state for a feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssSubscription {
    pub subscription_id: String,
    pub feed_url: String,
    pub title: String,
    pub site_url: Option<String>,
    pub enabled: bool,
    pub last_fetch_at: Option<i64>,
    pub last_entry_id: Option<String>,
    pub unread_count: u32,
}

/// Result of merging a newly fetched feed into subscription state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshResult {
    pub subscription: RssSubscription,
    pub new_entries: Vec<RssEntry>,
}

/// Stored state for one RSS entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssEntryState {
    pub subscription_id: String,
    pub entry: RssEntry,
    pub first_seen_at: i64,
    pub read: bool,
    pub read_at: Option<i64>,
    pub starred: bool,
}

/// Reason a refresh should be fetched or skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RssRefreshDecisionReason {
    Disabled,
    Forced,
    MissingLastFetchedAt,
    MissingUpdateInterval,
    IntervalElapsed,
    IntervalNotElapsed,
}

/// Pure refresh decision for RSS/subscription polling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshDecision {
    pub should_fetch: bool,
    pub reason: RssRefreshDecisionReason,
    pub evaluated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_eligible_fetch_at: Option<i64>,
}

/// Persistable refresh metadata from the last fetch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<i64>,
    #[serde(default, rename = "lastETag", skip_serializing_if = "Option::is_none")]
    pub last_etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_eligible_fetch_at: Option<i64>,
    #[serde(default)]
    pub not_modified: bool,
}

/// Minimal response metadata needed to update RSS refresh cache state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshResponseMetadata {
    pub status_code: u16,
    pub response_at: i64,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Core-side projection used when a legacy RSS source must execute through the
/// product-gated BookSource/WebView path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssProductGatedBookSource {
    pub id: String,
    pub book_source_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_order: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_source_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_comment: Option<String>,
    #[serde(default, rename = "jsLib", skip_serializing_if = "Option::is_none")]
    pub js_lib: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrent_rate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_check_js: Option<String>,
    pub explore_url: String,
    pub enabled_explore: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub header: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_ui: Option<String>,
    pub enabled_cookie_jar: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_decode_js: Option<String>,
    pub web_view: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown_fields: BTreeMap<String, serde_json::Value>,
}

/// Authorization scope required when an RSS source is handed to a product-gated host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RssSourceAuthorizationScope {
    FullAccess,
}

/// Core-owned login execution boundary for RSS sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RssLoginExecutionBoundary {
    UnsupportedWithoutProductGatedHost,
}

/// Capability requirement names used by the legacy P2 RSS fixture matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RssSourceCapabilityRequirement {
    NetworkRequest,
    CookieJar,
    Login,
    Javascript,
    WebView,
}

/// Product-gated handoff fields that Core can validate without executing host code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssProductGatedHandoffContract {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_login_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_login_check_js: Option<String>,
    pub source_enabled_cookie_jar: bool,
    #[serde(default)]
    pub unknown_field_keys: Vec<String>,
}

impl RssProductGatedHandoffContract {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_optional(&self.source_login_url, "source_login_url")?;
        validate_explore_optional(&self.source_login_check_js, "source_login_check_js")?;
        if self
            .unknown_field_keys
            .iter()
            .any(|key| key.trim().is_empty())
        {
            return Err(RssError::InvalidSubscription {
                field: "unknown_field_keys".into(),
            });
        }
        Ok(())
    }
}

/// Stable RSS source capability artifact for cookie/login product-gated handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssSourceCookieLoginCapabilityContract {
    pub authorization_scope: RssSourceAuthorizationScope,
    pub login_execution_boundary: RssLoginExecutionBoundary,
    #[serde(default)]
    pub capability_requirements: BTreeSet<RssSourceCapabilityRequirement>,
    pub product_gated_handoff: RssProductGatedHandoffContract,
}

impl RssSourceCookieLoginCapabilityContract {
    pub fn validate(&self) -> Result<(), RssError> {
        if !self
            .capability_requirements
            .contains(&RssSourceCapabilityRequirement::NetworkRequest)
        {
            return Err(RssError::InvalidSubscription {
                field: "capability_requirements".into(),
            });
        }
        self.product_gated_handoff.validate()
    }
}

/// Inputs used by the refresh decision state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssRefreshPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_interval_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<i64>,
    #[serde(default)]
    pub force_refresh: bool,
}

/// Complete export/import unit for RSS subscription and entry state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssLibrarySnapshot {
    pub schema_version: u32,
    pub exported_at: i64,
    #[serde(default)]
    pub subscriptions: Vec<RssSubscription>,
    #[serde(default)]
    pub entries: Vec<RssEntryState>,
}

impl RssLibrarySnapshot {
    pub fn empty(exported_at: i64) -> Self {
        Self {
            schema_version: RSS_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            subscriptions: Vec::new(),
            entries: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), RssError> {
        if self.schema_version != RSS_LIBRARY_SNAPSHOT_SCHEMA_VERSION {
            return Err(RssError::InvalidSnapshot {
                field: "schema_version".into(),
            });
        }

        let mut subscription_ids = HashSet::new();
        for subscription in &self.subscriptions {
            validate_subscription(subscription)?;
            if !subscription_ids.insert(subscription.subscription_id.clone()) {
                return Err(RssError::InvalidSnapshot {
                    field: "subscriptions".into(),
                });
            }
        }

        let mut entry_keys = HashSet::new();
        for state in &self.entries {
            validate_entry_state(state)?;
            if !subscription_ids.contains(&state.subscription_id) {
                return Err(RssError::InvalidSnapshot {
                    field: "entries.subscription_id".into(),
                });
            }
            let key = RssEntryKey {
                subscription_id: state.subscription_id.clone(),
                entry_id: state.entry.id.clone(),
            };
            if !entry_keys.insert(key) {
                return Err(RssError::InvalidSnapshot {
                    field: "entries".into(),
                });
            }
        }

        Ok(())
    }
}

/// In-memory RSS subscription and entry state.
///
/// This is a data-layer state machine. It deliberately does not fetch network
/// content; callers provide parsed feeds, and this type preserves read/starred
/// state across feed refreshes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RssLibrary {
    subscriptions: HashMap<String, RssSubscription>,
    entries: HashMap<RssEntryKey, RssEntryState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RssEntryKey {
    subscription_id: String,
    entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RssError {
    EmptyInput,
    UnsupportedFormat,
    InvalidJsonFeed {
        detail: String,
    },
    MissingField {
        field: String,
    },
    InvalidSubscription {
        field: String,
    },
    InvalidSnapshot {
        field: String,
    },
    SubscriptionNotFound {
        subscription_id: String,
    },
    EntryNotFound {
        subscription_id: String,
        entry_id: String,
    },
    UnsupportedRuntimeCapabilities {
        capabilities: Vec<RssRuntimeCapability>,
    },
}

impl std::fmt::Display for RssError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RssError::EmptyInput => write!(f, "RSS input is empty"),
            RssError::UnsupportedFormat => write!(f, "unsupported RSS/Atom format"),
            RssError::InvalidJsonFeed { detail } => write!(f, "invalid JSON Feed: {detail}"),
            RssError::MissingField { field } => write!(f, "missing RSS field: {field}"),
            RssError::InvalidSubscription { field } => {
                write!(f, "invalid RSS subscription field: {field}")
            }
            RssError::InvalidSnapshot { field } => {
                write!(f, "invalid RSS snapshot field: {field}")
            }
            RssError::SubscriptionNotFound { subscription_id } => {
                write!(f, "RSS subscription not found: {subscription_id}")
            }
            RssError::EntryNotFound {
                subscription_id,
                entry_id,
            } => {
                write!(
                    f,
                    "RSS entry not found: subscription={subscription_id} entry={entry_id}"
                )
            }
            RssError::UnsupportedRuntimeCapabilities { capabilities } => {
                let capabilities = capabilities
                    .iter()
                    .map(|capability| rss_runtime_capability_wire_value(*capability))
                    .collect::<Vec<_>>()
                    .join(",");
                write!(f, "unsupported RSS runtime capabilities: {capabilities}")
            }
        }
    }
}

impl std::error::Error for RssError {}

impl RssSourceConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            name: None,
            update_interval_minutes: None,
            last_fetched_at: None,
            last_etag: None,
            last_modified: None,
            enabled: true,
            source_icon: None,
            source_group: None,
            source_comment: None,
            variable_comment: None,
            js_lib: None,
            enabled_cookie_jar: None,
            concurrent_rate: None,
            header: None,
            login_url: None,
            login_ui: None,
            login_check_js: None,
            cover_decode_js: None,
            sort_url: None,
            single_url: false,
            article_style: 0,
            rule_articles: None,
            rule_next_page: None,
            rule_title: None,
            rule_pub_date: None,
            rule_description: None,
            rule_image: None,
            rule_link: None,
            rule_content: None,
            content_whitelist: None,
            content_blacklist: None,
            should_override_url_loading: None,
            style: None,
            enable_js: true,
            load_with_base_url: true,
            inject_js: None,
            last_update_time: None,
            custom_order: None,
            unknown_fields: BTreeMap::new(),
        }
    }

    pub fn has_rule_based_articles(&self) -> bool {
        non_empty_trimmed(self.rule_articles.as_deref()).is_some()
    }

    pub fn has_dynamic_header_rule(&self) -> bool {
        let Some(header) = non_empty_trimmed(self.header.as_deref()) else {
            return false;
        };
        let header = header.to_ascii_lowercase();
        header.starts_with("@js:") || header.starts_with("<js>")
    }

    pub fn refresh_policy(&self, force_refresh: bool) -> RssRefreshPolicy {
        RssRefreshPolicy {
            enabled: self.enabled,
            update_interval_minutes: self.update_interval_minutes,
            last_fetched_at: self.last_fetched_at,
            force_refresh,
        }
    }
}

impl Serialize for RssSourceConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut fields = BTreeMap::new();
        fields.insert("url".to_string(), serde_json::json!(self.url));
        fields.insert("sourceUrl".to_string(), serde_json::json!(self.url));
        insert_optional_json(&mut fields, "name", self.name.as_ref());
        insert_optional_json(&mut fields, "sourceName", self.name.as_ref());
        insert_optional_json(
            &mut fields,
            "updateIntervalMinutes",
            self.update_interval_minutes,
        );
        insert_optional_json(&mut fields, "lastFetchedAt", self.last_fetched_at);
        insert_optional_json(&mut fields, "lastETag", self.last_etag.as_ref());
        insert_optional_json(&mut fields, "lastModified", self.last_modified.as_ref());
        fields.insert("enabled".to_string(), serde_json::json!(self.enabled));
        insert_optional_json(&mut fields, "sourceIcon", self.source_icon.as_ref());
        insert_optional_json(&mut fields, "sourceGroup", self.source_group.as_ref());
        insert_optional_json(&mut fields, "sourceComment", self.source_comment.as_ref());
        insert_optional_json(
            &mut fields,
            "variableComment",
            self.variable_comment.as_ref(),
        );
        insert_optional_json(&mut fields, "jsLib", self.js_lib.as_ref());
        insert_optional_json(&mut fields, "enabledCookieJar", self.enabled_cookie_jar);
        insert_optional_json(&mut fields, "concurrentRate", self.concurrent_rate.as_ref());
        insert_optional_json(&mut fields, "header", self.header.as_ref());
        insert_optional_json(&mut fields, "loginUrl", self.login_url.as_ref());
        insert_optional_json(&mut fields, "loginUi", self.login_ui.as_ref());
        insert_optional_json(&mut fields, "loginCheckJs", self.login_check_js.as_ref());
        insert_optional_json(&mut fields, "coverDecodeJs", self.cover_decode_js.as_ref());
        insert_optional_json(&mut fields, "sortUrl", self.sort_url.as_ref());
        fields.insert("singleUrl".to_string(), serde_json::json!(self.single_url));
        fields.insert(
            "articleStyle".to_string(),
            serde_json::json!(self.article_style),
        );
        insert_optional_json(&mut fields, "ruleArticles", self.rule_articles.as_ref());
        insert_optional_json(&mut fields, "ruleNextPage", self.rule_next_page.as_ref());
        insert_optional_json(&mut fields, "ruleTitle", self.rule_title.as_ref());
        insert_optional_json(&mut fields, "rulePubDate", self.rule_pub_date.as_ref());
        insert_optional_json(
            &mut fields,
            "ruleDescription",
            self.rule_description.as_ref(),
        );
        insert_optional_json(&mut fields, "ruleImage", self.rule_image.as_ref());
        insert_optional_json(&mut fields, "ruleLink", self.rule_link.as_ref());
        insert_optional_json(&mut fields, "ruleContent", self.rule_content.as_ref());
        insert_optional_json(
            &mut fields,
            "contentWhitelist",
            self.content_whitelist.as_ref(),
        );
        insert_optional_json(
            &mut fields,
            "contentBlacklist",
            self.content_blacklist.as_ref(),
        );
        insert_optional_json(
            &mut fields,
            "shouldOverrideUrlLoading",
            self.should_override_url_loading.as_ref(),
        );
        insert_optional_json(&mut fields, "style", self.style.as_ref());
        fields.insert("enableJs".to_string(), serde_json::json!(self.enable_js));
        fields.insert(
            "loadWithBaseUrl".to_string(),
            serde_json::json!(self.load_with_base_url),
        );
        insert_optional_json(&mut fields, "injectJs", self.inject_js.as_ref());
        insert_optional_json(&mut fields, "lastUpdateTime", self.last_update_time);
        insert_optional_json(&mut fields, "customOrder", self.custom_order);

        for (key, value) in &self.unknown_fields {
            if !is_rss_source_config_key(key) {
                fields.insert(key.clone(), value.clone());
            }
        }

        fields.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RssSourceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let url = optional_string_alias(&fields, "url", "sourceUrl")?.unwrap_or_default();
        let name = optional_string_alias(&fields, "name", "sourceName")?;
        let unknown_fields = fields
            .iter()
            .filter(|(key, _)| !is_rss_source_config_key(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        Ok(Self {
            url,
            name,
            update_interval_minutes: optional_u32(&fields, "updateIntervalMinutes")?,
            last_fetched_at: optional_i64(&fields, "lastFetchedAt")?,
            last_etag: optional_string(&fields, "lastETag")?,
            last_modified: optional_string(&fields, "lastModified")?,
            enabled: optional_bool(&fields, "enabled")?.unwrap_or(true),
            source_icon: optional_string(&fields, "sourceIcon")?,
            source_group: optional_string(&fields, "sourceGroup")?,
            source_comment: optional_string(&fields, "sourceComment")?,
            variable_comment: optional_string(&fields, "variableComment")?,
            js_lib: optional_string(&fields, "jsLib")?,
            enabled_cookie_jar: optional_bool(&fields, "enabledCookieJar")?,
            concurrent_rate: optional_string(&fields, "concurrentRate")?,
            header: optional_string(&fields, "header")?,
            login_url: optional_string(&fields, "loginUrl")?,
            login_ui: optional_string(&fields, "loginUi")?,
            login_check_js: optional_string(&fields, "loginCheckJs")?,
            cover_decode_js: optional_string(&fields, "coverDecodeJs")?,
            sort_url: optional_string(&fields, "sortUrl")?,
            single_url: optional_bool(&fields, "singleUrl")?.unwrap_or(false),
            article_style: optional_i32(&fields, "articleStyle")?.unwrap_or(0),
            rule_articles: optional_string(&fields, "ruleArticles")?,
            rule_next_page: optional_string(&fields, "ruleNextPage")?,
            rule_title: optional_string(&fields, "ruleTitle")?,
            rule_pub_date: optional_string(&fields, "rulePubDate")?,
            rule_description: optional_string(&fields, "ruleDescription")?,
            rule_image: optional_string(&fields, "ruleImage")?,
            rule_link: optional_string(&fields, "ruleLink")?,
            rule_content: optional_string(&fields, "ruleContent")?,
            content_whitelist: optional_string(&fields, "contentWhitelist")?,
            content_blacklist: optional_string(&fields, "contentBlacklist")?,
            should_override_url_loading: optional_string(&fields, "shouldOverrideUrlLoading")?,
            style: optional_string(&fields, "style")?,
            enable_js: optional_bool(&fields, "enableJs")?.unwrap_or(true),
            load_with_base_url: optional_bool(&fields, "loadWithBaseUrl")?.unwrap_or(true),
            inject_js: optional_string(&fields, "injectJs")?,
            last_update_time: optional_i64(&fields, "lastUpdateTime")?,
            custom_order: optional_i64(&fields, "customOrder")?,
            unknown_fields,
        })
    }
}

impl RssSubscriptionItem {
    pub fn new(title: impl Into<String>, link: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            link: link.into(),
            author: None,
            summary: None,
            published_at: None,
            source_id: String::new(),
            source_name: None,
            unknown_fields: BTreeMap::new(),
        }
    }

    pub fn from_entry(
        entry: &RssEntry,
        source_id: impl Into<String>,
        source_name: Option<String>,
    ) -> Self {
        let mut item = Self::new(
            entry.title.clone(),
            entry.link.clone().unwrap_or_else(|| entry.id.clone()),
        );
        item.summary = entry.summary.clone();
        item.published_at = entry.published_at.clone();
        item.source_id = source_id.into();
        item.source_name = source_name;
        item.unknown_fields = entry.unknown_fields.clone();
        item.unknown_fields
            .insert("entryId".into(), serde_json::json!(entry.id));
        item
    }

    pub fn validate(&self) -> Result<(), RssError> {
        if self.title.trim().is_empty() {
            return Err(RssError::MissingField {
                field: "subscription_item.title".into(),
            });
        }
        if self.link.trim().is_empty() {
            return Err(RssError::MissingField {
                field: "subscription_item.link".into(),
            });
        }
        Ok(())
    }
}

impl ExploreCategory {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_required(&self.id, "categories.id")?;
        validate_explore_required(&self.title, "categories.title")?;
        validate_explore_optional(&self.url_template, "categories.url_template")?;
        if let Some(children) = &self.children {
            let mut child_ids = BTreeSet::<String>::new();
            for child in children {
                child.validate()?;
                if !child_ids.insert(child.id.clone()) {
                    return Err(RssError::InvalidSubscription {
                        field: "categories.children.id".into(),
                    });
                }
            }
        }
        if let Some(metadata) = &self.metadata {
            if metadata
                .iter()
                .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
            {
                return Err(RssError::InvalidSubscription {
                    field: "categories.metadata".into(),
                });
            }
        }
        Ok(())
    }
}

impl ExploreFixtureManifest {
    pub fn validate(&self) -> Result<(), RssError> {
        validate_explore_required(&self.source_id, "source_id")?;
        validate_explore_required(&self.source_name, "source_name")?;
        validate_explore_required(&self.fixture_root, "fixture_root")?;
        if self
            .expected_result_count
            .is_some_and(|expected_result_count| expected_result_count == 0)
        {
            return Err(RssError::InvalidSubscription {
                field: "expected_result_count".into(),
            });
        }

        let mut category_ids = BTreeSet::<String>::new();
        for category in &self.categories {
            category.validate()?;
            if !category_ids.insert(category.id.clone()) {
                return Err(RssError::InvalidSubscription {
                    field: "categories.id".into(),
                });
            }
        }

        let mut snapshots = BTreeSet::<String>::new();
        for snapshot in &self.snapshots {
            validate_explore_snapshot_path(snapshot)?;
            if !snapshots.insert(snapshot.clone()) {
                return Err(RssError::InvalidSubscription {
                    field: "snapshots".into(),
                });
            }
        }
        Ok(())
    }

    pub fn requires_offline_replay(&self) -> bool {
        self.no_network_replay && self.repeated_fetch_forbidden
    }
}

pub fn parse_explore_screens(rule: &ExploreRequestRule) -> Result<Vec<ExploreScreen>, RssError> {
    rule.validate()?;
    let Some(raw_screens) = non_empty_trimmed(rule.explore_screen.as_deref()) else {
        return Ok(Vec::new());
    };
    let screens = if raw_screens.trim_start().starts_with('[') {
        parse_explore_screen_json_array(raw_screens)
            .filter(|screens| !screens.is_empty())
            .unwrap_or_else(|| parse_delimited_explore_screens(raw_screens))
    } else {
        parse_delimited_explore_screens(raw_screens)
    };
    for screen in &screens {
        screen.validate()?;
    }
    Ok(screens)
}

pub fn build_explore_request_spec(
    rule: &ExploreRequestRule,
    request: &ExploreRequest,
) -> Result<ExploreRequestSpec, RssError> {
    rule.validate()?;
    request.validate()?;
    if !rule.enabled_explore {
        return Err(RssError::InvalidSubscription {
            field: "enabled_explore".into(),
        });
    }

    let screens = parse_explore_screens(rule)?;
    let selected_url = selected_explore_screen_url(rule, request, &screens)?;
    let (url_template, options) = split_legacy_explore_url_dsl(&selected_url)?;
    let variables = explore_request_variables(request);
    let url_template = expand_explore_template(&url_template, &variables);
    let method = explore_method_from_options(&options)?;
    let headers = explore_headers_from_options(&options)?;
    let body_template =
        explore_body_from_options(&options)?.map(|body| ExploreRequestBodyTemplate {
            template: expand_explore_template(&body, &variables),
        });
    let charset =
        explore_string_option(&options, "charset")?.map(|value| value.to_ascii_lowercase());
    let expected_content_type = explore_content_type_from_options(&options)?;

    let mut capability_requirements = BTreeSet::from([ExploreRequestCapability::NetworkRequest]);
    if !headers.is_empty() {
        capability_requirements.insert(ExploreRequestCapability::CustomHeader);
    }
    if body_template.is_some() {
        capability_requirements.insert(ExploreRequestCapability::PostBody);
    }
    if charset.is_some() {
        capability_requirements.insert(ExploreRequestCapability::Charset);
    }

    Ok(ExploreRequestSpec {
        stage: "explore".into(),
        method,
        url_template,
        headers,
        body_template,
        charset,
        expected_content_type,
        debug_description: format!("explore:{}:page:{}", request.source_id, request.page),
        capability_requirements,
    })
}

pub fn parse_explore_html(
    html: &str,
    rule: &ExploreHtmlParseRule,
    request: &ExploreRequest,
    base_url: Option<&str>,
    snapshot_id: Option<&str>,
    generated_at: i64,
    replayed_from_local_snapshot: bool,
) -> Result<ExploreHtmlParseResult, RssError> {
    rule.validate()?;
    request.validate()?;
    let input = html.trim();
    if input.is_empty() {
        return Err(RssError::EmptyInput);
    }

    let document = Html::parse_document(input);
    let book_list = rule
        .book_list
        .as_deref()
        .ok_or_else(|| RssError::InvalidSubscription {
            field: "book_list".into(),
        })?;
    let book_selector = rss_rule_selector(book_list)?;
    let containers = document.select(&book_selector).collect::<Vec<_>>();

    let mut items = Vec::new();
    for container in &containers {
        if let Some(item) = parse_explore_html_item(*container, rule, base_url)? {
            items.push(item);
        }
    }
    let (has_next_page, next_page_url) = explore_html_next_page(&document, rule, base_url)?;
    let warnings = if containers.len() > items.len() {
        vec![format!(
            "dropped_invalid_explore_items:{}",
            containers.len() - items.len()
        )]
    } else {
        Vec::new()
    };

    Ok(ExploreHtmlParseResult {
        source_id: request.source_id.clone(),
        source_name: request.source_name.clone(),
        category_id: request.category_id.clone(),
        category_title: request.category_title.clone(),
        screen_id: request.screen_id.clone(),
        screen_title: request.screen_title.clone(),
        page: request.page,
        total_count: items.len(),
        items,
        has_next_page,
        next_page_url,
        snapshot_id: snapshot_id.map(ToString::to_string),
        generated_at,
        warnings,
        replayed_from_local_snapshot,
    })
}

pub fn summarize_explore_rss_execution(
    request: &ExploreRssExecutionSummaryRequest,
) -> Result<ExploreRssExecutionSummary, RssError> {
    request.validate()?;
    let replayed_from_local_snapshot = request.mode == ExploreRssExecutionMode::LocalSnapshotReplay;
    let warnings = match request.mode {
        ExploreRssExecutionMode::Network if request.response_status_code == Some(304) => {
            vec!["not_modified".into()]
        }
        ExploreRssExecutionMode::LocalSnapshotReplay => vec!["replayed_from_local_snapshot".into()],
        ExploreRssExecutionMode::RefreshSkipped => {
            let decision =
                request
                    .refresh_decision
                    .as_ref()
                    .ok_or_else(|| RssError::InvalidSubscription {
                        field: "refresh_skipped".into(),
                    })?;
            vec![format!(
                "refresh_skipped_{}",
                rss_refresh_decision_reason_wire_value(decision.reason)
            )]
        }
        ExploreRssExecutionMode::Network => Vec::new(),
    };
    Ok(ExploreRssExecutionSummary {
        request_spec: request.request_spec.clone(),
        network_accessed: request.mode == ExploreRssExecutionMode::Network,
        response_status_code: request.response_status_code,
        final_url: request.final_url.clone(),
        next_page_url: request.next_page_url.clone(),
        replayed_from_local_snapshot,
        snapshot_id: request.snapshot_id.clone(),
        item_count: request.items.len(),
        first_item_title: request.items.first().map(|item| item.title.clone()),
        refresh_state: request.refresh_state.clone(),
        warnings,
    })
}

impl RssSubscriptionSourceConfig {
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            name: None,
            source_group: None,
            update_interval_minutes: None,
            last_fetched_at: None,
            last_etag: None,
            last_modified: None,
            enabled: true,
            replace_rules: None,
            unknown_fields: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), RssError> {
        if self.id.trim().is_empty() {
            return Err(RssError::InvalidSubscription { field: "id".into() });
        }
        if self.url.trim().is_empty() {
            return Err(RssError::InvalidSubscription {
                field: "url".into(),
            });
        }
        if self
            .replace_rules
            .as_ref()
            .is_some_and(|rules| rules.iter().any(|rule| rule.trim().is_empty()))
        {
            return Err(RssError::InvalidSubscription {
                field: "replace_rules".into(),
            });
        }
        Ok(())
    }

    pub fn has_replace_rules(&self) -> bool {
        self.replace_rules
            .as_ref()
            .is_some_and(|rules| rules.iter().any(|rule| !rule.trim().is_empty()))
    }

    pub fn refresh_policy(&self, force_refresh: bool) -> RssRefreshPolicy {
        RssRefreshPolicy {
            enabled: self.enabled,
            update_interval_minutes: self.update_interval_minutes,
            last_fetched_at: self.last_fetched_at,
            force_refresh,
        }
    }

    pub fn refresh_state(&self) -> RssRefreshState {
        RssRefreshState {
            last_fetched_at: self.last_fetched_at,
            last_etag: self.last_etag.clone(),
            last_modified: self.last_modified.clone(),
            next_eligible_fetch_at: self
                .update_interval_minutes
                .filter(|minutes| *minutes > 0)
                .and_then(|minutes| {
                    self.last_fetched_at
                        .map(|last_fetched_at| last_fetched_at.saturating_add(minutes as i64 * 60))
                }),
            not_modified: false,
        }
    }
}

impl RssSubscription {
    pub fn new(
        subscription_id: impl Into<String>,
        feed_url: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<Self, RssError> {
        let subscription_id = subscription_id.into().trim().to_string();
        let feed_url = feed_url.into().trim().to_string();
        let title = title.into().trim().to_string();
        validate_subscription_fields(&subscription_id, &feed_url)?;
        Ok(Self {
            subscription_id,
            title: if title.is_empty() {
                feed_url.clone()
            } else {
                title
            },
            feed_url,
            site_url: None,
            enabled: true,
            last_fetch_at: None,
            last_entry_id: None,
            unread_count: 0,
        })
    }

    /// Merge parsed feed metadata and unread state into this subscription.
    ///
    /// Feed entries are assumed to be ordered newest-first. New entries are the
    /// prefix before the previously observed `last_entry_id`; if that id has
    /// fallen out of the feed window, the current feed is treated as all new.
    pub fn apply_feed(
        &mut self,
        feed: &RssFeed,
        fetched_at: i64,
    ) -> Result<RssRefreshResult, RssError> {
        validate_subscription_fields(&self.subscription_id, &self.feed_url)?;
        if feed.title.trim().is_empty() {
            return Err(RssError::MissingField {
                field: "feed.title".into(),
            });
        }

        let new_entries = collect_new_entries(&feed.entries, self.last_entry_id.as_deref());
        self.title = feed.title.clone();
        if let Some(feed_url) = feed.feed_url.as_ref().filter(|url| !url.trim().is_empty()) {
            self.feed_url = feed_url.clone();
        }
        if let Some(site_url) = feed.site_url.as_ref().filter(|url| !url.trim().is_empty()) {
            self.site_url = Some(site_url.clone());
        }
        self.last_fetch_at = Some(fetched_at);
        if let Some(entry) = feed.entries.first() {
            self.last_entry_id = Some(entry.id.clone());
        }
        self.unread_count = self.unread_count.saturating_add(new_entries.len() as u32);

        Ok(RssRefreshResult {
            subscription: self.clone(),
            new_entries,
        })
    }

    pub fn mark_all_read(&mut self) {
        self.unread_count = 0;
    }
}

/// Decide whether an RSS/subscription fetch should run at `evaluated_at`.
///
/// This is transport-free and mirrors the legacy Reader-Core refresh contract:
/// disabled sources never fetch, forced refresh bypasses interval checks, and a
/// missing/zero interval or missing previous fetch timestamp is fetch-eligible.
pub fn decide_rss_refresh(policy: &RssRefreshPolicy, evaluated_at: i64) -> RssRefreshDecision {
    if !policy.enabled {
        return RssRefreshDecision {
            should_fetch: false,
            reason: RssRefreshDecisionReason::Disabled,
            evaluated_at,
            next_eligible_fetch_at: None,
        };
    }

    if policy.force_refresh {
        return RssRefreshDecision {
            should_fetch: true,
            reason: RssRefreshDecisionReason::Forced,
            evaluated_at,
            next_eligible_fetch_at: None,
        };
    }

    let Some(update_interval_minutes) = policy
        .update_interval_minutes
        .filter(|minutes| *minutes > 0)
    else {
        return RssRefreshDecision {
            should_fetch: true,
            reason: RssRefreshDecisionReason::MissingUpdateInterval,
            evaluated_at,
            next_eligible_fetch_at: None,
        };
    };

    let Some(last_fetched_at) = policy.last_fetched_at else {
        return RssRefreshDecision {
            should_fetch: true,
            reason: RssRefreshDecisionReason::MissingLastFetchedAt,
            evaluated_at,
            next_eligible_fetch_at: None,
        };
    };

    let interval_seconds = (update_interval_minutes as i64).saturating_mul(60);
    let next_eligible_fetch_at = last_fetched_at.saturating_add(interval_seconds);
    if evaluated_at >= next_eligible_fetch_at {
        RssRefreshDecision {
            should_fetch: true,
            reason: RssRefreshDecisionReason::IntervalElapsed,
            evaluated_at,
            next_eligible_fetch_at: Some(next_eligible_fetch_at),
        }
    } else {
        RssRefreshDecision {
            should_fetch: false,
            reason: RssRefreshDecisionReason::IntervalNotElapsed,
            evaluated_at,
            next_eligible_fetch_at: Some(next_eligible_fetch_at),
        }
    }
}

pub fn rss_conditional_refresh_headers(state: &RssRefreshState) -> BTreeMap<String, String> {
    rss_conditional_refresh_headers_from_parts(
        state.last_etag.as_deref(),
        state.last_modified.as_deref(),
    )
}

pub fn rss_conditional_refresh_headers_from_parts(
    last_etag: Option<&str>,
    last_modified: Option<&str>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(etag) = non_empty_trimmed(last_etag) {
        headers.insert(RSS_HEADER_IF_NONE_MATCH.to_string(), etag.to_string());
    }
    if let Some(last_modified) = non_empty_trimmed(last_modified) {
        headers.insert(
            RSS_HEADER_IF_MODIFIED_SINCE.to_string(),
            last_modified.to_string(),
        );
    }
    headers
}

pub fn rss_source_runtime_capabilities(source: &RssSourceConfig) -> BTreeSet<RssRuntimeCapability> {
    let mut capabilities = BTreeSet::new();
    if source.enabled_cookie_jar == Some(true) {
        capabilities.insert(RssRuntimeCapability::CookieJar);
    }
    if non_empty_trimmed(source.login_url.as_deref()).is_some()
        || non_empty_trimmed(source.login_ui.as_deref()).is_some()
    {
        capabilities.insert(RssRuntimeCapability::Login);
    }
    if non_empty_trimmed(source.login_check_js.as_deref()).is_some()
        || non_empty_trimmed(source.cover_decode_js.as_deref()).is_some()
    {
        capabilities.insert(RssRuntimeCapability::Javascript);
    }
    if non_empty_trimmed(source.inject_js.as_deref()).is_some()
        || non_empty_trimmed(source.should_override_url_loading.as_deref()).is_some()
    {
        capabilities.insert(RssRuntimeCapability::WebView);
    }
    capabilities
}

pub fn unsupported_rss_rule_runtime_capabilities(
    source: &RssSourceConfig,
) -> Vec<RssRuntimeCapability> {
    let mut unsupported = Vec::new();
    let rule_requires_javascript = [
        source.rule_articles.as_deref(),
        source.rule_next_page.as_deref(),
        source.rule_title.as_deref(),
        source.rule_pub_date.as_deref(),
        source.rule_description.as_deref(),
        source.rule_image.as_deref(),
        source.rule_link.as_deref(),
        source.rule_content.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|value| non_empty_trimmed(Some(value)))
    .any(rss_rule_text_requires_javascript);

    if rule_requires_javascript
        || non_empty_trimmed(source.login_check_js.as_deref()).is_some()
        || non_empty_trimmed(source.cover_decode_js.as_deref()).is_some()
    {
        unsupported.push(RssRuntimeCapability::Javascript);
    }
    if non_empty_trimmed(source.inject_js.as_deref()).is_some()
        || non_empty_trimmed(source.should_override_url_loading.as_deref()).is_some()
    {
        unsupported.push(RssRuntimeCapability::WebView);
    }
    if non_empty_trimmed(source.login_url.as_deref()).is_some()
        || non_empty_trimmed(source.login_ui.as_deref()).is_some()
    {
        unsupported.push(RssRuntimeCapability::Login);
    }
    unsupported
}

pub fn rss_source_static_headers(
    source: &RssSourceConfig,
) -> Result<BTreeMap<String, String>, RssError> {
    if source.has_dynamic_header_rule() {
        return Err(RssError::UnsupportedRuntimeCapabilities {
            capabilities: vec![RssRuntimeCapability::Javascript],
        });
    }
    let Some(header) = non_empty_trimmed(source.header.as_deref()) else {
        return Ok(BTreeMap::new());
    };
    Ok(serde_json::from_str::<BTreeMap<String, String>>(header).unwrap_or_default())
}

pub fn rss_source_request_headers(
    source: &RssSourceConfig,
    additional_headers: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, RssError> {
    let mut headers = rss_source_static_headers(source)?;
    headers.extend(rss_conditional_refresh_headers_from_parts(
        source.last_etag.as_deref(),
        source.last_modified.as_deref(),
    ));
    headers.extend(
        additional_headers
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    Ok(headers)
}

pub fn project_rss_source_to_product_gated_book_source(
    source: &RssSourceConfig,
    source_id: Option<&str>,
    additional_headers: &BTreeMap<String, String>,
) -> Result<RssProductGatedBookSource, RssError> {
    let Some(url) = non_empty_trimmed(Some(source.url.as_str())) else {
        return Err(RssError::MissingField {
            field: "url".into(),
        });
    };
    let id = rss_source_projection_id(source, source_id);
    let headers = rss_source_request_headers(source, additional_headers)?;
    let enabled_cookie_jar =
        rss_source_runtime_capabilities(source).contains(&RssRuntimeCapability::CookieJar);

    Ok(RssProductGatedBookSource {
        id: id.clone(),
        book_source_name: source.name.clone().unwrap_or_else(|| id.clone()),
        book_source_url: source_root_url(url),
        book_source_group: source.source_group.clone(),
        custom_order: source.custom_order,
        book_source_comment: source.source_comment.clone(),
        source_icon: source.source_icon.clone(),
        variable_comment: source.variable_comment.clone(),
        js_lib: source.js_lib.clone(),
        concurrent_rate: source.concurrent_rate.clone(),
        last_update_time: source.last_update_time.map(|value| value.to_string()),
        login_check_js: source.login_check_js.clone(),
        explore_url: url.to_string(),
        enabled_explore: true,
        header: headers,
        login_url: source.login_url.clone(),
        login_ui: source.login_ui.clone(),
        enabled_cookie_jar,
        cover_decode_js: source.cover_decode_js.clone(),
        web_view: true,
        unknown_fields: rss_product_gated_unknown_fields(source),
    })
}

pub fn build_rss_source_cookie_login_capability_contract(
    source: &RssSourceConfig,
    source_id: Option<&str>,
    additional_headers: &BTreeMap<String, String>,
) -> Result<RssSourceCookieLoginCapabilityContract, RssError> {
    let projection =
        project_rss_source_to_product_gated_book_source(source, source_id, additional_headers)?;
    let mut capability_requirements =
        BTreeSet::from([RssSourceCapabilityRequirement::NetworkRequest]);
    for capability in rss_source_runtime_capabilities(source) {
        capability_requirements.insert(rss_source_capability_requirement(capability));
    }

    let contract = RssSourceCookieLoginCapabilityContract {
        authorization_scope: RssSourceAuthorizationScope::FullAccess,
        login_execution_boundary: RssLoginExecutionBoundary::UnsupportedWithoutProductGatedHost,
        capability_requirements,
        product_gated_handoff: RssProductGatedHandoffContract {
            source_login_url: projection.login_url,
            source_login_check_js: projection.login_check_js,
            source_enabled_cookie_jar: projection.enabled_cookie_jar,
            unknown_field_keys: projection.unknown_fields.keys().cloned().collect(),
        },
    };
    contract.validate()?;
    Ok(contract)
}

pub fn rss_product_gated_unknown_fields(
    source: &RssSourceConfig,
) -> BTreeMap<String, serde_json::Value> {
    let mut fields = BTreeMap::new();
    if let Some(sort_url) = non_empty_trimmed(source.sort_url.as_deref()) {
        fields.insert("rssSortUrl".into(), serde_json::json!(sort_url));
    }
    if source.single_url {
        fields.insert("rssSingleUrl".into(), serde_json::json!(true));
    }
    fields.insert(
        "rssArticleStyle".into(),
        serde_json::json!(source.article_style),
    );
    if let Some(should_override_url_loading) =
        non_empty_trimmed(source.should_override_url_loading.as_deref())
    {
        fields.insert(
            "shouldOverrideUrlLoading".into(),
            serde_json::json!(should_override_url_loading),
        );
    }
    if let Some(style) = non_empty_trimmed(source.style.as_deref()) {
        fields.insert("style".into(), serde_json::json!(style));
    }
    fields.insert("enableJs".into(), serde_json::json!(source.enable_js));
    fields.insert(
        "loadWithBaseUrl".into(),
        serde_json::json!(source.load_with_base_url),
    );
    if let Some(inject_js) = non_empty_trimmed(source.inject_js.as_deref()) {
        fields.insert("injectJs".into(), serde_json::json!(inject_js));
    }
    fields.extend(
        source
            .unknown_fields
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    fields
}

pub fn rss_refresh_state_from_response(
    previous_state: &RssRefreshState,
    response: &RssRefreshResponseMetadata,
    update_interval_minutes: Option<u32>,
) -> RssRefreshState {
    let last_etag = header_value(&response.headers, RSS_HEADER_ETAG)
        .or_else(|| previous_state.last_etag.as_deref())
        .map(ToString::to_string);
    let last_modified = header_value(&response.headers, RSS_HEADER_LAST_MODIFIED)
        .or_else(|| previous_state.last_modified.as_deref())
        .map(ToString::to_string);
    let next_eligible_fetch_at =
        update_interval_minutes
            .filter(|minutes| *minutes > 0)
            .map(|minutes| {
                response
                    .response_at
                    .saturating_add((minutes as i64).saturating_mul(60))
            });

    RssRefreshState {
        last_fetched_at: Some(response.response_at),
        last_etag,
        last_modified,
        next_eligible_fetch_at,
        not_modified: response.status_code == 304,
    }
}

impl RssLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_subscription(
        &mut self,
        subscription: RssSubscription,
    ) -> Result<RssSubscription, RssError> {
        validate_subscription_fields(&subscription.subscription_id, &subscription.feed_url)?;
        self.subscriptions
            .insert(subscription.subscription_id.clone(), subscription.clone());
        Ok(subscription)
    }

    pub fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<RssSubscription>, RssError> {
        validate_subscription_id(subscription_id)?;
        Ok(self.subscriptions.get(subscription_id).cloned())
    }

    pub fn list_subscriptions(&self) -> Vec<RssSubscription> {
        let mut subscriptions = self.subscriptions.values().cloned().collect::<Vec<_>>();
        subscriptions.sort_by(|a, b| {
            a.title
                .cmp(&b.title)
                .then_with(|| a.subscription_id.cmp(&b.subscription_id))
        });
        subscriptions
    }

    pub fn remove_subscription(&mut self, subscription_id: &str) -> Result<usize, RssError> {
        validate_subscription_id(subscription_id)?;
        self.subscriptions.remove(subscription_id);
        let before = self.entries.len();
        self.entries
            .retain(|key, _| key.subscription_id != subscription_id);
        Ok(before - self.entries.len())
    }

    pub fn refresh_subscription(
        &mut self,
        subscription_id: &str,
        feed: &RssFeed,
        fetched_at: i64,
    ) -> Result<RssRefreshResult, RssError> {
        validate_subscription_id(subscription_id)?;
        let subscription = self
            .subscriptions
            .get(subscription_id)
            .cloned()
            .ok_or_else(|| RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            })?;
        let before_entry_ids = self
            .entries
            .keys()
            .filter(|key| key.subscription_id == subscription_id)
            .map(|key| key.entry_id.clone())
            .collect::<HashSet<_>>();

        let mut updated_subscription = subscription;
        updated_subscription.apply_feed(feed, fetched_at)?;

        let mut actual_new_entries = Vec::new();
        for entry in &feed.entries {
            let key = RssEntryKey {
                subscription_id: subscription_id.to_string(),
                entry_id: entry.id.clone(),
            };
            if let Some(state) = self.entries.get_mut(&key) {
                state.entry = entry.clone();
            } else {
                actual_new_entries.push(entry.clone());
                self.entries.insert(
                    key,
                    RssEntryState {
                        subscription_id: subscription_id.to_string(),
                        entry: entry.clone(),
                        first_seen_at: fetched_at,
                        read: false,
                        read_at: None,
                        starred: false,
                    },
                );
            }
        }

        updated_subscription.unread_count = self.unread_count(subscription_id);
        self.subscriptions
            .insert(subscription_id.to_string(), updated_subscription.clone());

        Ok(RssRefreshResult {
            subscription: updated_subscription,
            new_entries: actual_new_entries
                .into_iter()
                .filter(|entry| !before_entry_ids.contains(&entry.id))
                .collect(),
        })
    }

    pub fn list_entries(&self, subscription_id: &str) -> Result<Vec<RssEntryState>, RssError> {
        validate_subscription_id(subscription_id)?;
        if !self.subscriptions.contains_key(subscription_id) {
            return Err(RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            });
        }
        let mut entries = self
            .entries
            .values()
            .filter(|state| state.subscription_id == subscription_id)
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            b.first_seen_at
                .cmp(&a.first_seen_at)
                .then_with(|| a.entry.id.cmp(&b.entry.id))
        });
        Ok(entries)
    }

    pub fn mark_entry_read(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
        read_at: i64,
    ) -> Result<RssEntryState, RssError> {
        self.update_entry(subscription_id, entry_id, |state| {
            state.read = true;
            state.read_at = Some(read_at);
        })
    }

    pub fn mark_entry_unread(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
    ) -> Result<RssEntryState, RssError> {
        self.update_entry(subscription_id, entry_id, |state| {
            state.read = false;
            state.read_at = None;
        })
    }

    pub fn mark_all_read(&mut self, subscription_id: &str, read_at: i64) -> Result<(), RssError> {
        validate_subscription_id(subscription_id)?;
        if !self.subscriptions.contains_key(subscription_id) {
            return Err(RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            });
        }
        for state in self.entries.values_mut() {
            if state.subscription_id == subscription_id {
                state.read = true;
                state.read_at = Some(read_at);
            }
        }
        self.recompute_unread_count(subscription_id);
        Ok(())
    }

    pub fn set_entry_starred(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
        starred: bool,
    ) -> Result<RssEntryState, RssError> {
        self.update_entry(subscription_id, entry_id, |state| {
            state.starred = starred;
        })
    }

    fn update_entry(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
        update: impl FnOnce(&mut RssEntryState),
    ) -> Result<RssEntryState, RssError> {
        validate_subscription_id(subscription_id)?;
        validate_entry_id(entry_id)?;
        if !self.subscriptions.contains_key(subscription_id) {
            return Err(RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            });
        }
        let key = RssEntryKey {
            subscription_id: subscription_id.to_string(),
            entry_id: entry_id.to_string(),
        };
        let state = self
            .entries
            .get_mut(&key)
            .ok_or_else(|| RssError::EntryNotFound {
                subscription_id: subscription_id.to_string(),
                entry_id: entry_id.to_string(),
            })?;
        update(state);
        let state = state.clone();
        self.recompute_unread_count(subscription_id);
        Ok(state)
    }

    fn recompute_unread_count(&mut self, subscription_id: &str) {
        let unread_count = self.unread_count(subscription_id);
        if let Some(subscription) = self.subscriptions.get_mut(subscription_id) {
            subscription.unread_count = unread_count;
        }
    }

    fn unread_count(&self, subscription_id: &str) -> u32 {
        self.entries
            .values()
            .filter(|state| state.subscription_id == subscription_id && !state.read)
            .count() as u32
    }
}

/// Export/import surface for RSS library state.
pub trait RssLibrarySnapshotStore {
    fn export_snapshot(&self, exported_at: i64) -> Result<RssLibrarySnapshot, RssError>;

    fn replace_with_snapshot(&mut self, snapshot: RssLibrarySnapshot) -> Result<(), RssError>;
}

impl RssLibrarySnapshotStore for RssLibrary {
    fn export_snapshot(&self, exported_at: i64) -> Result<RssLibrarySnapshot, RssError> {
        let mut snapshot = RssLibrarySnapshot {
            schema_version: RSS_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            subscriptions: self.subscriptions.values().cloned().collect(),
            entries: self.entries.values().cloned().collect(),
        };
        sort_rss_snapshot(&mut snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn replace_with_snapshot(&mut self, snapshot: RssLibrarySnapshot) -> Result<(), RssError> {
        snapshot.validate()?;
        let RssLibrarySnapshot {
            subscriptions: snapshot_subscriptions,
            entries: snapshot_entries,
            ..
        } = snapshot;
        let mut subscriptions = HashMap::new();
        let mut entries = HashMap::new();

        for mut subscription in snapshot_subscriptions {
            subscription.unread_count = snapshot_unread_count(&snapshot_entries, &subscription);
            subscriptions.insert(subscription.subscription_id.clone(), subscription);
        }
        for state in snapshot_entries {
            entries.insert(
                RssEntryKey {
                    subscription_id: state.subscription_id.clone(),
                    entry_id: state.entry.id.clone(),
                },
                state,
            );
        }

        self.subscriptions = subscriptions;
        self.entries = entries;
        Ok(())
    }
}

fn sort_rss_snapshot(snapshot: &mut RssLibrarySnapshot) {
    snapshot.subscriptions.sort_by(|a, b| {
        a.subscription_id
            .cmp(&b.subscription_id)
            .then_with(|| a.feed_url.cmp(&b.feed_url))
    });
    snapshot.entries.sort_by(|a, b| {
        a.subscription_id
            .cmp(&b.subscription_id)
            .then_with(|| a.entry.id.cmp(&b.entry.id))
    });
}

fn snapshot_unread_count(entries: &[RssEntryState], subscription: &RssSubscription) -> u32 {
    entries
        .iter()
        .filter(|state| state.subscription_id == subscription.subscription_id && !state.read)
        .count() as u32
}

fn default_true() -> bool {
    true
}

fn default_explore_page() -> u32 {
    1
}

fn rss_runtime_capability_wire_value(capability: RssRuntimeCapability) -> &'static str {
    match capability {
        RssRuntimeCapability::CookieJar => "cookieJar",
        RssRuntimeCapability::Login => "login",
        RssRuntimeCapability::Javascript => "javascript",
        RssRuntimeCapability::WebView => "webView",
    }
}

fn rss_source_capability_requirement(
    capability: RssRuntimeCapability,
) -> RssSourceCapabilityRequirement {
    match capability {
        RssRuntimeCapability::CookieJar => RssSourceCapabilityRequirement::CookieJar,
        RssRuntimeCapability::Login => RssSourceCapabilityRequirement::Login,
        RssRuntimeCapability::Javascript => RssSourceCapabilityRequirement::Javascript,
        RssRuntimeCapability::WebView => RssSourceCapabilityRequirement::WebView,
    }
}

fn rss_rule_text_requires_javascript(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("@js:")
        || value.contains("<js>")
        || value.contains("java.")
        || value.contains("document.")
        || value.contains("window.")
        || value.contains("webview")
}

const RSS_SOURCE_CONFIG_KEYS: &[&str] = &[
    "url",
    "sourceUrl",
    "name",
    "sourceName",
    "updateIntervalMinutes",
    "lastFetchedAt",
    "lastETag",
    "lastModified",
    "enabled",
    "sourceIcon",
    "sourceGroup",
    "sourceComment",
    "variableComment",
    "jsLib",
    "enabledCookieJar",
    "concurrentRate",
    "header",
    "loginUrl",
    "loginUi",
    "loginCheckJs",
    "coverDecodeJs",
    "sortUrl",
    "singleUrl",
    "articleStyle",
    "ruleArticles",
    "ruleNextPage",
    "ruleTitle",
    "rulePubDate",
    "ruleDescription",
    "ruleImage",
    "ruleLink",
    "ruleContent",
    "contentWhitelist",
    "contentBlacklist",
    "shouldOverrideUrlLoading",
    "style",
    "enableJs",
    "loadWithBaseUrl",
    "injectJs",
    "lastUpdateTime",
    "customOrder",
];

fn is_rss_source_config_key(key: &str) -> bool {
    RSS_SOURCE_CONFIG_KEYS.contains(&key)
}

fn rss_source_projection_id(source: &RssSourceConfig, source_id: Option<&str>) -> String {
    non_empty_trimmed(source_id)
        .or_else(|| non_empty_trimmed(source.name.as_deref()))
        .unwrap_or_else(|| source.url.trim())
        .to_string()
}

fn source_root_url(url: &str) -> Option<String> {
    let url = non_empty_trimmed(Some(url))?;
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    if scheme.is_empty() {
        return None;
    }
    let remainder = &url[scheme_end + 3..];
    let authority_end = remainder
        .find(|ch| matches!(ch, '/' | '?' | '#'))
        .unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    if host_port.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host_port}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RssReplaceRule {
    Literal {
        from: String,
        to: String,
    },
    RegularExpression {
        pattern: String,
        template: String,
        case_insensitive: bool,
        multi_line: bool,
        dot_matches_new_line: bool,
    },
}

fn parse_rss_replace_rule(raw_rule: &str) -> Option<RssReplaceRule> {
    let rule = raw_rule.trim();
    if rule.is_empty() {
        return None;
    }
    if let Some(regex_rule) = parse_rss_replace_function(rule) {
        return Some(regex_rule);
    }
    let (from, to) = rule.split_once("=>")?;
    if from.is_empty() {
        return None;
    }
    Some(RssReplaceRule::Literal {
        from: from.to_string(),
        to: to.to_string(),
    })
}

fn parse_rss_replace_function(rule: &str) -> Option<RssReplaceRule> {
    let inner = rule.strip_prefix("replace(")?.strip_suffix(')')?.trim();
    let inner = inner.strip_prefix('/')?;
    let (pattern, remainder) = split_regex_pattern_and_remainder(inner)?;
    let (flags, template) = remainder.trim().split_once(',')?;
    let template = parse_rss_quoted_argument(template)?;
    Some(RssReplaceRule::RegularExpression {
        pattern,
        template,
        case_insensitive: flags.trim().contains('i'),
        multi_line: flags.trim().contains('m'),
        dot_matches_new_line: flags.trim().contains('s'),
    })
}

fn split_regex_pattern_and_remainder(input: &str) -> Option<(String, &str)> {
    let mut pattern = String::new();
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if escaped {
            pattern.push(ch);
            escaped = false;
        } else if ch == '\\' {
            pattern.push(ch);
            escaped = true;
        } else if ch == '/' {
            return Some((pattern, &input[index + ch.len_utf8()..]));
        } else {
            pattern.push(ch);
        }
    }
    None
}

fn parse_rss_quoted_argument(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let mut chars = trimmed.chars();
    let quote = chars.next()?;
    if !matches!(quote, '"' | '\'') {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return Some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

fn normalize_subscription_items(items: &mut [RssSubscriptionItem]) {
    for item in items {
        item.title = item.title.trim().to_string();
        item.link = item.link.trim().to_string();
        item.author = item.author.as_deref().map(str::trim).map(str::to_string);
        item.summary = item.summary.as_deref().map(str::trim).map(str::to_string);
    }
}

fn insert_optional_json<T: Serialize>(
    fields: &mut BTreeMap<String, serde_json::Value>,
    key: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        fields.insert(key.to_string(), serde_json::json!(value));
    }
}

fn optional_string_alias<E>(
    fields: &BTreeMap<String, serde_json::Value>,
    primary_key: &str,
    alias_key: &str,
) -> Result<Option<String>, E>
where
    E: serde::de::Error,
{
    optional_string(fields, primary_key)?.map_or_else(
        || optional_string(fields, alias_key),
        |value| Ok(Some(value)),
    )
}

fn optional_string<E>(
    fields: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, E>
where
    E: serde::de::Error,
{
    optional_value(fields, key, |value| {
        value
            .as_str()
            .map(ToString::to_string)
            .ok_or_else(|| E::custom(format!("expected string for RSS source field {key}")))
    })
}

fn optional_bool<E>(
    fields: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<Option<bool>, E>
where
    E: serde::de::Error,
{
    optional_value(fields, key, |value| {
        value
            .as_bool()
            .ok_or_else(|| E::custom(format!("expected bool for RSS source field {key}")))
    })
}

fn optional_i64<E>(
    fields: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<Option<i64>, E>
where
    E: serde::de::Error,
{
    optional_value(fields, key, |value| {
        value
            .as_i64()
            .ok_or_else(|| E::custom(format!("expected i64 for RSS source field {key}")))
    })
}

fn optional_i32<E>(
    fields: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<Option<i32>, E>
where
    E: serde::de::Error,
{
    optional_value(fields, key, |value| {
        let value = value
            .as_i64()
            .ok_or_else(|| E::custom(format!("expected i32 for RSS source field {key}")))?;
        i32::try_from(value)
            .map_err(|_| E::custom(format!("i32 out of range for RSS source field {key}")))
    })
}

fn optional_u32<E>(
    fields: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<Option<u32>, E>
where
    E: serde::de::Error,
{
    optional_value(fields, key, |value| {
        let value = value
            .as_i64()
            .ok_or_else(|| E::custom(format!("expected u32 for RSS source field {key}")))?;
        u32::try_from(value)
            .map_err(|_| E::custom(format!("u32 out of range for RSS source field {key}")))
    })
}

fn optional_value<T, E, F>(
    fields: &BTreeMap<String, serde_json::Value>,
    key: &str,
    parse: F,
) -> Result<Option<T>, E>
where
    E: serde::de::Error,
    F: FnOnce(&serde_json::Value) -> Result<T, E>,
{
    match fields.get(key) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => parse(value).map(Some),
    }
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| non_empty_trimmed(Some(value.as_str())))
}

fn non_empty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Parse an RSS 2.0 or Atom feed from an already-fetched XML string.
pub fn parse_feed(xml: &str) -> Result<RssFeed, RssError> {
    parse_feed_inner(xml, None)
}

/// Parse a feed and attach the caller-known feed URL to the result.
pub fn parse_feed_with_url(feed_url: &str, xml: &str) -> Result<RssFeed, RssError> {
    let feed_url = feed_url.trim();
    if feed_url.is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "feed_url".into(),
        });
    }
    parse_feed_inner(xml, Some(feed_url.to_string()))
}

/// Parse RSS/Atom XML pagination metadata without fetching a next page.
pub fn plan_xml_feed_pagination(
    xml: &str,
    current_feed_url: Option<&str>,
) -> Result<RssXmlPaginationPlan, RssError> {
    let current_feed_url = current_feed_url.and_then(|url| non_empty_trimmed(Some(url)));
    parse_xml_feed_pagination_inner(xml, current_feed_url)
}

pub fn parse_json_feed_page(
    json: &str,
    source_url: Option<&str>,
    source_name: Option<&str>,
) -> Result<RssJsonFeedPage, RssError> {
    let source_url = source_url.and_then(|url| non_empty_trimmed(Some(url)));
    let source_name = source_name.and_then(|name| non_empty_trimmed(Some(name)));
    parse_json_feed_page_inner(json, source_url, source_name)
}

pub fn parse_subscription_items(
    feed_xml: &str,
    source: &RssSubscriptionSourceConfig,
    limit: Option<usize>,
) -> Result<Vec<RssSubscriptionItem>, RssError> {
    source.validate()?;
    let feed = parse_feed_with_url(&source.url, feed_xml)?;
    let mut items = feed
        .entries
        .iter()
        .map(|entry| RssSubscriptionItem::from_entry(entry, source.id.clone(), source.name.clone()))
        .collect::<Vec<_>>();
    if let Some(replace_rules) = source.replace_rules.as_ref() {
        items = apply_rss_subscription_replace_rules(&items, replace_rules);
    }
    normalize_subscription_items(&mut items);
    if let Some(limit) = limit {
        items.truncate(limit);
    }
    Ok(items)
}

pub fn parse_rss_rule_html(
    html: &str,
    source: &RssSourceConfig,
    source_id: &str,
    source_name: Option<&str>,
    base_url: Option<&str>,
    limit: Option<usize>,
) -> Result<RssRuleParseResult, RssError> {
    let unsupported = unsupported_rss_rule_runtime_capabilities(source);
    if !unsupported.is_empty() {
        return Err(RssError::UnsupportedRuntimeCapabilities {
            capabilities: unsupported,
        });
    }
    let rule_articles = non_empty_trimmed(source.rule_articles.as_deref()).ok_or_else(|| {
        RssError::MissingField {
            field: "rss_rule.rule_articles".into(),
        }
    })?;
    let source_id = non_empty_trimmed(Some(source_id)).ok_or_else(|| RssError::MissingField {
        field: "rss_rule.source_id".into(),
    })?;
    let source_name = source_name
        .and_then(|name| non_empty_trimmed(Some(name)))
        .map(ToString::to_string);
    let base_url = base_url.and_then(|url| non_empty_trimmed(Some(url)));
    let input = html.trim();
    if input.is_empty() {
        return Err(RssError::EmptyInput);
    }

    let document = Html::parse_document(input);
    let (articles_rule, reverse) = normalized_rss_articles_rule(rule_articles);
    let article_selector = rss_rule_selector(&articles_rule)?;
    let mut article_elements = document.select(&article_selector).collect::<Vec<_>>();
    if reverse {
        article_elements.reverse();
    }

    let mut items = Vec::new();
    for element in article_elements {
        if let Some(item) =
            parse_rss_rule_item(element, source, source_id, source_name.clone(), base_url)?
        {
            items.push(item);
        }
        if limit.is_some_and(|limit| items.len() >= limit) {
            break;
        }
    }

    let next_page_url = rss_rule_next_page_url(&document, source, base_url)?;
    Ok(RssRuleParseResult {
        items,
        next_page_url,
    })
}

pub fn apply_rss_subscription_replace_rules(
    items: &[RssSubscriptionItem],
    rules: &[String],
) -> Vec<RssSubscriptionItem> {
    items
        .iter()
        .cloned()
        .map(|mut item| {
            item.title = apply_rss_replace_rules(&item.title, rules)
                .trim()
                .to_string();
            item.summary = item
                .summary
                .map(|summary| apply_rss_replace_rules(&summary, rules).trim().to_string());
            item.author = item
                .author
                .map(|author| apply_rss_replace_rules(&author, rules).trim().to_string());
            item
        })
        .collect()
}

pub fn apply_rss_replace_rules(text: &str, rules: &[String]) -> String {
    let mut result = text.to_string();
    for rule in rules {
        match parse_rss_replace_rule(rule) {
            Some(RssReplaceRule::Literal { from, to }) => {
                result = result.replace(&from, &to);
            }
            Some(RssReplaceRule::RegularExpression {
                pattern,
                template,
                case_insensitive,
                multi_line,
                dot_matches_new_line,
            }) => {
                let Ok(regex) = RegexBuilder::new(&pattern)
                    .case_insensitive(case_insensitive)
                    .multi_line(multi_line)
                    .dot_matches_new_line(dot_matches_new_line)
                    .build()
                else {
                    continue;
                };
                result = regex.replace_all(&result, template.as_str()).into_owned();
            }
            None => {}
        }
    }
    result
}

fn parse_explore_html_item(
    container: ElementRef<'_>,
    rule: &ExploreHtmlParseRule,
    base_url: Option<&str>,
) -> Result<Option<ExploreHtmlItem>, RssError> {
    let Some(title) = rss_rule_first_value(container, rule.name.as_deref(), Some("text"))? else {
        return Ok(None);
    };
    let Some(book_url) = rss_rule_first_value(container, rule.book_url.as_deref(), Some("href"))?
        .map(|url| resolve_rss_rule_url(&url, base_url))
    else {
        return Ok(None);
    };
    let author = rss_rule_first_value(container, rule.author.as_deref(), Some("text"))?
        .and_then(|value| trim_explore_label_prefix(value, &["作者"]));
    let cover_url = rss_rule_first_value(container, rule.cover_url.as_deref(), Some("src"))?
        .map(|url| resolve_rss_rule_url(&url, base_url));
    let intro = rss_rule_first_value(container, rule.intro.as_deref(), Some("text"))?;
    let last_chapter = rss_rule_first_value(container, rule.last_chapter.as_deref(), Some("text"))?
        .and_then(|value| trim_explore_label_prefix(value, &["最新章节"]));
    let update_time = rss_rule_first_value(container, rule.update_time.as_deref(), Some("text"))?
        .and_then(|value| trim_explore_label_prefix(value, &["更新时间"]));
    let raw_tags = rss_rule_first_value(container, rule.tags.as_deref(), Some("text"))?;
    let tags = raw_tags
        .as_deref()
        .map(parse_explore_tags)
        .unwrap_or_default();
    let kind = rss_rule_first_value(container, rule.kind.as_deref(), Some("text"))?
        .or_else(|| tags.first().cloned());
    let raw_fields = [
        ("bookList", rule.book_list.as_ref()),
        ("name", rule.name.as_ref()),
        ("bookUrl", rule.book_url.as_ref()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key.to_string(), value.clone())))
    .collect::<BTreeMap<_, _>>();

    Ok(Some(ExploreHtmlItem {
        id: stable_explore_item_id(&rule.source_id, &book_url),
        title,
        book_url,
        author,
        cover_url,
        intro,
        last_chapter,
        kind,
        update_time,
        tags,
        raw_fields,
    }))
}

fn explore_html_next_page(
    document: &Html,
    rule: &ExploreHtmlParseRule,
    base_url: Option<&str>,
) -> Result<(bool, Option<String>), RssError> {
    if let Some(next_page) = rule
        .next_page
        .as_deref()
        .and_then(|value| non_empty_trimmed(Some(value)))
    {
        if next_page.eq_ignore_ascii_case("PAGE") {
            return Ok((base_url.is_some(), base_url.map(ToString::to_string)));
        }
        let next_page_url = rss_rule_first_value_in_document(document, next_page, Some("href"))?
            .map(|url| resolve_rss_rule_url(&url, base_url));
        return Ok((next_page_url.is_some(), next_page_url));
    }

    let fallback_selector = parse_scraper_selector(".next-link")?;
    let has_next_page = document
        .select(&fallback_selector)
        .next()
        .is_some_and(|element| {
            element
                .value()
                .attr("href")
                .and_then(normalize_rss_rule_value)
                .is_some()
                || normalize_rss_rule_value(&element.text().collect::<Vec<_>>().join(" ")).is_some()
        });
    Ok((has_next_page, None))
}

fn parse_rss_rule_item(
    container: ElementRef<'_>,
    source: &RssSourceConfig,
    source_id: &str,
    source_name: Option<String>,
    base_url: Option<&str>,
) -> Result<Option<RssSubscriptionItem>, RssError> {
    let Some(title) = rss_rule_first_value(container, source.rule_title.as_deref(), Some("text"))?
    else {
        return Ok(None);
    };
    let link = rss_rule_first_value(container, source.rule_link.as_deref(), Some("href"))?
        .map(|link| resolve_rss_rule_url(&link, base_url))
        .unwrap_or_else(|| synthetic_rss_rule_link(source_id, &title));
    let summary =
        rss_rule_first_value(container, source.rule_description.as_deref(), Some("text"))?;
    let content = rss_rule_first_value(container, source.rule_content.as_deref(), Some("html"))?;
    let image = rss_rule_first_value(container, source.rule_image.as_deref(), Some("src"))?
        .map(|image| resolve_rss_rule_url(&image, base_url));
    let published_at =
        rss_rule_first_value(container, source.rule_pub_date.as_deref(), Some("text"))?;

    let mut item = RssSubscriptionItem::new(title, link);
    item.source_id = source_id.to_string();
    item.source_name = source_name;
    item.summary = summary;
    item.published_at = published_at.clone();
    if let Some(content) = content {
        item.unknown_fields
            .insert("content".into(), serde_json::json!(content));
    }
    if let Some(image) = image {
        item.unknown_fields
            .insert("image".into(), serde_json::json!(image));
    }
    if let Some(published_at) = published_at {
        item.unknown_fields
            .insert("pubDateRaw".into(), serde_json::json!(published_at));
    }
    if let Some(group) = non_empty_trimmed(source.source_group.as_deref()) {
        item.unknown_fields
            .insert("group".into(), serde_json::json!(group));
    }
    item.unknown_fields.insert(
        "rssRuleMode".into(),
        serde_json::json!("non_js_rule_articles"),
    );
    item.validate()?;
    Ok(Some(item))
}

fn rss_rule_next_page_url(
    document: &Html,
    source: &RssSourceConfig,
    base_url: Option<&str>,
) -> Result<Option<String>, RssError> {
    let Some(rule) = non_empty_trimmed(source.rule_next_page.as_deref()) else {
        return Ok(None);
    };
    if rule.eq_ignore_ascii_case("PAGE") {
        return Ok(base_url.map(ToString::to_string));
    }
    Ok(
        rss_rule_first_value_in_document(document, rule, Some("href"))?
            .map(|url| resolve_rss_rule_url(&url, base_url)),
    )
}

fn rss_rule_first_value(
    scope: ElementRef<'_>,
    raw_rule: Option<&str>,
    default_attribute: Option<&str>,
) -> Result<Option<String>, RssError> {
    let Some(raw_rule) = raw_rule.and_then(|rule| non_empty_trimmed(Some(rule))) else {
        return Ok(None);
    };
    let rule = parse_rss_rule_extraction_rule(raw_rule, default_attribute)?;
    Ok(scope
        .select(&rule.selector)
        .next()
        .and_then(|element| rss_rule_extract_value(element, &rule.attribute)))
}

fn rss_rule_first_value_in_document(
    document: &Html,
    raw_rule: &str,
    default_attribute: Option<&str>,
) -> Result<Option<String>, RssError> {
    let rule = parse_rss_rule_extraction_rule(raw_rule, default_attribute)?;
    Ok(document
        .select(&rule.selector)
        .next()
        .and_then(|element| rss_rule_extract_value(element, &rule.attribute)))
}

struct RssRuleExtractionRule {
    selector: Selector,
    attribute: RssRuleAttribute,
}

enum RssRuleAttribute {
    Text,
    Html,
    Attribute(String),
}

fn parse_rss_rule_extraction_rule(
    raw_rule: &str,
    default_attribute: Option<&str>,
) -> Result<RssRuleExtractionRule, RssError> {
    let normalized = normalized_rss_rule_selector_text(raw_rule);
    let (selector_text, attribute) =
        if let Some((selector, attribute)) = normalized.rsplit_once('@') {
            (selector.trim(), Some(attribute.trim()))
        } else {
            (normalized.as_str(), default_attribute)
        };
    let selector = parse_scraper_selector(selector_text)?;
    let attribute = match attribute
        .and_then(|attribute| non_empty_trimmed(Some(attribute)))
        .unwrap_or("text")
    {
        "text" => RssRuleAttribute::Text,
        "html" => RssRuleAttribute::Html,
        other => RssRuleAttribute::Attribute(other.to_string()),
    };
    Ok(RssRuleExtractionRule {
        selector,
        attribute,
    })
}

fn rss_rule_selector(raw_rule: &str) -> Result<Selector, RssError> {
    let normalized = normalized_rss_rule_selector_text(raw_rule);
    let selector = normalized
        .rsplit_once('@')
        .map(|(selector, _)| selector.trim())
        .unwrap_or(normalized.as_str());
    parse_scraper_selector(selector)
}

fn parse_scraper_selector(selector: &str) -> Result<Selector, RssError> {
    let selector =
        non_empty_trimmed(Some(selector)).ok_or_else(|| RssError::InvalidSubscription {
            field: "rss_rule.selector".into(),
        })?;
    Selector::parse(selector).map_err(|_| RssError::InvalidSubscription {
        field: "rss_rule.selector".into(),
    })
}

fn normalized_rss_articles_rule(rule: &str) -> (String, bool) {
    let trimmed = rule.trim();
    trimmed
        .strip_prefix('-')
        .map(|rule| (rule.trim().to_string(), true))
        .unwrap_or_else(|| (trimmed.to_string(), false))
}

fn normalized_rss_rule_selector_text(selector: &str) -> String {
    let trimmed = selector.trim();
    trimmed
        .strip_prefix("css:")
        .map(str::trim)
        .unwrap_or(trimmed)
        .to_string()
}

fn rss_rule_extract_value(element: ElementRef<'_>, attribute: &RssRuleAttribute) -> Option<String> {
    let raw = match attribute {
        RssRuleAttribute::Text => element.text().collect::<Vec<_>>().join(" "),
        RssRuleAttribute::Html => element.inner_html(),
        RssRuleAttribute::Attribute(attribute) => element.value().attr(attribute)?.to_string(),
    };
    normalize_rss_rule_value(&raw)
}

fn normalize_rss_rule_value(raw: &str) -> Option<String> {
    let decoded = decode_xml_entities(raw);
    let normalized = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn trim_explore_label_prefix(value: String, labels: &[&str]) -> Option<String> {
    let mut value = value.trim().to_string();
    for label in labels {
        for delimiter in [":", "："] {
            let prefix = format!("{label}{delimiter}");
            if value.starts_with(&prefix) {
                value = value[prefix.len()..].trim().to_string();
            }
        }
    }
    (!value.is_empty()).then_some(value)
}

fn parse_explore_tags(value: &str) -> Vec<String> {
    value
        .split([',', '，'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn stable_explore_item_id(source_id: &str, book_url: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in format!("{source_id}|{book_url}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn resolve_rss_rule_url(value: &str, base_url: Option<&str>) -> String {
    let value = value.trim();
    if value.is_empty() || is_absolute_rss_rule_url(value) {
        return value.to_string();
    }
    let Some(base_url) = base_url.and_then(|base_url| non_empty_trimmed(Some(base_url))) else {
        return value.to_string();
    };
    if let Some(rest) = value.strip_prefix("//") {
        if let Some((scheme, _)) = base_url.split_once("://") {
            return format!("{scheme}://{rest}");
        }
        return value.to_string();
    }
    if value.starts_with('/') {
        if let Some(origin) = rss_rule_url_origin(base_url) {
            return format!("{origin}{value}");
        }
        return value.to_string();
    }
    if value.starts_with('?') {
        return format!(
            "{}{}",
            rss_rule_url_without_query_or_fragment(base_url),
            value
        );
    }
    let base = rss_rule_url_directory(base_url);
    format!("{base}{value}")
}

fn is_absolute_rss_rule_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("rss-rule://")
        || lower.starts_with("urn:")
        || lower.starts_with("mailto:")
}

fn rss_rule_url_origin(base_url: &str) -> Option<String> {
    let (scheme, rest) = base_url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

fn rss_rule_url_without_query_or_fragment(base_url: &str) -> String {
    let end = base_url.find(['?', '#']).unwrap_or(base_url.len());
    base_url[..end].to_string()
}

fn rss_rule_url_directory(base_url: &str) -> String {
    let clean = rss_rule_url_without_query_or_fragment(base_url);
    if clean.ends_with('/') {
        return clean;
    }
    match clean.rsplit_once('/') {
        Some((prefix, _)) if prefix.contains("://") => format!("{prefix}/"),
        _ => format!("{clean}/"),
    }
}

fn synthetic_rss_rule_link(source_id: &str, title: &str) -> String {
    let material = format!("{source_id}|{title}");
    let mut hash = 14_695_981_039_346_656_037u64;
    for ch in material.chars() {
        hash ^= u64::from(ch as u32);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("rss-rule://{hash:x}")
}

fn parse_feed_inner(xml: &str, provided_feed_url: Option<String>) -> Result<RssFeed, RssError> {
    let input = xml.trim();
    if input.is_empty() {
        return Err(RssError::EmptyInput);
    }

    if input.starts_with('{') {
        return parse_json_feed_page_inner(input, provided_feed_url.as_deref(), None)
            .map(|page| page.feed);
    }

    if has_element(xml, "rss") || has_element(xml, "channel") {
        parse_rss_feed(xml, provided_feed_url)
    } else if has_element(xml, "feed") {
        parse_atom_feed(xml, provided_feed_url)
    } else {
        Err(RssError::UnsupportedFormat)
    }
}

fn parse_xml_feed_pagination_inner(
    xml: &str,
    current_feed_url: Option<&str>,
) -> Result<RssXmlPaginationPlan, RssError> {
    let input = xml.trim();
    if input.is_empty() {
        return Err(RssError::EmptyInput);
    }
    if input.starts_with('{') {
        return Err(RssError::UnsupportedFormat);
    }

    let metadata = if has_element(xml, "rss") || has_element(xml, "channel") {
        let channel = first_element_body(xml, "channel").unwrap_or_else(|| xml.to_string());
        remove_element_blocks(&channel, "item")
    } else if has_element(xml, "feed") {
        let feed = first_element_body(xml, "feed").unwrap_or_else(|| xml.to_string());
        remove_element_blocks(&feed, "entry")
    } else {
        return Err(RssError::UnsupportedFormat);
    };

    let next_url = link_href_by_rel_local(&metadata, "next");
    let self_link = link_href_by_rel_local(&metadata, "self");
    let self_urls = [current_feed_url, self_link.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    let next_page_url = match next_url {
        Some(next_url)
            if self_urls
                .iter()
                .any(|self_url| urls_equivalent(self_url, &next_url)) =>
        {
            diagnostics.push("pagination_next_url_rejected:self_reference".into());
            None
        }
        Some(next_url) => {
            diagnostics.push("pagination_next_url_detected:xml".into());
            diagnostics.push("pagination_metadata_parse_only:no_network_fetch".into());
            Some(next_url)
        }
        None => None,
    };

    Ok(RssXmlPaginationPlan {
        next_page_url,
        diagnostics,
        network_fetch_executed: false,
    })
}

fn parse_rss_feed(xml: &str, feed_url: Option<String>) -> Result<RssFeed, RssError> {
    let channel = first_element_body(xml, "channel").unwrap_or_else(|| xml.to_string());
    let channel_metadata = remove_element_blocks(&channel, "item");
    let title = required_text(&channel_metadata, "title", "feed.title")?;
    let site_url = first_text(&channel_metadata, "link");
    let description = first_text(&channel_metadata, "description");

    let mut entries = element_bodies(&channel, "item")
        .into_iter()
        .map(|item| parse_rss_item(&item))
        .collect::<Result<Vec<_>, _>>()?;
    dedupe_entries(&mut entries);

    Ok(RssFeed {
        title,
        feed_url,
        site_url,
        description,
        entries,
    })
}

fn parse_rss_item(item: &str) -> Result<RssEntry, RssError> {
    let title = first_text(item, "title").unwrap_or_default();
    let link = first_text(item, "link");
    let guid = first_text(item, "guid");
    let content = first_xml_character_text(item, "content:encoded");
    let id = guid
        .clone()
        .or_else(|| link.clone())
        .or_else(|| (!title.is_empty()).then(|| title.clone()))
        .ok_or_else(|| RssError::MissingField {
            field: "entry.id".into(),
        })?;
    Ok(RssEntry {
        id,
        title,
        link,
        summary: first_summary_text(item, "description")
            .or_else(|| first_summary_text(item, "content:encoded")),
        published_at: first_text(item, "pubDate").or_else(|| first_text(item, "dc:date")),
        unknown_fields: xml_item_unknown_fields(item, guid.as_deref(), content.as_deref()),
    })
}

fn parse_atom_feed(xml: &str, provided_feed_url: Option<String>) -> Result<RssFeed, RssError> {
    let feed = first_element_body(xml, "feed").unwrap_or_else(|| xml.to_string());
    let feed_metadata = remove_element_blocks(&feed, "entry");
    let title = required_text(&feed_metadata, "title", "feed.title")?;
    let feed_url = provided_feed_url.or_else(|| link_href_by_rel(&feed_metadata, "self"));
    let site_url =
        link_href_by_rel(&feed_metadata, "alternate").or_else(|| first_link_href(&feed_metadata));
    let description = first_text(&feed_metadata, "subtitle");

    let mut entries = element_bodies(&feed, "entry")
        .into_iter()
        .map(|entry| parse_atom_entry(&entry))
        .collect::<Result<Vec<_>, _>>()?;
    dedupe_entries(&mut entries);

    Ok(RssFeed {
        title,
        feed_url,
        site_url,
        description,
        entries,
    })
}

fn parse_atom_entry(entry: &str) -> Result<RssEntry, RssError> {
    let title = first_text(entry, "title").unwrap_or_default();
    let link = link_href_by_rel(entry, "alternate").or_else(|| first_link_href(entry));
    let content = first_xml_character_text(entry, "content");
    let id = first_text(entry, "id")
        .or_else(|| link.clone())
        .or_else(|| (!title.is_empty()).then(|| title.clone()))
        .ok_or_else(|| RssError::MissingField {
            field: "entry.id".into(),
        })?;
    Ok(RssEntry {
        id,
        title,
        link,
        summary: first_summary_text(entry, "summary")
            .or_else(|| first_summary_text(entry, "content")),
        published_at: first_text(entry, "updated").or_else(|| first_text(entry, "published")),
        unknown_fields: xml_item_unknown_fields(entry, None, content.as_deref()),
    })
}

fn required_text(input: &str, tag: &str, field: &str) -> Result<String, RssError> {
    first_text(input, tag).ok_or_else(|| RssError::MissingField {
        field: field.into(),
    })
}

fn parse_json_feed_page_inner(
    json: &str,
    source_url: Option<&str>,
    source_name_override: Option<&str>,
) -> Result<RssJsonFeedPage, RssError> {
    if json.trim().is_empty() {
        return Err(RssError::EmptyInput);
    }

    let value = serde_json::from_str::<serde_json::Value>(json).map_err(|err| {
        RssError::InvalidJsonFeed {
            detail: err.to_string(),
        }
    })?;
    let object = value.as_object().ok_or(RssError::UnsupportedFormat)?;
    let raw_items = object
        .get("items")
        .and_then(serde_json::Value::as_array)
        .ok_or(RssError::UnsupportedFormat)?;

    let feed_url =
        json_string_field(object, "feed_url").or_else(|| source_url.map(ToString::to_string));
    let feed_title = source_name_override
        .map(ToString::to_string)
        .or_else(|| json_string_field(object, "title"))
        .or_else(|| feed_url.clone())
        .ok_or_else(|| RssError::MissingField {
            field: "feed.title".into(),
        })?;
    let source_id = source_url
        .map(ToString::to_string)
        .or_else(|| feed_url.clone())
        .unwrap_or_default();
    let source_name = source_name_override
        .map(ToString::to_string)
        .or_else(|| json_string_field(object, "title"));
    let feed_author = json_feed_author_name(object.get("author"))
        .or_else(|| json_feed_authors_name(object.get("authors")));
    let feed_metadata = json_feed_metadata_fields(object);
    let fallback_link = source_id.clone();

    let mut entries = Vec::new();
    let mut items = Vec::new();
    for item in raw_items {
        let item_object = item.as_object().ok_or_else(|| RssError::InvalidJsonFeed {
            detail: "items must contain objects".into(),
        })?;
        let (entry, item) = json_feed_entry_and_item(
            item_object,
            &source_id,
            source_name.as_deref(),
            feed_author.as_deref(),
            &feed_metadata,
            &fallback_link,
        )?;
        entries.push(entry);
        items.push(item);
    }
    dedupe_entries(&mut entries);

    let mut diagnostics = vec!["json_feed_mapping:core".to_string()];
    let mut next_page_url = None;
    if let Some(next_url) = json_string_field(object, "next_url") {
        let is_self_reference = [feed_url.as_deref(), source_url]
            .into_iter()
            .flatten()
            .any(|url| url == next_url);
        if is_self_reference {
            diagnostics.push("pagination_next_url_rejected:self_reference".into());
        } else {
            next_page_url = Some(next_url);
            diagnostics.push("pagination_next_url_detected:json".into());
            diagnostics.push("pagination_metadata_parse_only:no_network_fetch".into());
        }
    }

    Ok(RssJsonFeedPage {
        feed: RssFeed {
            title: feed_title,
            feed_url,
            site_url: json_string_field(object, "home_page_url"),
            description: json_string_field(object, "description")
                .or_else(|| json_string_field(object, "user_comment")),
            entries,
        },
        items,
        next_page_url,
        diagnostics,
    })
}

fn json_feed_entry_and_item(
    object: &serde_json::Map<String, serde_json::Value>,
    source_id: &str,
    source_name: Option<&str>,
    feed_author: Option<&str>,
    feed_metadata: &BTreeMap<String, serde_json::Value>,
    fallback_link: &str,
) -> Result<(RssEntry, RssSubscriptionItem), RssError> {
    let item_id = json_string_field(object, "id");
    let url = json_string_field(object, "url");
    let external_url = json_string_field(object, "external_url");
    let link = url
        .clone()
        .or_else(|| external_url.clone())
        .or_else(|| item_id.clone())
        .or_else(|| (!fallback_link.is_empty()).then(|| fallback_link.to_string()))
        .ok_or_else(|| RssError::MissingField {
            field: "entry.id".into(),
        })?;
    let content_html = json_string_field(object, "content_html");
    let summary = json_string_field(object, "summary")
        .or_else(|| json_string_field(object, "content_text"))
        .or_else(|| content_html.as_deref().and_then(normalized_html_summary));
    let title = json_string_field(object, "title")
        .or_else(|| summary.clone())
        .or_else(|| item_id.clone())
        .unwrap_or_else(|| link.clone());
    let id = item_id.clone().unwrap_or_else(|| link.clone());
    let published_at = json_string_field(object, "date_published")
        .or_else(|| json_string_field(object, "date_modified"));

    let mut unknown_fields = BTreeMap::new();
    insert_json_string(&mut unknown_fields, "jsonFeedId", item_id.as_deref());
    insert_json_string(&mut unknown_fields, "jsonFeedURL", url.as_deref());
    insert_json_string(&mut unknown_fields, "externalURL", external_url.as_deref());
    insert_json_string(
        &mut unknown_fields,
        "datePublished",
        json_string_field(object, "date_published").as_deref(),
    );
    insert_json_string(
        &mut unknown_fields,
        "dateModified",
        json_string_field(object, "date_modified").as_deref(),
    );
    insert_json_string(
        &mut unknown_fields,
        "contentText",
        json_string_field(object, "content_text").as_deref(),
    );
    insert_json_string(&mut unknown_fields, "contentHTML", content_html.as_deref());
    if let Some(authors) = json_feed_author_names(object.get("authors")) {
        unknown_fields.insert("authors".into(), serde_json::json!(authors));
    }
    if let Some(author_metadata) = json_feed_object_metadata(object.get("author")) {
        unknown_fields.insert("authorMetadata".into(), author_metadata);
    }
    if let Some(authors_metadata) = json_feed_object_array_metadata(object.get("authors")) {
        unknown_fields.insert(
            "authorsMetadata".into(),
            serde_json::json!(authors_metadata),
        );
    }
    if let Some(tags) = json_string_array_field(object, "tags") {
        unknown_fields.insert("categories".into(), serde_json::json!(tags));
    }
    if let Some(image) =
        json_string_field(object, "image").or_else(|| json_string_field(object, "banner_image"))
    {
        unknown_fields.insert("image".into(), serde_json::json!(image));
    }
    insert_json_string(
        &mut unknown_fields,
        "bannerImage",
        json_string_field(object, "banner_image").as_deref(),
    );
    if let Some(attachments) = json_feed_attachments(object.get("attachments")) {
        unknown_fields.insert("attachments".into(), serde_json::json!(attachments));
    }
    if let Some(extensions) = json_feed_extension_fields(object) {
        unknown_fields.insert("extensions".into(), serde_json::json!(extensions));
    }
    for (key, value) in feed_metadata {
        unknown_fields
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }

    Ok((
        RssEntry {
            id,
            title: title.clone(),
            link: Some(link.clone()),
            summary: summary.clone(),
            published_at: published_at.clone(),
            unknown_fields: unknown_fields.clone(),
        },
        RssSubscriptionItem {
            title,
            link,
            author: json_feed_author_name(object.get("author"))
                .or_else(|| json_feed_authors_name(object.get("authors")))
                .or_else(|| feed_author.map(ToString::to_string)),
            summary,
            published_at,
            source_id: source_id.to_string(),
            source_name: source_name.map(ToString::to_string),
            unknown_fields,
        },
    ))
}

fn json_feed_metadata_fields(
    object: &serde_json::Map<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    let mut fields = BTreeMap::new();
    for (json_key, output_key) in [
        ("version", "feedVersion"),
        ("title", "feedTitle"),
        ("feed_url", "feedURL"),
        ("next_url", "feedNextURL"),
        ("home_page_url", "feedHomePageURL"),
        ("language", "feedLanguage"),
        ("icon", "feedIcon"),
        ("favicon", "feedFavicon"),
        ("description", "feedDescription"),
        ("user_comment", "feedUserComment"),
    ] {
        insert_json_string(
            &mut fields,
            output_key,
            json_string_field(object, json_key).as_deref(),
        );
    }
    if let Some(expired) = object.get("expired").and_then(serde_json::Value::as_bool) {
        fields.insert("feedExpired".into(), serde_json::json!(expired));
    }
    if let Some(hubs) = json_feed_json_value(object.get("hubs")) {
        fields.insert("feedHubs".into(), hubs);
    }
    if let Some(author_metadata) = json_feed_object_metadata(object.get("author")) {
        fields.insert("feedAuthorMetadata".into(), author_metadata);
    }
    if let Some(authors_metadata) = json_feed_object_array_metadata(object.get("authors")) {
        fields.insert(
            "feedAuthorsMetadata".into(),
            serde_json::json!(authors_metadata),
        );
    }
    if let Some(extensions) = json_feed_extension_fields(object) {
        fields.insert("feedExtensions".into(), serde_json::json!(extensions));
    }
    fields
}

fn json_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn json_string_array_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<Vec<String>> {
    let values = object.get(key)?.as_array()?;
    let values = values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn json_feed_author_name(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(value) => non_empty_trimmed(Some(value)).map(ToString::to_string),
        serde_json::Value::Object(object) => json_string_field(object, "name"),
        _ => None,
    }
}

fn json_feed_author_names(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let values = value?.as_array()?;
    let names = values
        .iter()
        .filter_map(|value| json_feed_author_name(Some(value)))
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some(names)
}

fn json_feed_authors_name(value: Option<&serde_json::Value>) -> Option<String> {
    json_feed_author_names(value).map(|names| names.join(", "))
}

fn json_feed_object_metadata(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    json_feed_json_object(value?.as_object()?)
}

fn json_feed_object_array_metadata(
    value: Option<&serde_json::Value>,
) -> Option<Vec<serde_json::Value>> {
    let objects = value?.as_array()?;
    let values = objects
        .iter()
        .filter_map(|value| json_feed_object_metadata(Some(value)))
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn json_feed_attachments(value: Option<&serde_json::Value>) -> Option<Vec<serde_json::Value>> {
    let attachments = value?.as_array()?;
    let attachments = attachments
        .iter()
        .filter_map(|attachment| {
            let object = attachment.as_object()?;
            json_feed_json_object(object)
        })
        .collect::<Vec<_>>();
    (!attachments.is_empty()).then_some(attachments)
}

fn json_feed_extension_fields(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<BTreeMap<String, serde_json::Value>> {
    let fields = object
        .iter()
        .filter(|(key, _)| key.starts_with('_'))
        .filter_map(|(key, value)| {
            json_feed_json_value(Some(value)).map(|value| (key.clone(), value))
        })
        .collect::<BTreeMap<_, _>>();
    (!fields.is_empty()).then_some(fields)
}

fn json_feed_json_object(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let fields = object
        .iter()
        .filter_map(|(key, value)| {
            json_feed_json_value(Some(value)).map(|value| (key.clone(), value))
        })
        .collect::<serde_json::Map<_, _>>();
    (!fields.is_empty()).then_some(serde_json::Value::Object(fields))
}

fn json_feed_json_value(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    match value? {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => {
            non_empty_trimmed(Some(value)).map(|value| serde_json::Value::String(value.to_string()))
        }
        serde_json::Value::Bool(value) => Some(serde_json::Value::Bool(*value)),
        serde_json::Value::Number(value) => Some(serde_json::Value::Number(value.clone())),
        serde_json::Value::Array(values) => {
            let values = values
                .iter()
                .filter_map(|value| json_feed_json_value(Some(value)))
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(serde_json::Value::Array(values))
        }
        serde_json::Value::Object(object) => json_feed_json_object(object),
    }
}

fn validate_explore_required(value: &str, field: &str) -> Result<(), RssError> {
    if value.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_explore_optional(value: &Option<String>, field: &str) -> Result<(), RssError> {
    if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
        return Err(RssError::InvalidSubscription {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_explore_string_map(
    value: &Option<BTreeMap<String, String>>,
    field: &str,
) -> Result<(), RssError> {
    if let Some(values) = value {
        for (key, value) in values {
            if key.trim().is_empty() || value.trim().is_empty() {
                return Err(RssError::InvalidSubscription {
                    field: field.into(),
                });
            }
        }
    }
    Ok(())
}

fn parse_explore_screen_json_array(raw: &str) -> Option<Vec<ExploreScreen>> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let array = value.as_array()?;
    Some(
        array
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                value
                    .as_object()
                    .and_then(|object| explore_screen_from_json_object(object, index))
            })
            .collect(),
    )
}

fn explore_screen_from_json_object(
    object: &serde_json::Map<String, serde_json::Value>,
    index: usize,
) -> Option<ExploreScreen> {
    let title =
        explore_screen_first_string(object, &["title", "name", "label"]).unwrap_or_default();
    let url = explore_screen_first_string(object, &["url", "exploreUrl", "urlTemplate"]);
    if non_empty_trimmed(Some(title.as_str())).is_none()
        && non_empty_trimmed(url.as_deref()).is_none()
    {
        return None;
    }

    let fallback_title = non_empty_trimmed(Some(title.as_str()))
        .map(ToString::to_string)
        .or_else(|| non_empty_trimmed(url.as_deref()).map(ToString::to_string))
        .unwrap_or_else(|| format!("screen-{}", index + 1));
    let id = explore_screen_first_string(object, &["id", "key"])
        .and_then(|value| non_empty_trimmed(Some(value.as_str())).map(ToString::to_string))
        .or_else(|| Some(stable_explore_screen_id(&fallback_title, index)));
    let children = object
        .get("children")
        .and_then(serde_json::Value::as_array)
        .map(|children| {
            children
                .iter()
                .enumerate()
                .filter_map(|(child_index, value)| {
                    value
                        .as_object()
                        .and_then(|object| explore_screen_from_json_object(object, child_index))
                })
                .collect::<Vec<_>>()
        })
        .filter(|children| !children.is_empty());
    let style = object
        .get("style")
        .and_then(explore_screen_string_dictionary)
        .filter(|style| !style.is_empty());
    let metadata = ["type", "group", "description"]
        .into_iter()
        .filter_map(|key| {
            object
                .get(key)
                .and_then(explore_screen_value_to_string)
                .map(|value| (key.to_string(), value))
        })
        .collect::<BTreeMap<_, _>>();

    Some(ExploreScreen {
        id,
        title: fallback_title,
        url: url.and_then(|value| non_empty_trimmed(Some(value.as_str())).map(ToString::to_string)),
        order: index as i32,
        children,
        style,
        metadata: (!metadata.is_empty()).then_some(metadata),
    })
}

fn parse_delimited_explore_screens(raw: &str) -> Vec<ExploreScreen> {
    delimited_explore_screen_entries(raw)
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let (title, url) = split_explore_screen_title_and_url(&entry);
            let title = non_empty_trimmed(Some(title.as_str()))
                .map(ToString::to_string)
                .or_else(|| non_empty_trimmed(url.as_deref()).map(ToString::to_string))
                .unwrap_or_else(|| format!("screen-{}", index + 1));
            ExploreScreen {
                id: Some(stable_explore_screen_id(&title, index)),
                title,
                url: url.and_then(|value| {
                    non_empty_trimmed(Some(value.as_str())).map(ToString::to_string)
                }),
                order: index as i32,
                children: None,
                style: None,
                metadata: None,
            }
        })
        .collect()
}

fn delimited_explore_screen_entries(raw: &str) -> Vec<String> {
    raw.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("&&", "\n")
        .lines()
        .flat_map(split_comma_separated_explore_screen_entry)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn split_comma_separated_explore_screen_entry(value: &str) -> Vec<&str> {
    if value.contains("://") || value.contains(", {") || value.contains(",{") {
        vec![value]
    } else {
        value.split(',').collect()
    }
}

fn split_explore_screen_title_and_url(value: &str) -> (String, Option<String>) {
    let Some((title, url)) = value.split_once("::") else {
        return (value.to_string(), None);
    };
    (title.to_string(), Some(url.to_string()))
}

fn explore_screen_first_string(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(explore_screen_value_to_string))
}

fn explore_screen_string_dictionary(value: &serde_json::Value) -> Option<BTreeMap<String, String>> {
    let object = value.as_object()?;
    Some(
        object
            .iter()
            .filter_map(|(key, value)| {
                explore_screen_value_to_string(value).map(|value| (key.clone(), value))
            })
            .collect(),
    )
}

fn explore_screen_value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Some(value.to_string()),
    }
}

fn stable_explore_screen_id(title: &str, order: usize) -> String {
    let material = format!("{order}|{title}");
    let mut hash = 14_695_981_039_346_656_037u64;
    for scalar in material.chars() {
        hash ^= u64::from(scalar as u32);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("screen-{hash:x}")
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn validate_rss_refresh_decision(decision: &RssRefreshDecision) -> Result<(), RssError> {
    let expected_should_fetch = match decision.reason {
        RssRefreshDecisionReason::Disabled | RssRefreshDecisionReason::IntervalNotElapsed => false,
        RssRefreshDecisionReason::Forced
        | RssRefreshDecisionReason::MissingLastFetchedAt
        | RssRefreshDecisionReason::MissingUpdateInterval
        | RssRefreshDecisionReason::IntervalElapsed => true,
    };
    if decision.should_fetch != expected_should_fetch {
        return Err(RssError::InvalidSubscription {
            field: "refresh_decision.should_fetch".into(),
        });
    }
    Ok(())
}

fn validate_rss_refresh_state(state: &RssRefreshState) -> Result<(), RssError> {
    if state
        .last_etag
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(RssError::InvalidSubscription {
            field: "refresh_state.last_etag".into(),
        });
    }
    if state
        .last_modified
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(RssError::InvalidSubscription {
            field: "refresh_state.last_modified".into(),
        });
    }
    Ok(())
}

fn rss_refresh_decision_reason_wire_value(reason: RssRefreshDecisionReason) -> &'static str {
    match reason {
        RssRefreshDecisionReason::Disabled => "disabled",
        RssRefreshDecisionReason::Forced => "forced",
        RssRefreshDecisionReason::MissingLastFetchedAt => "missingLastFetchedAt",
        RssRefreshDecisionReason::MissingUpdateInterval => "missingUpdateInterval",
        RssRefreshDecisionReason::IntervalElapsed => "intervalElapsed",
        RssRefreshDecisionReason::IntervalNotElapsed => "intervalNotElapsed",
    }
}

fn validate_explore_snapshot_path(path: &str) -> Result<(), RssError> {
    validate_explore_required(path, "snapshots")?;
    let path = path.trim();
    if path.starts_with('/')
        || path.starts_with("http://")
        || path.starts_with("https://")
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(RssError::InvalidSubscription {
            field: "snapshots".into(),
        });
    }
    Ok(())
}

fn selected_explore_screen_url(
    rule: &ExploreRequestRule,
    request: &ExploreRequest,
    screens: &[ExploreScreen],
) -> Result<String, RssError> {
    let selected_screen = request
        .screen_id
        .as_deref()
        .and_then(|screen_id| {
            screens.iter().find(|screen| {
                screen
                    .id
                    .as_deref()
                    .is_some_and(|candidate| candidate == screen_id)
            })
        })
        .or_else(|| {
            request.screen_title.as_deref().and_then(|screen_title| {
                screens
                    .iter()
                    .find(|screen| screen.title.trim() == screen_title.trim())
            })
        });

    if let Some(screen) = selected_screen {
        return non_empty_trimmed(screen.url.as_deref())
            .map(ToString::to_string)
            .ok_or_else(|| RssError::InvalidSubscription {
                field: format!(
                    "explore_screen.url:{}:{}",
                    rule.source_id,
                    screen
                        .id
                        .as_deref()
                        .or_else(|| non_empty_trimmed(Some(&screen.title)))
                        .unwrap_or("selected")
                ),
            });
    }

    non_empty_trimmed(rule.explore_url.as_deref())
        .map(ToString::to_string)
        .ok_or_else(|| RssError::MissingField {
            field: "explore_url".into(),
        })
}

fn split_legacy_explore_url_dsl(
    raw: &str,
) -> Result<(String, Option<serde_json::Map<String, serde_json::Value>>), RssError> {
    let raw = non_empty_trimmed(Some(raw)).ok_or_else(|| RssError::MissingField {
        field: "explore_url".into(),
    })?;
    for (index, ch) in raw.char_indices() {
        if ch != ',' {
            continue;
        }
        let candidate = raw[index + ch.len_utf8()..].trim();
        if !candidate.starts_with('{') {
            continue;
        }
        if let Ok(serde_json::Value::Object(options)) =
            serde_json::from_str::<serde_json::Value>(candidate)
        {
            let template = raw[..index].trim().to_string();
            if template.is_empty() {
                return Err(RssError::MissingField {
                    field: "explore_url".into(),
                });
            }
            return Ok((template, Some(options)));
        }
    }
    Ok((raw.to_string(), None))
}

fn explore_method_from_options(
    options: &Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<ExploreRequestMethod, RssError> {
    let Some(method) = explore_string_option(options, "method")? else {
        return Ok(ExploreRequestMethod::GET);
    };
    match method.to_ascii_uppercase().as_str() {
        "GET" => Ok(ExploreRequestMethod::GET),
        "POST" => Ok(ExploreRequestMethod::POST),
        _ => Err(RssError::InvalidSubscription {
            field: "explore_url.method".into(),
        }),
    }
}

fn explore_headers_from_options(
    options: &Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<BTreeMap<String, String>, RssError> {
    let Some(serde_json::Value::Object(headers)) =
        options.as_ref().and_then(|options| options.get("headers"))
    else {
        return Ok(BTreeMap::new());
    };
    headers
        .iter()
        .map(|(key, value)| {
            let serde_json::Value::String(value) = value else {
                return Err(RssError::InvalidSubscription {
                    field: "explore_url.headers".into(),
                });
            };
            let key =
                non_empty_trimmed(Some(key)).ok_or_else(|| RssError::InvalidSubscription {
                    field: "explore_url.headers".into(),
                })?;
            let value =
                non_empty_trimmed(Some(value)).ok_or_else(|| RssError::InvalidSubscription {
                    field: "explore_url.headers".into(),
                })?;
            Ok((key.to_string(), value.to_string()))
        })
        .collect()
}

fn explore_body_from_options(
    options: &Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<Option<String>, RssError> {
    explore_string_option(options, "body")
}

fn explore_content_type_from_options(
    options: &Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<Option<ExploreExpectedContentType>, RssError> {
    let Some(value) = explore_string_option(options, "type")? else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "html" => Ok(Some(ExploreExpectedContentType::Html)),
        "json" => Ok(Some(ExploreExpectedContentType::Json)),
        "xml" => Ok(Some(ExploreExpectedContentType::Xml)),
        "text" => Ok(Some(ExploreExpectedContentType::Text)),
        _ => Err(RssError::InvalidSubscription {
            field: "explore_url.type".into(),
        }),
    }
}

fn explore_string_option(
    options: &Option<serde_json::Map<String, serde_json::Value>>,
    key: &str,
) -> Result<Option<String>, RssError> {
    let Some(value) = options.as_ref().and_then(|options| options.get(key)) else {
        return Ok(None);
    };
    let serde_json::Value::String(value) = value else {
        return Err(RssError::InvalidSubscription {
            field: format!("explore_url.{key}"),
        });
    };
    Ok(non_empty_trimmed(Some(value)).map(ToString::to_string))
}

fn explore_request_variables(request: &ExploreRequest) -> BTreeMap<String, String> {
    let mut variables = BTreeMap::from([
        ("sourceId".into(), request.source_id.clone()),
        ("sourceName".into(), request.source_name.clone()),
        ("page".into(), request.page.to_string()),
    ]);
    if let Some(category_id) = &request.category_id {
        variables.insert("categoryId".into(), category_id.clone());
    }
    if let Some(category_title) = &request.category_title {
        variables.insert("categoryTitle".into(), category_title.clone());
    }
    if let Some(screen_id) = &request.screen_id {
        variables.insert("screenId".into(), screen_id.clone());
    }
    if let Some(screen_title) = &request.screen_title {
        variables.insert("screenTitle".into(), screen_title.clone());
    }
    for (key, value) in &request.query_parameters {
        variables.insert(key.clone(), value.clone());
    }
    variables
}

fn expand_explore_template(template: &str, variables: &BTreeMap<String, String>) -> String {
    variables
        .iter()
        .fold(template.to_string(), |expanded, (key, value)| {
            expanded.replace(
                &format!("{{{{{key}}}}}"),
                &percent_encode_explore_template_value(value),
            )
        })
}

fn percent_encode_explore_template_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn insert_json_string(
    fields: &mut BTreeMap<String, serde_json::Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.and_then(|value| non_empty_trimmed(Some(value))) {
        fields.insert(key.to_string(), serde_json::json!(value));
    }
}

fn normalized_html_summary(html: &str) -> Option<String> {
    let mut text = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let decoded = decode_xml_entities(&text);
    let normalized = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn collect_new_entries(entries: &[RssEntry], last_entry_id: Option<&str>) -> Vec<RssEntry> {
    let Some(last_entry_id) = last_entry_id else {
        return entries.to_vec();
    };

    let mut new_entries = Vec::new();
    for entry in entries {
        if entry.id == last_entry_id {
            break;
        }
        new_entries.push(entry.clone());
    }
    new_entries
}

fn dedupe_entries(entries: &mut Vec<RssEntry>) {
    let mut seen = HashSet::new();
    entries.retain(|entry| seen.insert(entry.id.clone()));
}

fn validate_subscription_fields(subscription_id: &str, feed_url: &str) -> Result<(), RssError> {
    validate_subscription_id(subscription_id)?;
    if feed_url.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "feed_url".into(),
        });
    }
    Ok(())
}

fn validate_subscription(subscription: &RssSubscription) -> Result<(), RssError> {
    validate_subscription_fields(&subscription.subscription_id, &subscription.feed_url)?;
    if subscription.title.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "title".into(),
        });
    }
    Ok(())
}

fn validate_subscription_id(subscription_id: &str) -> Result<(), RssError> {
    if subscription_id.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "subscription_id".into(),
        });
    }
    Ok(())
}

fn validate_entry_id(entry_id: &str) -> Result<(), RssError> {
    if entry_id.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "entry_id".into(),
        });
    }
    Ok(())
}

fn validate_entry(entry: &RssEntry) -> Result<(), RssError> {
    validate_entry_id(&entry.id)
}

fn validate_entry_state(state: &RssEntryState) -> Result<(), RssError> {
    validate_subscription_id(&state.subscription_id)?;
    validate_entry(&state.entry)?;
    if !state.read && state.read_at.is_some() {
        return Err(RssError::InvalidSnapshot {
            field: "entries.read_at".into(),
        });
    }
    Ok(())
}

fn first_text(input: &str, tag: &str) -> Option<String> {
    first_element_body(input, tag).and_then(|body| {
        let text = clean_text(&body);
        (!text.is_empty()).then_some(text)
    })
}

fn first_xml_character_text(input: &str, tag: &str) -> Option<String> {
    first_element_body(input, tag).and_then(|body| {
        let text = xml_character_text(&body);
        (!text.is_empty()).then_some(text)
    })
}

fn first_summary_text(input: &str, tag: &str) -> Option<String> {
    first_element_body(input, tag).and_then(|body| normalize_rss_summary_text(&body))
}

fn first_element_body(input: &str, tag: &str) -> Option<String> {
    element_bodies(input, tag).into_iter().next()
}

fn xml_item_unknown_fields(
    item: &str,
    guid: Option<&str>,
    content: Option<&str>,
) -> BTreeMap<String, serde_json::Value> {
    let mut fields = BTreeMap::new();
    insert_json_string(&mut fields, "guid", guid);
    insert_json_string(&mut fields, "content", content);
    if let Some(categories) = xml_category_values(item) {
        fields.insert("categories".into(), serde_json::json!(categories));
    }
    if let Some(image) = xml_item_image(item) {
        fields.insert("image".into(), serde_json::json!(image));
    }
    if let Some(media_type) = xml_media_content_type(item) {
        fields.insert("mediaType".into(), serde_json::json!(media_type));
    }
    if let Some(enclosure) = xml_enclosure_metadata(item) {
        fields.insert("enclosure".into(), enclosure);
    }
    fields
}

fn xml_category_values(input: &str) -> Option<Vec<String>> {
    let mut categories = Vec::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, "category", from) {
        let start_tag = &input[start.open_start..=start.open_end];
        let term = attr_value(start_tag, "term");
        let mut text = None;
        if start.self_closing {
            from = start.open_end + 1;
        } else if let Some((close_start, close_end)) =
            find_end_tag(input, "category", start.content_start)
        {
            let raw = &input[start.content_start..close_start];
            let cleaned = clean_text(raw);
            text = non_empty_trimmed(Some(&cleaned)).map(ToString::to_string);
            from = close_end;
        } else {
            from = start.open_end + 1;
        }
        if let Some(category) = text.or(term) {
            categories.push(category);
        }
    }
    (!categories.is_empty()).then_some(categories)
}

fn xml_item_image(input: &str) -> Option<String> {
    first_text(input, "image")
        .or_else(|| xml_start_tag_attr(input, "media:thumbnail", "url"))
        .or_else(|| xml_start_tag_attr(input, "thumbnail", "url"))
        .or_else(|| xml_start_tag_attr(input, "media:content", "url"))
}

fn xml_media_content_type(input: &str) -> Option<String> {
    xml_start_tag_attr(input, "media:content", "type")
}

fn xml_enclosure_metadata(input: &str) -> Option<serde_json::Value> {
    let tag = xml_start_tag(input, "enclosure")?;
    let mut fields = serde_json::Map::new();
    if let Some(url) = attr_value(&tag, "url") {
        fields.insert("url".into(), serde_json::json!(url));
    }
    if let Some(content_type) = attr_value(&tag, "type") {
        fields.insert("type".into(), serde_json::json!(content_type));
    }
    if let Some(length) = attr_value(&tag, "length") {
        let value = length
            .parse::<u64>()
            .map(serde_json::Value::from)
            .unwrap_or_else(|_| serde_json::json!(length));
        fields.insert("length".into(), value);
    }
    (!fields.is_empty()).then_some(serde_json::Value::Object(fields))
}

fn xml_start_tag_attr(input: &str, tag: &str, attr: &str) -> Option<String> {
    attr_value(&xml_start_tag(input, tag)?, attr)
}

fn xml_start_tag(input: &str, tag: &str) -> Option<String> {
    let start = find_start_tag(input, tag, 0)?;
    Some(input[start.open_start..=start.open_end].to_string())
}

fn element_bodies(input: &str, tag: &str) -> Vec<String> {
    let mut bodies = Vec::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, tag, from) {
        if start.self_closing {
            from = start.open_end + 1;
            continue;
        }
        let Some((close_start, close_end)) = find_end_tag(input, tag, start.content_start) else {
            break;
        };
        bodies.push(input[start.content_start..close_start].to_string());
        from = close_end;
    }
    bodies
}

fn remove_element_blocks(input: &str, tag: &str) -> String {
    let mut output = String::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, tag, from) {
        output.push_str(&input[from..start.open_start]);
        if start.self_closing {
            from = start.open_end + 1;
            continue;
        }
        let Some((_, close_end)) = find_end_tag(input, tag, start.content_start) else {
            from = start.open_end + 1;
            continue;
        };
        from = close_end;
    }
    output.push_str(&input[from..]);
    output
}

fn has_element(input: &str, tag: &str) -> bool {
    find_start_tag(input, tag, 0).is_some()
}

fn first_link_href(input: &str) -> Option<String> {
    link_start_tags(input)
        .into_iter()
        .find_map(|tag| attr_value(&tag, "href"))
}

fn link_href_by_rel(input: &str, rel: &str) -> Option<String> {
    link_start_tags(input).into_iter().find_map(|tag| {
        let tag_rel = attr_value(&tag, "rel")?;
        tag_rel
            .eq_ignore_ascii_case(rel)
            .then(|| attr_value(&tag, "href"))
            .flatten()
    })
}

fn link_href_by_rel_local(input: &str, rel: &str) -> Option<String> {
    local_link_start_tags(input).into_iter().find_map(|tag| {
        let tag_rel = attr_value(&tag, "rel")?;
        tag_rel
            .eq_ignore_ascii_case(rel)
            .then(|| attr_value(&tag, "href"))
            .flatten()
    })
}

fn link_start_tags(input: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, "link", from) {
        tags.push(input[start.open_start..=start.open_end].to_string());
        from = start.open_end + 1;
    }
    tags
}

fn local_link_start_tags(input: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut from = 0usize;
    while let Some(relative) = input[from..].find('<') {
        let open_start = from + relative;
        let Some(open_end) = input[open_start..].find('>').map(|end| open_start + end) else {
            break;
        };
        let tag = &input[open_start..=open_end];
        if start_tag_local_name(tag).is_some_and(|name| name.eq_ignore_ascii_case("link")) {
            tags.push(tag.to_string());
        }
        from = open_end + 1;
    }
    tags
}

fn start_tag_local_name(start_tag: &str) -> Option<&str> {
    let trimmed = start_tag.trim_start_matches('<').trim_start();
    if trimmed.starts_with('/') || trimmed.starts_with('!') || trimmed.starts_with('?') {
        return None;
    }
    let name = trimmed
        .split(|ch: char| ch == '>' || ch == '/' || ch.is_ascii_whitespace())
        .next()?;
    if name.is_empty() {
        return None;
    }
    Some(
        name.rsplit_once(':')
            .map(|(_, local)| local)
            .unwrap_or(name),
    )
}

fn urls_equivalent(left: &str, right: &str) -> bool {
    left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
}

#[derive(Debug, Clone, Copy)]
struct StartTag {
    open_start: usize,
    open_end: usize,
    content_start: usize,
    self_closing: bool,
}

fn find_start_tag(input: &str, tag: &str, from: usize) -> Option<StartTag> {
    let lower_input = input.to_ascii_lowercase();
    let lower_tag = tag.to_ascii_lowercase();
    let needle = format!("<{lower_tag}");
    let mut search_from = from;

    while search_from < input.len() {
        let relative = lower_input[search_from..].find(&needle)?;
        let open_start = search_from + relative;
        let name_end = open_start + needle.len();
        if !is_tag_boundary(input, name_end) {
            search_from = name_end;
            continue;
        }
        let open_end = input[open_start..].find('>')? + open_start;
        let start_tag = &input[open_start..=open_end];
        return Some(StartTag {
            open_start,
            open_end,
            content_start: open_end + 1,
            self_closing: start_tag.trim_end().ends_with("/>"),
        });
    }

    None
}

fn find_end_tag(input: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let lower_input = input.to_ascii_lowercase();
    let needle = format!("</{}>", tag.to_ascii_lowercase());
    let relative = lower_input[from..].find(&needle)?;
    let close_start = from + relative;
    Some((close_start, close_start + needle.len()))
}

fn is_tag_boundary(input: &str, index: usize) -> bool {
    input[index..]
        .chars()
        .next()
        .map(|ch| ch == '>' || ch == '/' || ch.is_ascii_whitespace())
        .unwrap_or(false)
}

fn attr_value(start_tag: &str, attr: &str) -> Option<String> {
    let lower = start_tag.to_ascii_lowercase();
    let needle = attr.to_ascii_lowercase();
    let mut from = 0usize;

    while from < start_tag.len() {
        let relative = lower[from..].find(&needle)?;
        let name_start = from + relative;
        let name_end = name_start + needle.len();
        if !is_attr_boundary(start_tag, name_start, name_end) {
            from = name_end;
            continue;
        }

        let mut cursor = name_end;
        cursor = skip_ascii_ws(start_tag, cursor);
        if start_tag[cursor..].chars().next() != Some('=') {
            from = name_end;
            continue;
        }
        cursor += 1;
        cursor = skip_ascii_ws(start_tag, cursor);
        let quote = start_tag[cursor..].chars().next()?;
        if quote != '"' && quote != '\'' {
            from = cursor;
            continue;
        }
        cursor += quote.len_utf8();
        let end_relative = start_tag[cursor..].find(quote)?;
        let raw = &start_tag[cursor..cursor + end_relative];
        return Some(clean_text(raw));
    }

    None
}

fn is_attr_boundary(input: &str, start: usize, end: usize) -> bool {
    let before_ok = input[..start]
        .chars()
        .next_back()
        .map(|ch| ch == '<' || ch.is_ascii_whitespace())
        .unwrap_or(true);
    let after_ok = input[end..]
        .chars()
        .next()
        .map(|ch| ch == '=' || ch.is_ascii_whitespace())
        .unwrap_or(false);
    before_ok && after_ok
}

fn skip_ascii_ws(input: &str, mut cursor: usize) -> usize {
    while cursor < input.len() {
        let Some(ch) = input[cursor..].chars().next() else {
            break;
        };
        if !ch.is_ascii_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn clean_text(raw: &str) -> String {
    decode_xml_entities(&strip_cdata(raw.trim()))
        .trim()
        .to_string()
}

fn xml_character_text(raw: &str) -> String {
    let trimmed = raw.trim();
    let text = if is_wrapped_cdata(trimmed) {
        strip_cdata(trimmed)
    } else {
        decode_xml_entities(trimmed)
    };
    text.trim().to_string()
}

fn normalize_rss_summary_text(raw: &str) -> Option<String> {
    let mut text = strip_cdata(raw.trim());
    text = strip_rss_summary_html_markup(&text);
    text = decode_xml_entities(&text);
    text = strip_rss_summary_html_markup(&text);
    text = decode_xml_entities(&text);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

fn strip_rss_summary_html_markup(input: &str) -> String {
    let without_script = remove_element_blocks(input, "script");
    let without_style = remove_element_blocks(&without_script, "style");
    strip_markup_tags_to_spaces(&without_style)
}

fn strip_markup_tags_to_spaces(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text
}

fn strip_cdata(input: &str) -> String {
    let mut text = input.trim().to_string();
    if is_wrapped_cdata(&text) {
        text = text[9..text.len() - 3].to_string();
    }
    text
}

fn is_wrapped_cdata(input: &str) -> bool {
    input.starts_with("<![CDATA[") && input.ends_with("]]>") && input.len() >= 12
}

fn decode_xml_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(relative) = input[cursor..].find('&') {
        let amp = cursor + relative;
        output.push_str(&input[cursor..amp]);
        if let Some((replacement, next)) = decode_entity_at(input, amp) {
            output.push_str(&replacement);
            cursor = next;
        } else {
            output.push('&');
            cursor = amp + 1;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn decode_entity_at(input: &str, amp: usize) -> Option<(String, usize)> {
    let after_amp = amp + 1;
    let rest = input.get(after_amp..)?;
    if let Some(rest) = rest.strip_prefix("#x").or_else(|| rest.strip_prefix("#X")) {
        let digit_len = rest
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .map(char::len_utf8)
            .sum::<usize>();
        return decode_numeric_entity(input, after_amp + 2, digit_len, 16);
    }
    if let Some(rest) = rest.strip_prefix('#') {
        let digit_len = rest
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .map(char::len_utf8)
            .sum::<usize>();
        return decode_numeric_entity(input, after_amp + 1, digit_len, 10);
    }

    let name_len = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .map(char::len_utf8)
        .sum::<usize>();
    if name_len == 0 {
        return None;
    }
    let name = &rest[..name_len];
    let replacement = named_html_entity(name)?;
    let mut next = after_amp + name_len;
    if input[next..].starts_with(';') {
        next += 1;
    }
    Some((replacement.to_string(), next))
}

fn decode_numeric_entity(
    input: &str,
    digit_start: usize,
    digit_len: usize,
    radix: u32,
) -> Option<(String, usize)> {
    if digit_len == 0 {
        return None;
    }
    let digits = &input[digit_start..digit_start + digit_len];
    let code_point = u32::from_str_radix(digits, radix).ok()?;
    let scalar = char::from_u32(code_point)?;
    let mut next = digit_start + digit_len;
    if input[next..].starts_with(';') {
        next += 1;
    }
    Some((scalar.to_string(), next))
}

fn named_html_entity(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "amp" => Some("&"),
        "quot" => Some("\""),
        "apos" => Some("'"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "nbsp" | "ensp" | "emsp" | "thinsp" => Some(" "),
        "ndash" => Some("\u{2013}"),
        "mdash" => Some("\u{2014}"),
        "hellip" => Some("\u{2026}"),
        "copy" => Some("\u{00A9}"),
        "reg" => Some("\u{00AE}"),
        "trade" => Some("\u{2122}"),
        "lsquo" => Some("\u{2018}"),
        "rsquo" => Some("\u{2019}"),
        "ldquo" => Some("\u{201C}"),
        "rdquo" => Some("\u{201D}"),
        "laquo" => Some("\u{00AB}"),
        "raquo" => Some("\u{00BB}"),
        "sect" => Some("\u{00A7}"),
        "para" => Some("\u{00B6}"),
        "deg" => Some("\u{00B0}"),
        "plusmn" => Some("\u{00B1}"),
        "middot" => Some("\u{00B7}"),
        "bull" => Some("\u{2022}"),
        "times" => Some("\u{00D7}"),
        "divide" => Some("\u{00F7}"),
        "frac14" => Some("\u{00BC}"),
        "frac12" => Some("\u{00BD}"),
        "frac34" => Some("\u{00BE}"),
        "euro" => Some("\u{20AC}"),
        "pound" => Some("\u{00A3}"),
        "yen" => Some("\u{00A5}"),
        "cent" => Some("\u{00A2}"),
        "agrave" => latin_entity(name, "\u{00E0}", "\u{00C0}"),
        "aacute" => latin_entity(name, "\u{00E1}", "\u{00C1}"),
        "egrave" => latin_entity(name, "\u{00E8}", "\u{00C8}"),
        "eacute" => latin_entity(name, "\u{00E9}", "\u{00C9}"),
        "iacute" => latin_entity(name, "\u{00ED}", "\u{00CD}"),
        "oacute" => latin_entity(name, "\u{00F3}", "\u{00D3}"),
        "uacute" => latin_entity(name, "\u{00FA}", "\u{00DA}"),
        "ntilde" => latin_entity(name, "\u{00F1}", "\u{00D1}"),
        "uuml" => latin_entity(name, "\u{00FC}", "\u{00DC}"),
        _ => None,
    }
}

fn latin_entity(
    name: &str,
    lowercase: &'static str,
    uppercase: &'static str,
) -> Option<&'static str> {
    name.chars().next().map(|ch| {
        if ch.is_ascii_uppercase() {
            uppercase
        } else {
            lowercase
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refresh_policy(
        enabled: bool,
        update_interval_minutes: Option<u32>,
        last_fetched_at: Option<i64>,
        force_refresh: bool,
    ) -> RssRefreshPolicy {
        RssRefreshPolicy {
            enabled,
            update_interval_minutes,
            last_fetched_at,
            force_refresh,
        }
    }

    #[test]
    fn refresh_decision_reason_wire_values_match_legacy_reader_core() {
        let cases = [
            (RssRefreshDecisionReason::Disabled, "disabled"),
            (RssRefreshDecisionReason::Forced, "forced"),
            (
                RssRefreshDecisionReason::MissingLastFetchedAt,
                "missingLastFetchedAt",
            ),
            (
                RssRefreshDecisionReason::MissingUpdateInterval,
                "missingUpdateInterval",
            ),
            (RssRefreshDecisionReason::IntervalElapsed, "intervalElapsed"),
            (
                RssRefreshDecisionReason::IntervalNotElapsed,
                "intervalNotElapsed",
            ),
        ];

        for (reason, expected) in cases {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let decoded: RssRefreshDecisionReason = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, reason);
        }
    }

    #[test]
    fn refresh_decision_and_state_json_match_legacy_reader_core_shape() {
        let decision = RssRefreshDecision {
            should_fetch: false,
            reason: RssRefreshDecisionReason::IntervalNotElapsed,
            evaluated_at: 1_781_943_600,
            next_eligible_fetch_at: Some(1_781_945_400),
        };
        let decision_json = serde_json::to_value(&decision).unwrap();
        assert_eq!(
            decision_json,
            serde_json::json!({
                "shouldFetch": false,
                "reason": "intervalNotElapsed",
                "evaluatedAt": 1_781_943_600,
                "nextEligibleFetchAt": 1_781_945_400
            })
        );
        assert_eq!(
            serde_json::from_value::<RssRefreshDecision>(decision_json).unwrap(),
            decision
        );

        let state = RssRefreshState {
            last_fetched_at: Some(1_781_947_200),
            last_etag: Some(r#""reader-core-etag-v2""#.into()),
            last_modified: Some("Sat, 20 Jun 2026 08:00:00 GMT".into()),
            next_eligible_fetch_at: Some(1_781_950_800),
            not_modified: true,
        };
        let state_json = serde_json::to_value(&state).unwrap();
        assert_eq!(
            state_json,
            serde_json::json!({
                "lastFetchedAt": 1_781_947_200,
                "lastETag": "\"reader-core-etag-v2\"",
                "lastModified": "Sat, 20 Jun 2026 08:00:00 GMT",
                "nextEligibleFetchAt": 1_781_950_800,
                "notModified": true
            })
        );
        assert_eq!(
            serde_json::from_value::<RssRefreshState>(state_json).unwrap(),
            state
        );
    }

    #[test]
    fn explore_fixture_manifest_decodes_legacy_no_network_corpus_contract() {
        let manifest_json = r#"{
            "sourceId": "fixture-explore-001",
            "sourceName": "Fixture Explore Source",
            "fixtureRoot": "samples/booksources/explore",
            "categories": [
                {
                    "id": "cat-fantasy",
                    "title": "玄幻",
                    "urlTemplate": "/explore/fantasy",
                    "order": 1,
                    "children": null,
                    "metadata": {}
                },
                {
                    "id": "cat-urban",
                    "title": "都市",
                    "urlTemplate": "/explore/urban",
                    "order": 2,
                    "children": null,
                    "metadata": {}
                },
                {
                    "id": "cat-history",
                    "title": "历史",
                    "urlTemplate": "/explore/history",
                    "order": 3,
                    "children": null,
                    "metadata": {}
                },
                {
                    "id": "cat-sci-fi",
                    "title": "科幻",
                    "urlTemplate": "/explore/sci-fi",
                    "order": 4,
                    "children": null,
                    "metadata": {}
                }
            ],
            "snapshots": [
                "fixtures/explore_list.html",
                "fixtures/rss_feed.xml",
                "fixtures/subscription_source.json"
            ],
            "expectedResultCount": 3,
            "noNetworkReplay": true,
            "firstFetchSavedAt": 1735090200,
            "repeatedFetchForbidden": true
        }"#;

        let manifest: ExploreFixtureManifest = serde_json::from_str(manifest_json).unwrap();

        manifest.validate().unwrap();
        assert_eq!(manifest.source_id, "fixture-explore-001");
        assert_eq!(manifest.fixture_root, "samples/booksources/explore");
        assert_eq!(
            manifest
                .categories
                .iter()
                .map(|category| category.title.as_str())
                .collect::<Vec<_>>(),
            vec!["玄幻", "都市", "历史", "科幻"]
        );
        assert_eq!(manifest.snapshots.len(), 3);
        assert_eq!(manifest.expected_result_count, Some(3));
        assert_eq!(manifest.first_fetch_saved_at, Some(1_735_090_200));
        assert!(manifest.requires_offline_replay());

        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["sourceId"], "fixture-explore-001");
        assert_eq!(json["expectedResultCount"], 3);
        assert_eq!(json["firstFetchSavedAt"], 1_735_090_200);
        assert_eq!(
            serde_json::from_value::<ExploreFixtureManifest>(json).unwrap(),
            manifest
        );
    }

    #[test]
    fn explore_fixture_manifest_rejects_drifted_fixture_evidence() {
        let mut manifest = ExploreFixtureManifest {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            fixture_root: "samples/booksources/explore".into(),
            categories: vec![
                ExploreCategory {
                    id: "cat-a".into(),
                    title: "A".into(),
                    url_template: Some("/a".into()),
                    order: 1,
                    children: None,
                    metadata: None,
                },
                ExploreCategory {
                    id: "cat-b".into(),
                    title: "B".into(),
                    url_template: Some("/b".into()),
                    order: 2,
                    children: None,
                    metadata: None,
                },
            ],
            snapshots: vec![
                "fixtures/explore_list.html".into(),
                "fixtures/rss_feed.xml".into(),
            ],
            expected_result_count: Some(2),
            no_network_replay: true,
            first_fetch_saved_at: None,
            repeated_fetch_forbidden: true,
        };
        manifest.validate().unwrap();

        let mut duplicate_category = manifest.clone();
        duplicate_category.categories[1].id = "cat-a".into();
        assert_eq!(
            duplicate_category.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "categories.id".into()
            }
        );

        let mut unsafe_snapshot = manifest.clone();
        unsafe_snapshot
            .snapshots
            .push("fixtures/../secret.html".into());
        assert_eq!(
            unsafe_snapshot.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "snapshots".into()
            }
        );

        let mut duplicate_snapshot = manifest.clone();
        duplicate_snapshot
            .snapshots
            .push("fixtures/rss_feed.xml".into());
        assert_eq!(
            duplicate_snapshot.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "snapshots".into()
            }
        );

        manifest.expected_result_count = Some(0);
        assert_eq!(
            manifest.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "expected_result_count".into()
            }
        );
    }

    #[test]
    fn explore_request_spec_expands_template_and_legacy_url_dsl_options() {
        let request = ExploreRequest {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            category_id: Some("fantasy".into()),
            category_title: Some("玄幻".into()),
            screen_id: None,
            screen_title: None,
            page: 3,
            query_parameters: BTreeMap::from([("key".into(), "斗破".into())]),
        };
        let rule = ExploreRequestRule {
            source_id: request.source_id.clone(),
            source_name: request.source_name.clone(),
            enabled_explore: true,
            explore_url: Some(r#"https://example.com/explore/{{categoryId}}?q={{key}}&page={{page}}, {"method":"POST","headers":{"X-Source":"fixture"},"body":"cat={{categoryId}}&q={{key}}","charset":"gbk","type":"html"}"#.into()),
            explore_screen: None,
        };

        let spec = build_explore_request_spec(&rule, &request).unwrap();

        assert_eq!(spec.stage, "explore");
        assert_eq!(spec.method, ExploreRequestMethod::POST);
        assert_eq!(
            spec.url_template,
            "https://example.com/explore/fantasy?q=%E6%96%97%E7%A0%B4&page=3"
        );
        assert_eq!(
            spec.headers.get("X-Source").map(String::as_str),
            Some("fixture")
        );
        assert_eq!(
            spec.body_template
                .as_ref()
                .map(|body| body.template.as_str()),
            Some("cat=fantasy&q=%E6%96%97%E7%A0%B4")
        );
        assert_eq!(spec.charset.as_deref(), Some("gbk"));
        assert_eq!(
            spec.expected_content_type,
            Some(ExploreExpectedContentType::Html)
        );
        assert_eq!(
            spec.capability_requirements,
            BTreeSet::from([
                ExploreRequestCapability::NetworkRequest,
                ExploreRequestCapability::CustomHeader,
                ExploreRequestCapability::PostBody,
                ExploreRequestCapability::Charset,
            ])
        );
    }

    #[test]
    fn explore_screen_parser_matches_legacy_delimited_and_json_shapes() {
        let delimited_rule = ExploreRequestRule {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            enabled_explore: true,
            explore_url: None,
            explore_screen: Some(
                "玄幻::https://example.com/fantasy?page={{page}}&&榜单::https://example.com/rank"
                    .into(),
            ),
        };

        let delimited = parse_explore_screens(&delimited_rule).unwrap();

        assert_eq!(delimited.len(), 2);
        assert_eq!(delimited[0].title, "玄幻");
        assert_eq!(
            delimited[0].url.as_deref(),
            Some("https://example.com/fantasy?page={{page}}")
        );
        assert_eq!(
            delimited[0].id.as_deref(),
            Some(stable_explore_screen_id("玄幻", 0).as_str())
        );
        assert_eq!(delimited[1].title, "榜单");
        assert!(delimited[1].is_executable());

        let json_rule = ExploreRequestRule {
            explore_screen: Some(
                r#"[
                  {
                    "id":"fantasy",
                    "title":"玄幻",
                    "url":"https://example.com/fantasy",
                    "style":{"layout":"grid"},
                    "type":"featured",
                    "children":[{"label":"子类","exploreUrl":"https://example.com/child"}]
                  },
                  {"key":"rank","label":"榜单","exploreUrl":"https://example.com/rank"}
                ]"#
                .into(),
            ),
            ..delimited_rule
        };

        let screens = parse_explore_screens(&json_rule).unwrap();

        assert_eq!(screens.len(), 2);
        assert_eq!(screens[0].id.as_deref(), Some("fantasy"));
        assert_eq!(
            screens[0].style.as_ref().unwrap().get("layout"),
            Some(&"grid".to_string())
        );
        assert_eq!(
            screens[0].metadata.as_ref().unwrap().get("type"),
            Some(&"featured".to_string())
        );
        let child = &screens[0].children.as_ref().unwrap()[0];
        assert_eq!(child.title, "子类");
        assert_eq!(child.url.as_deref(), Some("https://example.com/child"));
        assert_eq!(screens[1].id.as_deref(), Some("rank"));
        assert_eq!(screens[1].title, "榜单");
        assert_eq!(screens[1].url.as_deref(), Some("https://example.com/rank"));

        let json = serde_json::to_value(&screens[0]).unwrap();
        assert_eq!(json["urlTemplate"], "https://example.com/fantasy");
        assert!(json.get("url").is_none());
    }

    #[test]
    fn explore_request_spec_selects_screen_by_id_and_title_fallback() {
        let rule = ExploreRequestRule {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            enabled_explore: true,
            explore_url: Some("https://example.com/default?page={{page}}".into()),
            explore_screen: Some(
                r#"[{"id":"latest","title":"最新","url":"https://example.com/latest?page={{page}}"},{"id":"rank","title":"榜单","url":"https://example.com/rank?q={{key}}&page={{page}}"}]"#
                    .into(),
            ),
        };
        let by_id = ExploreRequest {
            source_id: rule.source_id.clone(),
            source_name: rule.source_name.clone(),
            category_id: None,
            category_title: None,
            screen_id: Some("rank".into()),
            screen_title: Some("榜单".into()),
            page: 4,
            query_parameters: BTreeMap::from([("key".into(), "热血".into())]),
        };

        let screens = parse_explore_screens(&rule).unwrap();
        let spec = build_explore_request_spec(&rule, &by_id).unwrap();

        assert_eq!(screens.len(), 2);
        assert_eq!(
            spec.url_template,
            "https://example.com/rank?q=%E7%83%AD%E8%A1%80&page=4"
        );
        assert_eq!(spec.debug_description, "explore:fixture-explore-001:page:4");

        let title_rule = ExploreRequestRule {
            explore_screen: Some(
                r#"[{"id":"rank","title":"榜单","url":"https://example.com/rank?page={{page}}"}]"#
                    .into(),
            ),
            ..rule.clone()
        };
        let by_title = ExploreRequest {
            screen_id: Some("stale-id".into()),
            screen_title: Some("榜单".into()),
            page: 2,
            query_parameters: BTreeMap::new(),
            ..by_id
        };
        let title_spec = build_explore_request_spec(&title_rule, &by_title).unwrap();
        assert_eq!(title_spec.url_template, "https://example.com/rank?page=2");
    }

    #[test]
    fn explore_request_spec_selects_legacy_delimited_screen_by_title() {
        let rule = ExploreRequestRule {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            enabled_explore: true,
            explore_url: Some("https://example.com/default?page={{page}}".into()),
            explore_screen: Some(
                "全部::https://example.com/all?page={{page}}&&榜单::https://example.com/rank?q={{key}}&page={{page}}"
                    .into(),
            ),
        };
        let request = ExploreRequest {
            source_id: rule.source_id.clone(),
            source_name: rule.source_name.clone(),
            category_id: None,
            category_title: None,
            screen_id: None,
            screen_title: Some("榜单".into()),
            page: 2,
            query_parameters: BTreeMap::from([("key".into(), "热血".into())]),
        };

        let spec = build_explore_request_spec(&rule, &request).unwrap();

        assert_eq!(
            spec.url_template,
            "https://example.com/rank?q=%E7%83%AD%E8%A1%80&page=2"
        );
        assert_eq!(spec.debug_description, "explore:fixture-explore-001:page:2");
    }

    #[test]
    fn explore_request_spec_fails_when_selected_screen_has_no_url() {
        let rule = ExploreRequestRule {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            enabled_explore: true,
            explore_url: None,
            explore_screen: Some(r#"[{"id":"screen-without-url","title":"筛选"}]"#.into()),
        };
        let request = ExploreRequest {
            source_id: rule.source_id.clone(),
            source_name: rule.source_name.clone(),
            category_id: None,
            category_title: None,
            screen_id: Some("screen-without-url".into()),
            screen_title: None,
            page: 1,
            query_parameters: BTreeMap::new(),
        };

        assert_eq!(
            build_explore_request_spec(&rule, &request).unwrap_err(),
            RssError::InvalidSubscription {
                field: "explore_screen.url:fixture-explore-001:screen-without-url".into()
            }
        );
    }

    #[test]
    fn explore_request_spec_rejects_drifted_options_without_new_runtime_taxonomy() {
        let request = ExploreRequest {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            category_id: None,
            category_title: None,
            screen_id: None,
            screen_title: None,
            page: 1,
            query_parameters: BTreeMap::new(),
        };
        let bad_method = ExploreRequestRule {
            source_id: request.source_id.clone(),
            source_name: request.source_name.clone(),
            enabled_explore: true,
            explore_url: Some(r#"https://example.com/explore, {"method":"PUT"}"#.into()),
            explore_screen: None,
        };

        assert_eq!(
            build_explore_request_spec(&bad_method, &request).unwrap_err(),
            RssError::InvalidSubscription {
                field: "explore_url.method".into()
            }
        );

        let disabled = ExploreRequestRule {
            enabled_explore: false,
            ..bad_method
        };
        assert_eq!(
            build_explore_request_spec(&disabled, &request).unwrap_err(),
            RssError::InvalidSubscription {
                field: "enabled_explore".into()
            }
        );
    }

    #[test]
    fn explore_html_parser_extracts_fixture_items_like_legacy_snapshot_parser() {
        let html = r#"
            <html><body>
              <div class="book-item">
                <h2 class="book-title"><a href="/book/12345">斗破苍穹</a></h2>
                <span class="book-author">作者：天蚕土豆</span>
                <div class="book-cover"><img src="/covers/fantasy-book-1.jpg" /></div>
                <p class="book-desc">这里是简介</p>
                <span class="book-last-chapter">最新章节：第一千六百四十三章 大结局</span>
                <span class="book-update-time">更新时间：2025-12-25</span>
                <span class="book-tags">玄幻, 斗气, 升级</span>
              </div>
              <div class="book-item">
                <h2 class="book-title"><a href="https://cdn.example.com/book/2">全职高手</a></h2>
                <span class="book-author">蝴蝶蓝</span>
                <span class="book-tags">游戏，竞技</span>
              </div>
              <a class="next-link" href="/explore?page=3">下一页</a>
            </body></html>
        "#;
        let request = ExploreRequest {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            category_id: Some("cat-fantasy".into()),
            category_title: Some("玄幻".into()),
            screen_id: None,
            screen_title: None,
            page: 2,
            query_parameters: BTreeMap::new(),
        };
        let rule = ExploreHtmlParseRule {
            source_id: request.source_id.clone(),
            source_name: request.source_name.clone(),
            enabled_explore: true,
            book_list: Some(".book-item".into()),
            name: Some(".book-title a@text".into()),
            author: Some(".book-author@text".into()),
            book_url: Some(".book-title a@href".into()),
            cover_url: Some(".book-cover img@src".into()),
            intro: Some(".book-desc@text".into()),
            last_chapter: Some(".book-last-chapter@text".into()),
            kind: Some(".book-tags@text".into()),
            update_time: Some(".book-update-time@text".into()),
            tags: Some(".book-tags@text".into()),
            next_page: None,
        };

        let result = parse_explore_html(
            html,
            &rule,
            &request,
            Some("https://example.com/explore?page=2"),
            Some("fixtures/explore_list.html"),
            1_735_090_200,
            true,
        )
        .unwrap();

        assert_eq!(result.source_id, "fixture-explore-001");
        assert_eq!(result.category_id.as_deref(), Some("cat-fantasy"));
        assert_eq!(result.page, 2);
        assert_eq!(result.total_count, 2);
        assert!(result.has_next_page);
        assert_eq!(result.next_page_url, None);
        assert_eq!(
            result.snapshot_id.as_deref(),
            Some("fixtures/explore_list.html")
        );
        assert!(result.replayed_from_local_snapshot);
        assert!(result.warnings.is_empty());

        let first = &result.items[0];
        assert_eq!(first.title, "斗破苍穹");
        assert_eq!(first.author.as_deref(), Some("天蚕土豆"));
        assert_eq!(first.book_url, "https://example.com/book/12345");
        assert_eq!(
            first.cover_url.as_deref(),
            Some("https://example.com/covers/fantasy-book-1.jpg")
        );
        assert_eq!(first.intro.as_deref(), Some("这里是简介"));
        assert_eq!(
            first.last_chapter.as_deref(),
            Some("第一千六百四十三章 大结局")
        );
        assert_eq!(first.update_time.as_deref(), Some("2025-12-25"));
        assert_eq!(first.tags, vec!["玄幻", "斗气", "升级"]);
        assert_eq!(first.kind.as_deref(), Some("玄幻, 斗气, 升级"));
        assert_eq!(first.raw_fields["bookList"], ".book-item");

        let second = &result.items[1];
        assert_eq!(second.title, "全职高手");
        assert_eq!(second.book_url, "https://cdn.example.com/book/2");
        assert_eq!(second.tags, vec!["游戏", "竞技"]);
        assert_ne!(first.id, second.id);
        assert_eq!(
            serde_json::to_value(&result).unwrap()["replayedFromLocalSnapshot"],
            true
        );
    }

    #[test]
    fn explore_html_parser_configured_next_page_overrides_fallback() {
        let html_with_configured_next = r#"
            <html><body>
              <div class="book-item">
                <h2 class="book-title"><a href="/book/1">Rule Next One</a></h2>
              </div>
              <nav class="pager"><a class="advance" href="/explore?page=2">More</a></nav>
            </body></html>
        "#;
        let html_with_only_fallback = r#"
            <html><body>
              <div class="book-item">
                <h2 class="book-title"><a href="/book/1">Fallback Exists</a></h2>
              </div>
              <a href="/explore?page=2" class="next-link">下一页</a>
            </body></html>
        "#;
        let request = ExploreRequest {
            source_id: "fixture-explore-next".into(),
            source_name: "Explore Next Fixture".into(),
            category_id: None,
            category_title: None,
            screen_id: None,
            screen_title: None,
            page: 1,
            query_parameters: BTreeMap::new(),
        };
        let rule = ExploreHtmlParseRule {
            source_id: request.source_id.clone(),
            source_name: request.source_name.clone(),
            enabled_explore: true,
            book_list: Some(".book-item".into()),
            name: Some(".book-title a@text".into()),
            author: None,
            book_url: Some(".book-title a@href".into()),
            cover_url: None,
            intro: None,
            last_chapter: None,
            kind: None,
            update_time: None,
            tags: None,
            next_page: Some(".pager a.advance@href".into()),
        };

        let with_next = parse_explore_html(
            html_with_configured_next,
            &rule,
            &request,
            Some("https://example.com/explore?page=1"),
            None,
            10,
            false,
        )
        .unwrap();
        assert_eq!(with_next.items[0].title, "Rule Next One");
        assert!(with_next.has_next_page);
        assert_eq!(
            with_next.next_page_url.as_deref(),
            Some("https://example.com/explore?page=2")
        );

        let without_configured_next = parse_explore_html(
            html_with_only_fallback,
            &rule,
            &request,
            Some("https://example.com/explore?page=1"),
            None,
            10,
            false,
        )
        .unwrap();
        assert_eq!(without_configured_next.items[0].title, "Fallback Exists");
        assert!(!without_configured_next.has_next_page);
        assert_eq!(without_configured_next.next_page_url, None);
    }

    #[test]
    fn explore_html_parser_rejects_disabled_or_drifted_rules() {
        let request = ExploreRequest {
            source_id: "disabled".into(),
            source_name: "Disabled".into(),
            category_id: None,
            category_title: None,
            screen_id: None,
            screen_title: None,
            page: 1,
            query_parameters: BTreeMap::new(),
        };
        let disabled = ExploreHtmlParseRule {
            source_id: request.source_id.clone(),
            source_name: request.source_name.clone(),
            enabled_explore: false,
            book_list: Some(".book-item".into()),
            name: Some(".title@text".into()),
            author: None,
            book_url: Some("a@href".into()),
            cover_url: None,
            intro: None,
            last_chapter: None,
            kind: None,
            update_time: None,
            tags: None,
            next_page: None,
        };
        assert_eq!(
            parse_explore_html("<html></html>", &disabled, &request, None, None, 1, false)
                .unwrap_err(),
            RssError::InvalidSubscription {
                field: "enabled_explore".into()
            }
        );

        let missing_book_url = ExploreHtmlParseRule {
            enabled_explore: true,
            book_url: None,
            ..disabled
        };
        assert_eq!(
            parse_explore_html(
                "<html><body><div class='book-item'></div></body></html>",
                &missing_book_url,
                &request,
                None,
                None,
                1,
                false,
            )
            .unwrap_err(),
            RssError::InvalidSubscription {
                field: "book_url".into()
            }
        );
    }

    #[test]
    fn explore_rss_execution_summary_preserves_network_result_envelope() {
        let request = ExploreRequest {
            source_id: "fixture-explore-001".into(),
            source_name: "Fixture Explore Source".into(),
            category_id: Some("cat-fantasy".into()),
            category_title: Some("玄幻".into()),
            screen_id: None,
            screen_title: None,
            page: 2,
            query_parameters: BTreeMap::new(),
        };
        let rule = ExploreRequestRule {
            source_id: request.source_id.clone(),
            source_name: request.source_name.clone(),
            enabled_explore: true,
            explore_url: Some("https://example.com/explore?page={{page}}".into()),
            explore_screen: None,
        };
        let request_spec = build_explore_request_spec(&rule, &request).unwrap();
        let items = vec![
            RssSubscriptionItem {
                title: "斗破苍穹".into(),
                link: "https://example.com/book/1".into(),
                author: Some("天蚕土豆".into()),
                summary: Some("Fixture intro".into()),
                published_at: None,
                source_id: "fixture-explore-001".into(),
                source_name: Some("Fixture Explore Source".into()),
                unknown_fields: BTreeMap::new(),
            },
            RssSubscriptionItem {
                title: "第二本".into(),
                link: "https://example.com/book/2".into(),
                author: None,
                summary: None,
                published_at: None,
                source_id: "fixture-explore-001".into(),
                source_name: Some("Fixture Explore Source".into()),
                unknown_fields: BTreeMap::new(),
            },
        ];

        let summary = summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
            mode: ExploreRssExecutionMode::Network,
            request_spec: Some(request_spec.clone()),
            response_status_code: Some(200),
            final_url: Some("https://example.com/explore?page=2".into()),
            next_page_url: None,
            snapshot_id: None,
            items,
            refresh_decision: None,
            refresh_state: None,
        })
        .unwrap();

        assert_eq!(summary.request_spec, Some(request_spec));
        assert!(summary.network_accessed);
        assert_eq!(summary.response_status_code, Some(200));
        assert_eq!(
            summary.final_url.as_deref(),
            Some("https://example.com/explore?page=2")
        );
        assert!(!summary.replayed_from_local_snapshot);
        assert_eq!(summary.item_count, 2);
        assert_eq!(summary.first_item_title.as_deref(), Some("斗破苍穹"));
        assert!(summary.warnings.is_empty());
    }

    #[test]
    fn explore_rss_execution_summary_preserves_rule_based_rss_next_page_url() {
        let html = r#"
          <html><body>
            <div class="article">
              <a class="title" href="/rss/1">Rule RSS One</a>
              <p class="summary">Rule summary one</p>
            </div>
            <nav class="pager"><a class="next" href="/rss-html?page=2">Next</a></nav>
          </body></html>
        "#;
        let mut source = RssSourceConfig::new("https://example.com/rss-html");
        source.name = Some("Rule RSS".into());
        source.rule_articles = Some(".article".into());
        source.rule_next_page = Some(".pager a.next@href".into());
        source.rule_title = Some(".title@text".into());
        source.rule_description = Some(".summary@text".into());
        source.rule_link = Some(".title@href".into());
        let parsed = parse_rss_rule_html(
            html,
            &source,
            "rule-rss",
            Some("Rule RSS"),
            Some("https://example.com/rss-html"),
            Some(1),
        )
        .unwrap();
        let request_spec = ExploreRequestSpec {
            stage: "discover".into(),
            method: ExploreRequestMethod::GET,
            url_template: "https://example.com/rss-html".into(),
            headers: BTreeMap::new(),
            body_template: None,
            charset: None,
            expected_content_type: Some(ExploreExpectedContentType::Html),
            debug_description: "rss:rule-rss".into(),
            capability_requirements: BTreeSet::from([ExploreRequestCapability::NetworkRequest]),
        };

        let summary = summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
            mode: ExploreRssExecutionMode::Network,
            request_spec: Some(request_spec),
            response_status_code: Some(200),
            final_url: Some("https://example.com/rss-html".into()),
            next_page_url: parsed.next_page_url.clone(),
            snapshot_id: None,
            items: parsed.items,
            refresh_decision: None,
            refresh_state: None,
        })
        .unwrap();

        assert!(summary.network_accessed);
        assert_eq!(summary.item_count, 1);
        assert_eq!(summary.first_item_title.as_deref(), Some("Rule RSS One"));
        assert_eq!(
            summary.next_page_url.as_deref(),
            Some("https://example.com/rss-html?page=2")
        );
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["nextPageUrl"], "https://example.com/rss-html?page=2");
    }

    #[test]
    fn explore_rss_execution_summary_replays_local_snapshot_without_request_spec() {
        let items = vec![RssSubscriptionItem {
            title: "斗破苍穹".into(),
            link: "https://example.com/book/1".into(),
            author: None,
            summary: None,
            published_at: None,
            source_id: "fixture-explore-001".into(),
            source_name: Some("Fixture Explore Source".into()),
            unknown_fields: BTreeMap::new(),
        }];

        let summary = summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
            mode: ExploreRssExecutionMode::LocalSnapshotReplay,
            request_spec: None,
            response_status_code: None,
            final_url: None,
            next_page_url: None,
            snapshot_id: Some("snapshot/explore_list.html".into()),
            items,
            refresh_decision: None,
            refresh_state: None,
        })
        .unwrap();

        assert_eq!(summary.request_spec, None);
        assert!(!summary.network_accessed);
        assert!(summary.replayed_from_local_snapshot);
        assert_eq!(
            summary.snapshot_id.as_deref(),
            Some("snapshot/explore_list.html")
        );
        assert_eq!(summary.item_count, 1);
        assert_eq!(summary.warnings, vec!["replayed_from_local_snapshot"]);
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["networkAccessed"], false);
        assert_eq!(json["replayedFromLocalSnapshot"], true);
        assert_eq!(json["snapshotId"], "snapshot/explore_list.html");
    }

    #[test]
    fn explore_rss_execution_summary_skips_refresh_before_network_when_interval_not_elapsed() {
        let now = 1_781_943_600;
        let previous = RssRefreshState {
            last_fetched_at: Some(now - 1_800),
            last_etag: Some("\"reader-core-etag\"".into()),
            last_modified: Some("Sat, 20 Jun 2026 07:00:00 GMT".into()),
            next_eligible_fetch_at: Some(now + 1_800),
            not_modified: false,
        };
        let decision = decide_rss_refresh(
            &RssRefreshPolicy {
                enabled: true,
                update_interval_minutes: Some(60),
                last_fetched_at: previous.last_fetched_at,
                force_refresh: false,
            },
            now,
        );

        let summary = summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
            mode: ExploreRssExecutionMode::RefreshSkipped,
            request_spec: None,
            response_status_code: None,
            final_url: None,
            next_page_url: None,
            snapshot_id: None,
            items: Vec::new(),
            refresh_decision: Some(decision),
            refresh_state: Some(previous.clone()),
        })
        .unwrap();

        assert!(!summary.network_accessed);
        assert!(!summary.replayed_from_local_snapshot);
        assert_eq!(summary.item_count, 0);
        assert_eq!(summary.refresh_state, Some(previous));
        assert_eq!(summary.warnings, vec!["refresh_skipped_intervalNotElapsed"]);
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["networkAccessed"], false);
        assert_eq!(json["refreshState"]["lastETag"], "\"reader-core-etag\"");
        assert_eq!(
            json["refreshState"]["nextEligibleFetchAt"],
            serde_json::json!(now + 1_800)
        );
    }

    #[test]
    fn explore_rss_execution_summary_handles_http_not_modified_refresh_state() {
        let response_at = 1_781_947_200;
        let previous = RssRefreshState {
            last_fetched_at: Some(response_at - 7_200),
            last_etag: Some("\"reader-core-etag\"".into()),
            last_modified: Some("Sat, 20 Jun 2026 07:00:00 GMT".into()),
            next_eligible_fetch_at: None,
            not_modified: false,
        };
        let response = RssRefreshResponseMetadata {
            status_code: 304,
            response_at,
            headers: BTreeMap::from([
                ("ETag".into(), "\"reader-core-etag-v2\"".into()),
                (
                    "Last-Modified".into(),
                    "Sat, 20 Jun 2026 08:00:00 GMT".into(),
                ),
            ]),
        };
        let state = rss_refresh_state_from_response(&previous, &response, Some(60));
        let request_spec = ExploreRequestSpec {
            stage: "discover".into(),
            method: ExploreRequestMethod::GET,
            url_template: "https://example.com/rss-refresh.xml".into(),
            headers: rss_conditional_refresh_headers(&previous),
            body_template: None,
            charset: None,
            expected_content_type: Some(ExploreExpectedContentType::Xml),
            debug_description: "rss:refresh-rss".into(),
            capability_requirements: BTreeSet::from([ExploreRequestCapability::NetworkRequest]),
        };

        let summary = summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
            mode: ExploreRssExecutionMode::Network,
            request_spec: Some(request_spec.clone()),
            response_status_code: Some(304),
            final_url: Some("https://example.com/rss-refresh.xml".into()),
            next_page_url: None,
            snapshot_id: None,
            items: Vec::new(),
            refresh_decision: None,
            refresh_state: Some(state.clone()),
        })
        .unwrap();

        assert_eq!(summary.request_spec, Some(request_spec));
        assert!(summary.network_accessed);
        assert_eq!(summary.response_status_code, Some(304));
        assert_eq!(summary.item_count, 0);
        assert_eq!(summary.refresh_state, Some(state.clone()));
        assert_eq!(summary.warnings, vec!["not_modified"]);
        assert!(state.not_modified);
        assert_eq!(state.last_fetched_at, Some(response_at));
        assert_eq!(state.last_etag.as_deref(), Some("\"reader-core-etag-v2\""));
        assert_eq!(state.next_eligible_fetch_at, Some(response_at + 3_600));
    }

    #[test]
    fn explore_rss_execution_summary_rejects_drifted_replay_boundaries() {
        let item = RssSubscriptionItem {
            title: "Item".into(),
            link: "https://example.com/book/1".into(),
            author: None,
            summary: None,
            published_at: None,
            source_id: "source".into(),
            source_name: None,
            unknown_fields: BTreeMap::new(),
        };
        let network_missing_status =
            summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
                mode: ExploreRssExecutionMode::Network,
                request_spec: None,
                response_status_code: None,
                final_url: None,
                next_page_url: None,
                snapshot_id: None,
                items: vec![item.clone()],
                refresh_decision: None,
                refresh_state: None,
            })
            .unwrap_err();
        assert_eq!(
            network_missing_status,
            RssError::InvalidSubscription {
                field: "network_execution".into()
            }
        );

        let replay_with_request_spec =
            summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
                mode: ExploreRssExecutionMode::LocalSnapshotReplay,
                request_spec: Some(ExploreRequestSpec {
                    stage: "explore".into(),
                    method: ExploreRequestMethod::GET,
                    url_template: "https://example.com".into(),
                    headers: BTreeMap::new(),
                    body_template: None,
                    charset: None,
                    expected_content_type: None,
                    debug_description: "explore:source:page:1".into(),
                    capability_requirements: BTreeSet::from([
                        ExploreRequestCapability::NetworkRequest,
                    ]),
                }),
                response_status_code: None,
                final_url: None,
                next_page_url: None,
                snapshot_id: Some("snapshot/explore_list.html".into()),
                items: vec![item],
                refresh_decision: None,
                refresh_state: None,
            })
            .unwrap_err();
        assert_eq!(
            replay_with_request_spec,
            RssError::InvalidSubscription {
                field: "local_snapshot_replay".into()
            }
        );

        let request_spec = ExploreRequestSpec {
            stage: "discover".into(),
            method: ExploreRequestMethod::GET,
            url_template: "https://example.com/rss.xml".into(),
            headers: BTreeMap::new(),
            body_template: None,
            charset: None,
            expected_content_type: Some(ExploreExpectedContentType::Xml),
            debug_description: "rss:source".into(),
            capability_requirements: BTreeSet::from([ExploreRequestCapability::NetworkRequest]),
        };
        let invalid_304_with_items =
            summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
                mode: ExploreRssExecutionMode::Network,
                request_spec: Some(request_spec),
                response_status_code: Some(304),
                final_url: Some("https://example.com/rss.xml".into()),
                next_page_url: None,
                snapshot_id: None,
                items: vec![RssSubscriptionItem {
                    title: "Unexpected".into(),
                    link: "https://example.com/item".into(),
                    author: None,
                    summary: None,
                    published_at: None,
                    source_id: "source".into(),
                    source_name: None,
                    unknown_fields: BTreeMap::new(),
                }],
                refresh_decision: None,
                refresh_state: Some(RssRefreshState {
                    last_fetched_at: Some(1),
                    last_etag: None,
                    last_modified: None,
                    next_eligible_fetch_at: None,
                    not_modified: true,
                }),
            })
            .unwrap_err();
        assert_eq!(
            invalid_304_with_items,
            RssError::InvalidSubscription {
                field: "network_not_modified".into()
            }
        );

        let invalid_304_with_next_page =
            summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
                mode: ExploreRssExecutionMode::Network,
                request_spec: Some(ExploreRequestSpec {
                    stage: "discover".into(),
                    method: ExploreRequestMethod::GET,
                    url_template: "https://example.com/rss.xml".into(),
                    headers: BTreeMap::new(),
                    body_template: None,
                    charset: None,
                    expected_content_type: Some(ExploreExpectedContentType::Xml),
                    debug_description: "rss:source".into(),
                    capability_requirements: BTreeSet::from([
                        ExploreRequestCapability::NetworkRequest,
                    ]),
                }),
                response_status_code: Some(304),
                final_url: Some("https://example.com/rss.xml".into()),
                next_page_url: Some("https://example.com/rss.xml?page=2".into()),
                snapshot_id: None,
                items: Vec::new(),
                refresh_decision: None,
                refresh_state: Some(RssRefreshState {
                    last_fetched_at: Some(1),
                    last_etag: None,
                    last_modified: None,
                    next_eligible_fetch_at: None,
                    not_modified: true,
                }),
            })
            .unwrap_err();
        assert_eq!(
            invalid_304_with_next_page,
            RssError::InvalidSubscription {
                field: "network_not_modified".into()
            }
        );

        let invalid_skipped_decision =
            summarize_explore_rss_execution(&ExploreRssExecutionSummaryRequest {
                mode: ExploreRssExecutionMode::RefreshSkipped,
                request_spec: None,
                response_status_code: None,
                final_url: None,
                next_page_url: None,
                snapshot_id: None,
                items: Vec::new(),
                refresh_decision: Some(RssRefreshDecision {
                    should_fetch: true,
                    reason: RssRefreshDecisionReason::Forced,
                    evaluated_at: 10,
                    next_eligible_fetch_at: None,
                }),
                refresh_state: Some(RssRefreshState {
                    last_fetched_at: Some(1),
                    last_etag: None,
                    last_modified: None,
                    next_eligible_fetch_at: None,
                    not_modified: false,
                }),
            })
            .unwrap_err();
        assert_eq!(
            invalid_skipped_decision,
            RssError::InvalidSubscription {
                field: "refresh_skipped".into()
            }
        );
    }

    #[test]
    fn rss_source_config_decodes_legacy_aliases_defaults_and_unknown_fields() {
        let source: RssSourceConfig = serde_json::from_value(serde_json::json!({
            "sourceUrl": "https://example.test/rss.xml",
            "sourceName": "Legacy RSS",
            "updateIntervalMinutes": 30,
            "lastFetchedAt": 1_781_943_600,
            "lastETag": "\"reader-core-etag\"",
            "lastModified": "Sat, 20 Jun 2026 07:00:00 GMT",
            "sourceGroup": "updates",
            "header": " @js:headers() ",
            "ruleArticles": " div.item ",
            "customLegacyFlag": true,
            "nestedLegacy": {"mode": "rss"}
        }))
        .unwrap();

        assert_eq!(source.url, "https://example.test/rss.xml");
        assert_eq!(source.name.as_deref(), Some("Legacy RSS"));
        assert_eq!(source.update_interval_minutes, Some(30));
        assert_eq!(source.last_fetched_at, Some(1_781_943_600));
        assert_eq!(source.last_etag.as_deref(), Some("\"reader-core-etag\""));
        assert_eq!(
            source.last_modified.as_deref(),
            Some("Sat, 20 Jun 2026 07:00:00 GMT")
        );
        assert!(source.enabled);
        assert!(!source.single_url);
        assert_eq!(source.article_style, 0);
        assert!(source.enable_js);
        assert!(source.load_with_base_url);
        assert_eq!(source.source_group.as_deref(), Some("updates"));
        assert!(source.has_rule_based_articles());
        assert!(source.has_dynamic_header_rule());
        assert_eq!(
            source.unknown_fields.get("customLegacyFlag"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            source.unknown_fields.get("nestedLegacy"),
            Some(&serde_json::json!({"mode": "rss"}))
        );
        assert!(!source.unknown_fields.contains_key("sourceUrl"));
        assert!(!source.unknown_fields.contains_key("sourceName"));

        assert_eq!(
            source.refresh_policy(false),
            RssRefreshPolicy {
                enabled: true,
                update_interval_minutes: Some(30),
                last_fetched_at: Some(1_781_943_600),
                force_refresh: false,
            }
        );
    }

    #[test]
    fn rss_source_config_encodes_dual_legacy_keys_and_preserves_unknown_top_level_fields() {
        let mut source = RssSourceConfig::new("https://example.test/rss.xml");
        source.name = Some("Encoded RSS".into());
        source.enabled = false;
        source.update_interval_minutes = Some(45);
        source.single_url = true;
        source.article_style = 2;
        source.enable_js = false;
        source.load_with_base_url = false;
        source.rule_articles = Some("  ".into());
        source.header = Some("<js>headers()</js>".into());
        source
            .unknown_fields
            .insert("customLegacyFlag".into(), serde_json::json!({"kept": true}));
        source
            .unknown_fields
            .insert("sourceUrl".into(), serde_json::json!("stale-ignored"));

        let json = serde_json::to_value(&source).unwrap();

        assert_eq!(json["url"], "https://example.test/rss.xml");
        assert_eq!(json["sourceUrl"], "https://example.test/rss.xml");
        assert_eq!(json["name"], "Encoded RSS");
        assert_eq!(json["sourceName"], "Encoded RSS");
        assert_eq!(json["enabled"], false);
        assert_eq!(json["updateIntervalMinutes"], 45);
        assert_eq!(json["singleUrl"], true);
        assert_eq!(json["articleStyle"], 2);
        assert_eq!(json["enableJs"], false);
        assert_eq!(json["loadWithBaseUrl"], false);
        assert_eq!(json["customLegacyFlag"], serde_json::json!({"kept": true}));
        assert!(source.has_dynamic_header_rule());
        assert!(!source.has_rule_based_articles());

        let mut expected_round_trip = source;
        expected_round_trip.unknown_fields.remove("sourceUrl");
        let round_trip: RssSourceConfig = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip, expected_round_trip);
    }

    #[test]
    fn rss_source_config_rejects_drifted_legacy_field_types() {
        let err = serde_json::from_value::<RssSourceConfig>(serde_json::json!({
            "sourceUrl": "https://example.test/rss.xml",
            "updateIntervalMinutes": -1
        }))
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("u32 out of range for RSS source field updateIntervalMinutes"));

        let err = serde_json::from_value::<RssSourceConfig>(serde_json::json!({
            "sourceUrl": "https://example.test/rss.xml",
            "enableJs": "yes"
        }))
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("expected bool for RSS source field enableJs"));
    }

    #[test]
    fn rss_runtime_capability_wire_values_match_legacy_core_boundary_names() {
        let cases = [
            (RssRuntimeCapability::CookieJar, "cookieJar"),
            (RssRuntimeCapability::Login, "login"),
            (RssRuntimeCapability::Javascript, "javascript"),
            (RssRuntimeCapability::WebView, "webView"),
        ];

        for (capability, expected_wire_value) in cases {
            let json = serde_json::to_string(&capability).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<RssRuntimeCapability>(&json).unwrap(),
                capability
            );
            assert_eq!(
                rss_runtime_capability_wire_value(capability),
                expected_wire_value
            );
        }
    }

    #[test]
    fn rss_source_runtime_capabilities_match_legacy_core_handoff_rules() {
        let mut source = RssSourceConfig::new("https://example.test/rss.xml");
        source.enabled_cookie_jar = Some(true);
        source.login_url = Some("https://example.test/login".into());
        source.login_ui = Some("username,password".into());
        source.login_check_js = Some("document.body.innerText.includes('ok')".into());
        source.cover_decode_js = Some("java.decodeCover()".into());
        source.inject_js = Some("window.injected = true".into());
        source.should_override_url_loading = Some("webview.shouldOverrideUrlLoading".into());

        assert_eq!(
            rss_source_runtime_capabilities(&source)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                RssRuntimeCapability::CookieJar,
                RssRuntimeCapability::Login,
                RssRuntimeCapability::Javascript,
                RssRuntimeCapability::WebView
            ]
        );
        assert_eq!(
            unsupported_rss_rule_runtime_capabilities(&source),
            vec![
                RssRuntimeCapability::Javascript,
                RssRuntimeCapability::WebView,
                RssRuntimeCapability::Login
            ]
        );

        let mut rule_only = RssSourceConfig::new("https://example.test/rss.xml");
        rule_only.rule_content = Some(".content@text##@js: document.querySelector('main')".into());
        assert_eq!(rss_source_runtime_capabilities(&rule_only), BTreeSet::new());
        assert_eq!(
            unsupported_rss_rule_runtime_capabilities(&rule_only),
            vec![RssRuntimeCapability::Javascript]
        );

        let mut cookie_only = RssSourceConfig::new("https://example.test/rss.xml");
        cookie_only.enabled_cookie_jar = Some(true);
        assert_eq!(
            rss_source_runtime_capabilities(&cookie_only)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![RssRuntimeCapability::CookieJar]
        );
        assert!(unsupported_rss_rule_runtime_capabilities(&cookie_only).is_empty());
    }

    #[test]
    fn rss_rule_html_parser_extracts_items_next_page_and_limit_like_legacy_runtime() {
        let html = r#"
          <html><body>
            <div class="article">
              <a class="title" href="/rss/1">Rule RSS One</a>
              <span class="date">2025-12-25</span>
              <p class="summary">Rule summary &amp; one</p>
              <img src="/covers/one.jpg" />
              <div class="content"><p>Rule content one</p></div>
            </div>
            <div class="article">
              <a class="title" href="/rss/2">Rule RSS Two</a>
              <span class="date">2025-12-26</span>
              <p class="summary">Rule summary two</p>
              <img src="/covers/two.jpg" />
              <div class="content"><p>Rule content two</p></div>
            </div>
            <nav class="pager"><a class="next" href="/rss-html?page=2">Next</a></nav>
          </body></html>
        "#;
        let mut source = RssSourceConfig::new("https://example.com/rss-html");
        source.name = Some("Rule RSS".into());
        source.source_group = Some("updates".into());
        source.rule_articles = Some(".article".into());
        source.rule_next_page = Some(".pager a.next@href".into());
        source.rule_title = Some(".title@text".into());
        source.rule_pub_date = Some(".date@text".into());
        source.rule_description = Some(".summary@text".into());
        source.rule_image = Some("img@src".into());
        source.rule_link = Some(".title@href".into());
        source.rule_content = Some(".content@html".into());

        let result = parse_rss_rule_html(
            html,
            &source,
            "rule-rss",
            Some("Rule RSS"),
            Some("https://example.com/rss-html"),
            Some(1),
        )
        .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.next_page_url.as_deref(),
            Some("https://example.com/rss-html?page=2")
        );
        let item = &result.items[0];
        assert_eq!(item.title, "Rule RSS One");
        assert_eq!(item.link, "https://example.com/rss/1");
        assert_eq!(item.summary.as_deref(), Some("Rule summary & one"));
        assert_eq!(item.published_at.as_deref(), Some("2025-12-25"));
        assert_eq!(item.source_id, "rule-rss");
        assert_eq!(item.source_name.as_deref(), Some("Rule RSS"));
        assert_eq!(
            item.unknown_fields["image"],
            "https://example.com/covers/one.jpg"
        );
        assert_eq!(item.unknown_fields["group"], "updates");
        assert_eq!(item.unknown_fields["rssRuleMode"], "non_js_rule_articles");
        assert_eq!(item.unknown_fields["pubDateRaw"], "2025-12-25");
        assert_eq!(item.unknown_fields["content"], "<p>Rule content one</p>");
    }

    #[test]
    fn rss_rule_html_parser_reverses_synthesizes_page_and_gates_dynamic_rules() {
        let html = r#"
          <main>
            <article class="entry"><h2>First</h2></article>
            <article class="entry"><h2>Second</h2></article>
          </main>
        "#;
        let mut source = RssSourceConfig::new("https://example.com/rss-html?page=1");
        source.rule_articles = Some("-css:article.entry".into());
        source.rule_title = Some("h2".into());
        source.rule_next_page = Some("PAGE".into());

        let result = parse_rss_rule_html(
            html,
            &source,
            "rule-rss",
            None,
            Some("https://example.com/rss-html?page=1"),
            None,
        )
        .unwrap();

        assert_eq!(
            result
                .items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Second", "First"]
        );
        assert_eq!(
            result.next_page_url.as_deref(),
            Some("https://example.com/rss-html?page=1")
        );
        assert!(result.items[0].link.starts_with("rss-rule://"));
        assert_ne!(result.items[0].link, result.items[1].link);

        let mut missing_articles = RssSourceConfig::new("https://example.com/rss-html");
        missing_articles.rule_title = Some("h2".into());
        assert_eq!(
            parse_rss_rule_html(html, &missing_articles, "rule-rss", None, None, None).unwrap_err(),
            RssError::MissingField {
                field: "rss_rule.rule_articles".into()
            }
        );

        let mut dynamic = source;
        dynamic.rule_title = Some("@js:java.get('title')".into());
        dynamic.login_url = Some("https://example.com/login".into());
        assert_eq!(
            parse_rss_rule_html(html, &dynamic, "rule-rss", None, None, None).unwrap_err(),
            RssError::UnsupportedRuntimeCapabilities {
                capabilities: vec![
                    RssRuntimeCapability::Javascript,
                    RssRuntimeCapability::Login
                ]
            }
        );
    }

    #[test]
    fn rss_source_request_headers_match_legacy_merge_order_and_dynamic_header_boundary() {
        let mut source = RssSourceConfig::new("https://example.test/rss.xml");
        source.header = Some(
            r#"{"User-Agent":"ReaderCore","If-None-Match":"source-etag","X-Base":"base"}"#.into(),
        );
        source.last_etag = Some(r#""reader-core-etag""#.into());
        source.last_modified = Some("Sat, 20 Jun 2026 07:00:00 GMT".into());
        let additional = BTreeMap::from([
            ("User-Agent".into(), "OverrideAgent".into()),
            ("X-Extra".into(), "extra".into()),
        ]);

        let headers = rss_source_request_headers(&source, &additional).unwrap();

        assert_eq!(
            headers.get("User-Agent").map(String::as_str),
            Some("OverrideAgent")
        );
        assert_eq!(headers.get("X-Base").map(String::as_str), Some("base"));
        assert_eq!(headers.get("X-Extra").map(String::as_str), Some("extra"));
        assert_eq!(
            headers.get(RSS_HEADER_IF_NONE_MATCH).map(String::as_str),
            Some(r#""reader-core-etag""#)
        );
        assert_eq!(
            headers
                .get(RSS_HEADER_IF_MODIFIED_SINCE)
                .map(String::as_str),
            Some("Sat, 20 Jun 2026 07:00:00 GMT")
        );

        let mut malformed_static = RssSourceConfig::new("https://example.test/rss.xml");
        malformed_static.header = Some(r#"{"User-Agent":42}"#.into());
        assert!(rss_source_static_headers(&malformed_static)
            .unwrap()
            .is_empty());

        let mut dynamic_header = RssSourceConfig::new("https://example.test/rss.xml");
        dynamic_header.header = Some(" @js:headers() ".into());
        assert_eq!(
            rss_source_static_headers(&dynamic_header).unwrap_err(),
            RssError::UnsupportedRuntimeCapabilities {
                capabilities: vec![RssRuntimeCapability::Javascript]
            }
        );
    }

    #[test]
    fn rss_product_gated_book_source_projection_matches_legacy_runtime_fields() {
        let mut source = RssSourceConfig::new("https://example.test:8443/rss.xml?section=fiction");
        source.name = Some("Legacy RSS".into());
        source.source_group = Some("fiction-updates".into());
        source.custom_order = Some(42);
        source.source_comment = Some("Imported from legacy Reader-Core".into());
        source.source_icon = Some("https://example.test/icon.png".into());
        source.variable_comment = Some("token={{secret}}".into());
        source.js_lib = Some("legacy-lib.js".into());
        source.concurrent_rate = Some("2".into());
        source.last_update_time = Some(1_781_943_600);
        source.login_check_js = Some("checkLogin()".into());
        source.header =
            Some(r#"{"User-Agent":"ReaderCore","X-Base":"base","If-None-Match":"stale"}"#.into());
        source.last_etag = Some(r#""reader-core-etag""#.into());
        source.last_modified = Some("Sat, 20 Jun 2026 07:00:00 GMT".into());
        source.login_url = Some("https://example.test/login".into());
        source.login_ui = Some("web".into());
        source.enabled_cookie_jar = Some(true);
        source.cover_decode_js = Some("decodeCover()".into());
        source.sort_url = Some("https://example.test/rss-sort".into());
        source.single_url = true;
        source.article_style = 2;
        source.should_override_url_loading = Some("shouldOverride(url)".into());
        source.style = Some("body { color: #111; }".into());
        source.enable_js = false;
        source.load_with_base_url = false;
        source.inject_js = Some("window.__reader = true;".into());
        source
            .unknown_fields
            .insert("vendorFlag".into(), serde_json::json!({"kept": true}));
        let additional_headers = BTreeMap::from([
            ("User-Agent".into(), "OverrideAgent".into()),
            ("X-Extra".into(), "extra".into()),
        ]);

        let projection = project_rss_source_to_product_gated_book_source(
            &source,
            Some("rss-source-legacy"),
            &additional_headers,
        )
        .unwrap();

        assert_eq!(projection.id, "rss-source-legacy");
        assert_eq!(projection.book_source_name, "Legacy RSS");
        assert_eq!(
            projection.book_source_url.as_deref(),
            Some("https://example.test:8443")
        );
        assert_eq!(
            projection.header.get("User-Agent").map(String::as_str),
            Some("OverrideAgent")
        );
        assert_eq!(
            projection
                .header
                .get(RSS_HEADER_IF_NONE_MATCH)
                .map(String::as_str),
            Some(r#""reader-core-etag""#)
        );
        assert_eq!(
            projection
                .header
                .get(RSS_HEADER_IF_MODIFIED_SINCE)
                .map(String::as_str),
            Some("Sat, 20 Jun 2026 07:00:00 GMT")
        );
        assert!(projection.enabled_explore);
        assert!(projection.enabled_cookie_jar);
        assert!(projection.web_view);

        let json = serde_json::to_value(&projection).unwrap();
        assert_eq!(json["bookSourceGroup"], "fiction-updates");
        assert_eq!(
            json["bookSourceComment"],
            "Imported from legacy Reader-Core"
        );
        assert_eq!(json["jsLib"], "legacy-lib.js");
        assert_eq!(json["lastUpdateTime"], "1781943600");
        assert_eq!(
            json["exploreUrl"],
            "https://example.test:8443/rss.xml?section=fiction"
        );
        assert_eq!(json["header"]["X-Extra"], "extra");
        assert_eq!(
            json["unknownFields"]["rssSortUrl"],
            "https://example.test/rss-sort"
        );
        assert_eq!(json["unknownFields"]["rssSingleUrl"], true);
        assert_eq!(json["unknownFields"]["rssArticleStyle"], 2);
        assert_eq!(
            json["unknownFields"]["shouldOverrideUrlLoading"],
            "shouldOverride(url)"
        );
        assert_eq!(json["unknownFields"]["style"], "body { color: #111; }");
        assert_eq!(json["unknownFields"]["enableJs"], false);
        assert_eq!(json["unknownFields"]["loadWithBaseUrl"], false);
        assert_eq!(json["unknownFields"]["injectJs"], "window.__reader = true;");
        assert_eq!(
            json["unknownFields"]["vendorFlag"],
            serde_json::json!({"kept": true})
        );
        let decoded: RssProductGatedBookSource = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, projection);
    }

    #[test]
    fn rss_source_cookie_login_capability_contract_matches_legacy_fixture_artifact() {
        let mut source = RssSourceConfig::new("https://example.com/rss.xml");
        source.name = Some("Authenticated RSS".into());
        source.enabled_cookie_jar = Some(true);
        source.login_url = Some("https://example.com/login".into());
        source.login_check_js = Some("document.body.innerText.includes('ok')".into());
        source.inject_js = Some("window.__rss = true;".into());
        source.should_override_url_loading = Some("shouldOverride(url)".into());
        source.style = Some("body { color: #222; }".into());

        let contract = build_rss_source_cookie_login_capability_contract(
            &source,
            Some("rss-source-handoff"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert_eq!(
            contract.authorization_scope,
            RssSourceAuthorizationScope::FullAccess
        );
        assert_eq!(
            contract.login_execution_boundary,
            RssLoginExecutionBoundary::UnsupportedWithoutProductGatedHost
        );
        assert_eq!(
            contract.capability_requirements,
            BTreeSet::from([
                RssSourceCapabilityRequirement::NetworkRequest,
                RssSourceCapabilityRequirement::CookieJar,
                RssSourceCapabilityRequirement::Login,
                RssSourceCapabilityRequirement::Javascript,
                RssSourceCapabilityRequirement::WebView
            ])
        );
        assert_eq!(
            contract.product_gated_handoff.source_login_url.as_deref(),
            Some("https://example.com/login")
        );
        assert_eq!(
            contract
                .product_gated_handoff
                .source_login_check_js
                .as_deref(),
            Some("document.body.innerText.includes('ok')")
        );
        assert!(contract.product_gated_handoff.source_enabled_cookie_jar);
        assert_eq!(
            contract.product_gated_handoff.unknown_field_keys,
            vec![
                "enableJs",
                "injectJs",
                "loadWithBaseUrl",
                "rssArticleStyle",
                "shouldOverrideUrlLoading",
                "style"
            ]
        );

        let json = serde_json::to_value(&contract).unwrap();
        assert_eq!(json["authorizationScope"], "fullAccess");
        assert_eq!(
            json["loginExecutionBoundary"],
            "unsupported_without_product_gated_host"
        );
        assert_eq!(
            json["capabilityRequirements"],
            serde_json::json!([
                "networkRequest",
                "cookieJar",
                "login",
                "javascript",
                "webView"
            ])
        );
        assert_eq!(
            json["productGatedHandoff"]["sourceLoginUrl"],
            "https://example.com/login"
        );
        assert_eq!(json["productGatedHandoff"]["sourceEnabledCookieJar"], true);
        assert_eq!(
            serde_json::from_value::<RssSourceCookieLoginCapabilityContract>(json).unwrap(),
            contract
        );
        assert!(
            serde_json::from_value::<RssSourceCookieLoginCapabilityContract>(serde_json::json!({
                "authorizationScope": "fullAccess",
                "loginExecutionBoundary": "unsupported_without_product_gated_host",
                "capabilityRequirements": ["networkRequest"],
                "productGatedHandoff": {
                    "sourceEnabledCookieJar": true,
                    "unexpected": true
                }
            }))
            .is_err()
        );
        let mut missing_network = contract.clone();
        missing_network
            .capability_requirements
            .remove(&RssSourceCapabilityRequirement::NetworkRequest);
        assert_eq!(
            missing_network.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "capability_requirements".into()
            }
        );
    }

    #[test]
    fn rss_product_gated_unknown_fields_preserve_legacy_overrides_and_empty_boundaries() {
        let mut source = RssSourceConfig::new("https://example.test/rss.xml");
        source.name = Some("Fallback Id".into());
        source.sort_url = Some("  ".into());
        source.article_style = 1;
        source.should_override_url_loading = Some(" ".into());
        source.style = Some("\n\t".into());
        source.inject_js = Some("  ".into());
        source
            .unknown_fields
            .insert("rssArticleStyle".into(), serde_json::json!(9));
        source
            .unknown_fields
            .insert("rssSingleUrl".into(), serde_json::json!(false));
        source
            .unknown_fields
            .insert("legacyFlag".into(), serde_json::json!("kept"));

        let fields = rss_product_gated_unknown_fields(&source);

        assert!(!fields.contains_key("rssSortUrl"));
        assert!(!fields.contains_key("shouldOverrideUrlLoading"));
        assert!(!fields.contains_key("style"));
        assert!(!fields.contains_key("injectJs"));
        assert_eq!(fields.get("rssArticleStyle"), Some(&serde_json::json!(9)));
        assert_eq!(fields.get("rssSingleUrl"), Some(&serde_json::json!(false)));
        assert_eq!(fields.get("enableJs"), Some(&serde_json::json!(true)));
        assert_eq!(
            fields.get("loadWithBaseUrl"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(fields.get("legacyFlag"), Some(&serde_json::json!("kept")));

        let projection =
            project_rss_source_to_product_gated_book_source(&source, None, &BTreeMap::new())
                .unwrap();
        assert_eq!(projection.id, "Fallback Id");

        let missing_url = RssSourceConfig::new("  ");
        assert_eq!(
            project_rss_source_to_product_gated_book_source(
                &missing_url,
                Some("empty-url-source"),
                &BTreeMap::new()
            )
            .unwrap_err(),
            RssError::MissingField {
                field: "url".into(),
            }
        );
    }

    #[test]
    fn rss_subscription_source_config_decodes_legacy_fixture_shape() {
        let source: RssSubscriptionSourceConfig = serde_json::from_value(serde_json::json!({
            "id": "sub-f0c62d1e",
            "url": "https://example.test/rss",
            "name": "Example RSS Feed",
            "sourceGroup": "fiction-updates",
            "updateIntervalMinutes": 60,
            "lastFetchedAt": 1_781_943_600,
            "lastETag": "\"reader-core-subscription-etag\"",
            "lastModified": "Sat, 20 Jun 2026 07:30:00 GMT",
            "enabled": true,
            "replaceRules": [
                "replace(/ad/g, '')",
                "replace(/typo/g, 'fixed')"
            ],
            "unknownFields": {
                "fetchTimeout": 30000,
                "maxItems": 20
            },
            "ignoredTopLevelExtension": "swift-codable-ignored"
        }))
        .unwrap();

        assert_eq!(source.id, "sub-f0c62d1e");
        assert_eq!(source.url, "https://example.test/rss");
        assert_eq!(source.name.as_deref(), Some("Example RSS Feed"));
        assert_eq!(source.source_group.as_deref(), Some("fiction-updates"));
        assert_eq!(source.update_interval_minutes, Some(60));
        assert_eq!(source.last_fetched_at, Some(1_781_943_600));
        assert_eq!(
            source.last_etag.as_deref(),
            Some("\"reader-core-subscription-etag\"")
        );
        assert_eq!(
            source.last_modified.as_deref(),
            Some("Sat, 20 Jun 2026 07:30:00 GMT")
        );
        assert!(source.enabled);
        assert!(source.has_replace_rules());
        assert_eq!(source.replace_rules.as_ref().unwrap().len(), 2);
        assert_eq!(
            source.unknown_fields.get("fetchTimeout"),
            Some(&serde_json::json!(30000))
        );
        assert_eq!(
            source.unknown_fields.get("maxItems"),
            Some(&serde_json::json!(20))
        );
        assert!(!source
            .unknown_fields
            .contains_key("ignoredTopLevelExtension"));
        source.validate().unwrap();

        assert_eq!(
            source.refresh_policy(false),
            RssRefreshPolicy {
                enabled: true,
                update_interval_minutes: Some(60),
                last_fetched_at: Some(1_781_943_600),
                force_refresh: false,
            }
        );
    }

    #[test]
    fn rss_subscription_source_config_round_trips_and_projects_refresh_state() {
        let mut source = RssSubscriptionSourceConfig::new(
            "subscription-refresh-contract",
            "https://example.test/rss",
        );
        source.name = Some("Refresh Contract Subscription".into());
        source.source_group = Some("updates".into());
        source.update_interval_minutes = Some(30);
        source.last_fetched_at = Some(1_781_947_800);
        source.last_etag = Some("\"reader-core-subscription-etag\"".into());
        source.last_modified = Some("Sat, 20 Jun 2026 07:30:00 GMT".into());
        source.replace_rules = Some(vec!["replace(/ad/g, '')".into()]);
        source
            .unknown_fields
            .insert("fetchTimeout".into(), serde_json::json!(30000));

        let json = serde_json::to_value(&source).unwrap();

        assert_eq!(json["id"], "subscription-refresh-contract");
        assert_eq!(json["url"], "https://example.test/rss");
        assert_eq!(json["sourceGroup"], "updates");
        assert_eq!(json["updateIntervalMinutes"], 30);
        assert_eq!(json["lastFetchedAt"], 1_781_947_800);
        assert_eq!(json["lastETag"], "\"reader-core-subscription-etag\"");
        assert_eq!(json["lastModified"], "Sat, 20 Jun 2026 07:30:00 GMT");
        assert_eq!(json["enabled"], true);
        assert_eq!(
            json["replaceRules"],
            serde_json::json!(["replace(/ad/g, '')"])
        );
        assert_eq!(json["unknownFields"]["fetchTimeout"], 30000);

        let round_trip: RssSubscriptionSourceConfig = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip, source);
        assert_eq!(
            round_trip.refresh_state(),
            RssRefreshState {
                last_fetched_at: Some(1_781_947_800),
                last_etag: Some("\"reader-core-subscription-etag\"".into()),
                last_modified: Some("Sat, 20 Jun 2026 07:30:00 GMT".into()),
                next_eligible_fetch_at: Some(1_781_949_600),
                not_modified: false,
            }
        );
        assert_eq!(
            rss_conditional_refresh_headers(&round_trip.refresh_state())
                .get(RSS_HEADER_IF_NONE_MATCH)
                .map(String::as_str),
            Some("\"reader-core-subscription-etag\"")
        );
    }

    #[test]
    fn rss_subscription_source_config_rejects_drifted_types_and_invalid_state() {
        let err = serde_json::from_value::<RssSubscriptionSourceConfig>(serde_json::json!({
            "id": "sub",
            "url": "https://example.test/rss",
            "replaceRules": "replace(/ad/g, '')"
        }))
        .unwrap_err();
        assert!(err.to_string().contains("invalid type"));

        let mut source = RssSubscriptionSourceConfig::new("sub", "https://example.test/rss");
        source.replace_rules = Some(vec!["replace(/ad/g, '')".into(), "  ".into()]);
        assert_eq!(
            source.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "replace_rules".into(),
            }
        );

        source.replace_rules = None;
        source.url = "  ".into();
        assert_eq!(
            source.validate().unwrap_err(),
            RssError::InvalidSubscription {
                field: "url".into(),
            }
        );
    }

    #[test]
    fn parse_subscription_items_applies_legacy_replace_rules_and_limit() {
        let mut source =
            RssSubscriptionSourceConfig::new("source-rss", "https://example.test/rss.xml");
        source.name = Some("Source Name".into());
        source.replace_rules = Some(vec![
            "Sponsored =>".into(),
            "AD=>Clean".into(),
            "replace(/chapter\\s+(\\d+)/i, 'Episode $1')".into(),
            "not-a-rule".into(),
        ]);
        let xml = r#"
            <rss><channel>
                <title>Feed</title>
                <item>
                    <guid>item-1</guid>
                    <title>  Sponsored Chapter 12  </title>
                    <link> https://example.test/item-1 </link>
                    <description> AD Summary </description>
                </item>
                <item>
                    <guid>item-2</guid>
                    <title>Chapter 13</title>
                    <link>https://example.test/item-2</link>
                </item>
            </channel></rss>
        "#;

        let items = parse_subscription_items(xml, &source, Some(1)).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Episode 12");
        assert_eq!(items[0].link, "https://example.test/item-1");
        assert_eq!(items[0].summary.as_deref(), Some("Clean Summary"));
        assert_eq!(items[0].source_id, "source-rss");
        assert_eq!(items[0].source_name.as_deref(), Some("Source Name"));
        assert_eq!(
            items[0].unknown_fields.get("entryId"),
            Some(&serde_json::json!("item-1"))
        );

        let json = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(json["title"], "Episode 12");
        assert_eq!(json["summary"], "Clean Summary");
        assert_eq!(json["sourceId"], "source-rss");
    }

    #[test]
    fn rss_subscription_replace_rules_cover_author_and_regex_boundaries() {
        let mut item = RssSubscriptionItem::new("Sponsored Item", "https://example.test/item");
        item.author = Some("AD Author".into());
        item.summary = Some("Intro\nSecret\nOutro".into());
        let rules = vec![
            "Sponsored =>".into(),
            "AD =>Clean ".into(),
            "replace(/^secret$/im, 'Visible')".into(),
            "replace(/unclosed, 'ignored')".into(),
            "=>ignored".into(),
        ];

        let items = apply_rss_subscription_replace_rules(&[item], &rules);

        assert_eq!(items[0].title, "Item");
        assert_eq!(items[0].author.as_deref(), Some("CleanAuthor"));
        assert_eq!(items[0].summary.as_deref(), Some("Intro\nVisible\nOutro"));

        assert_eq!(
            apply_rss_replace_rules("A\nb", &["replace(/^b$/m, 'B')".into()]),
            "A\nB"
        );
        assert_eq!(
            apply_rss_replace_rules("a\nb", &["replace(/a.b/s, 'x')".into()]),
            "x"
        );
        assert_eq!(
            apply_rss_replace_rules("Kept", &["  ".into(), "invalid".into()]),
            "Kept"
        );
    }

    #[test]
    fn rss_subscription_item_round_trips_legacy_dto_shape_and_defaults() {
        let mut item = RssSubscriptionItem::new("Chapter 1", "https://example.test/ch1");
        item.author = Some("Author".into());
        item.summary = Some("Summary text".into());
        item.published_at = Some("Tue, 14 Nov 2023 22:13:20 GMT".into());
        item.source_id = "src1".into();
        item.source_name = Some("Source".into());
        item.unknown_fields
            .insert("category".into(), serde_json::json!("updates"));

        let json = serde_json::to_value(&item).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "title": "Chapter 1",
                "link": "https://example.test/ch1",
                "author": "Author",
                "summary": "Summary text",
                "publishedAt": "Tue, 14 Nov 2023 22:13:20 GMT",
                "sourceId": "src1",
                "sourceName": "Source",
                "unknownFields": {
                    "category": "updates"
                }
            })
        );
        let decoded: RssSubscriptionItem = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, item);
        decoded.validate().unwrap();

        let default_item: RssSubscriptionItem = serde_json::from_value(serde_json::json!({
            "title": "T",
            "link": "https://example.test/item",
            "ignoredTopLevelExtension": true
        }))
        .unwrap();
        assert_eq!(default_item.source_id, "");
        assert_eq!(default_item.author, None);
        assert_eq!(default_item.published_at, None);
        assert_eq!(default_item.source_name, None);
        assert!(default_item.unknown_fields.is_empty());
    }

    #[test]
    fn rss_subscription_item_projects_parser_entry_with_source_metadata() {
        let entry = RssEntry {
            id: "urn:uuid:item-1".into(),
            title: "Item One".into(),
            link: Some("https://example.test/item-1".into()),
            summary: Some("First item".into()),
            published_at: Some("Fri, 03 Jan 2026 00:00:00 GMT".into()),
            unknown_fields: BTreeMap::new(),
        };

        let item =
            RssSubscriptionItem::from_entry(&entry, "rss-source-1", Some("Source Name".into()));

        assert_eq!(item.title, "Item One");
        assert_eq!(item.link, "https://example.test/item-1");
        assert_eq!(item.summary.as_deref(), Some("First item"));
        assert_eq!(
            item.published_at.as_deref(),
            Some("Fri, 03 Jan 2026 00:00:00 GMT")
        );
        assert_eq!(item.source_id, "rss-source-1");
        assert_eq!(item.source_name.as_deref(), Some("Source Name"));
        assert_eq!(
            item.unknown_fields.get("entryId"),
            Some(&serde_json::json!("urn:uuid:item-1"))
        );
        item.validate().unwrap();

        let without_link = RssEntry {
            link: None,
            ..entry
        };
        let item = RssSubscriptionItem::from_entry(&without_link, "rss-source-1", None);
        assert_eq!(item.link, "urn:uuid:item-1");
    }

    #[test]
    fn rss_subscription_item_rejects_missing_required_dto_fields() {
        let err = RssSubscriptionItem::new("  ", "https://example.test/item")
            .validate()
            .unwrap_err();
        assert_eq!(
            err,
            RssError::MissingField {
                field: "subscription_item.title".into(),
            }
        );

        let err = RssSubscriptionItem::new("Item", " ")
            .validate()
            .unwrap_err();
        assert_eq!(
            err,
            RssError::MissingField {
                field: "subscription_item.link".into(),
            }
        );

        let err = serde_json::from_value::<RssSubscriptionItem>(serde_json::json!({
            "title": "Item",
            "link": 42
        }))
        .unwrap_err();
        assert!(err.to_string().contains("invalid type"));
    }

    #[test]
    fn refresh_decision_state_machine_matches_legacy_reader_core_order() {
        let now = 1_781_943_600;

        let disabled = decide_rss_refresh(
            &refresh_policy(false, Some(60), Some(now - 7_200), true),
            now,
        );
        assert_eq!(
            disabled,
            RssRefreshDecision {
                should_fetch: false,
                reason: RssRefreshDecisionReason::Disabled,
                evaluated_at: now,
                next_eligible_fetch_at: None
            }
        );

        let forced = decide_rss_refresh(&refresh_policy(true, Some(60), Some(now - 30), true), now);
        assert_eq!(forced.reason, RssRefreshDecisionReason::Forced);
        assert!(forced.should_fetch);
        assert_eq!(forced.next_eligible_fetch_at, None);

        let missing_interval =
            decide_rss_refresh(&refresh_policy(true, None, Some(now - 30), false), now);
        assert_eq!(
            missing_interval.reason,
            RssRefreshDecisionReason::MissingUpdateInterval
        );
        assert!(missing_interval.should_fetch);

        let zero_interval =
            decide_rss_refresh(&refresh_policy(true, Some(0), Some(now - 30), false), now);
        assert_eq!(
            zero_interval.reason,
            RssRefreshDecisionReason::MissingUpdateInterval
        );
        assert!(zero_interval.should_fetch);

        let missing_last_fetch =
            decide_rss_refresh(&refresh_policy(true, Some(60), None, false), now);
        assert_eq!(
            missing_last_fetch.reason,
            RssRefreshDecisionReason::MissingLastFetchedAt
        );
        assert!(missing_last_fetch.should_fetch);

        let elapsed = decide_rss_refresh(
            &refresh_policy(true, Some(60), Some(now - 3_600), false),
            now,
        );
        assert_eq!(elapsed.reason, RssRefreshDecisionReason::IntervalElapsed);
        assert!(elapsed.should_fetch);
        assert_eq!(elapsed.next_eligible_fetch_at, Some(now));

        let not_elapsed = decide_rss_refresh(
            &refresh_policy(true, Some(60), Some(now - 1_800), false),
            now,
        );
        assert_eq!(
            not_elapsed.reason,
            RssRefreshDecisionReason::IntervalNotElapsed
        );
        assert!(!not_elapsed.should_fetch);
        assert_eq!(not_elapsed.next_eligible_fetch_at, Some(now + 1_800));
    }

    #[test]
    fn conditional_refresh_headers_match_legacy_reader_core_contract() {
        let state = RssRefreshState {
            last_fetched_at: Some(1_781_943_600),
            last_etag: Some("  \"reader-core-etag\" \n".into()),
            last_modified: Some("\nSat, 20 Jun 2026 07:00:00 GMT  ".into()),
            next_eligible_fetch_at: None,
            not_modified: false,
        };

        let headers = rss_conditional_refresh_headers(&state);

        assert_eq!(
            headers.get(RSS_HEADER_IF_NONE_MATCH).map(String::as_str),
            Some("\"reader-core-etag\"")
        );
        assert_eq!(
            headers
                .get(RSS_HEADER_IF_MODIFIED_SINCE)
                .map(String::as_str),
            Some("Sat, 20 Jun 2026 07:00:00 GMT")
        );
        assert_eq!(
            serde_json::to_value(&headers).unwrap(),
            serde_json::json!({
                "If-Modified-Since": "Sat, 20 Jun 2026 07:00:00 GMT",
                "If-None-Match": "\"reader-core-etag\""
            })
        );

        assert!(rss_conditional_refresh_headers_from_parts(Some(" \n "), Some("")).is_empty());
    }

    #[test]
    fn refresh_state_from_response_matches_legacy_reader_core_cache_update() {
        let previous = RssRefreshState {
            last_fetched_at: Some(1_781_940_000),
            last_etag: Some("\"reader-core-etag\"".into()),
            last_modified: Some("Sat, 20 Jun 2026 07:00:00 GMT".into()),
            next_eligible_fetch_at: Some(1_781_943_600),
            not_modified: false,
        };
        let response = RssRefreshResponseMetadata {
            status_code: 304,
            response_at: 1_781_947_200,
            headers: BTreeMap::from([
                ("etag".into(), "  \"reader-core-etag-v2\"  ".into()),
                (
                    "LAST-MODIFIED".into(),
                    " Sat, 20 Jun 2026 08:00:00 GMT ".into(),
                ),
            ]),
        };

        let state = rss_refresh_state_from_response(&previous, &response, Some(60));

        assert_eq!(
            state,
            RssRefreshState {
                last_fetched_at: Some(1_781_947_200),
                last_etag: Some("\"reader-core-etag-v2\"".into()),
                last_modified: Some("Sat, 20 Jun 2026 08:00:00 GMT".into()),
                next_eligible_fetch_at: Some(1_781_950_800),
                not_modified: true,
            }
        );
        assert_eq!(
            serde_json::to_value(&response).unwrap(),
            serde_json::json!({
                "statusCode": 304,
                "responseAt": 1_781_947_200,
                "headers": {
                    "LAST-MODIFIED": " Sat, 20 Jun 2026 08:00:00 GMT ",
                    "etag": "  \"reader-core-etag-v2\"  "
                }
            })
        );
    }

    #[test]
    fn refresh_state_from_response_preserves_cache_headers_when_304_omits_them() {
        let previous = RssRefreshState {
            last_fetched_at: Some(1_781_940_600),
            last_etag: Some("\"reader-core-subscription-etag\"".into()),
            last_modified: Some("Sat, 20 Jun 2026 07:30:00 GMT".into()),
            next_eligible_fetch_at: None,
            not_modified: false,
        };
        let response = RssRefreshResponseMetadata {
            status_code: 304,
            response_at: 1_781_947_800,
            headers: BTreeMap::from([
                (RSS_HEADER_ETAG.into(), "  ".into()),
                (RSS_HEADER_LAST_MODIFIED.into(), "\n".into()),
            ]),
        };

        let state = rss_refresh_state_from_response(&previous, &response, None);

        assert_eq!(
            state,
            RssRefreshState {
                last_fetched_at: Some(1_781_947_800),
                last_etag: Some("\"reader-core-subscription-etag\"".into()),
                last_modified: Some("Sat, 20 Jun 2026 07:30:00 GMT".into()),
                next_eligible_fetch_at: None,
                not_modified: true,
            }
        );
    }

    #[test]
    fn sanitized_rss_corpus_fixture_parses_and_drives_refresh_state() {
        let manifest =
            include_str!("../../../fixtures/sanitized-corpus/rss-feed/rf-001.manifest.json");
        let manifest_json: serde_json::Value = serde_json::from_str(manifest).unwrap();
        assert_eq!(manifest_json["id"], "rf-001");
        assert_eq!(manifest_json["privacy_check"]["passed"], true);

        let xml = include_str!("../../../fixtures/sanitized-corpus/rss-feed/rf-001-fixture.xml");
        let feed = parse_feed_with_url("https://feed.example.test/rss.xml", xml).unwrap();
        assert_eq!(feed.title, "Sanitized Demo Feed");
        assert_eq!(feed.entries.len(), 3);
        assert_eq!(feed.entries[0].id, "urn:uuid:item-1");
        assert_eq!(
            feed.entries[2].published_at.as_deref(),
            Some("Fri, 03 Jan 2026 00:00:00 GMT")
        );

        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("rf-001", "https://feed.example.test/rss.xml", "").unwrap(),
            )
            .unwrap();
        let result = library
            .refresh_subscription("rf-001", &feed, 1_767_225_600)
            .unwrap();
        assert_eq!(result.new_entries.len(), 3);
        assert_eq!(result.subscription.unread_count, 3);
        assert_eq!(
            result.subscription.last_entry_id.as_deref(),
            Some("urn:uuid:item-1")
        );
    }

    #[test]
    fn parses_json_feed_default_mapping_without_fetching_next_page() {
        let json = r#"
            {
              "version": "https://jsonfeed.org/version/1.1",
              "title": "JSON Source",
              "home_page_url": "https://example.com",
              "feed_url": "https://example.com/feed.json",
              "next_url": "https://example.com/feed.json?page=2",
              "description": "JSON feed description",
              "author": {"name": "Feed Author"},
              "items": [
                {
                  "id": "json-one",
                  "url": "https://example.com/articles/json-one",
                  "external_url": "https://mirror.example.com/json-one",
                  "title": "JSON One",
                  "summary": "JSON summary",
                  "content_text": "JSON content body",
                  "date_published": "2026-06-20T07:30:00Z",
                  "author": {"name": "Item Author"},
                  "tags": ["updates", "core"]
                }
              ]
            }
        "#;

        let page =
            parse_json_feed_page(json, Some("json-source"), Some("JSON Source Override")).unwrap();

        assert_eq!(page.feed.title, "JSON Source Override");
        assert_eq!(
            page.feed.feed_url.as_deref(),
            Some("https://example.com/feed.json")
        );
        assert_eq!(page.feed.site_url.as_deref(), Some("https://example.com"));
        assert_eq!(
            page.feed.description.as_deref(),
            Some("JSON feed description")
        );
        assert_eq!(page.feed.entries.len(), 1);
        assert_eq!(page.feed.entries[0].id, "json-one");
        assert_eq!(page.feed.entries[0].title, "JSON One");
        assert_eq!(
            page.feed.entries[0].link.as_deref(),
            Some("https://example.com/articles/json-one")
        );
        assert_eq!(
            page.feed.entries[0].summary.as_deref(),
            Some("JSON summary")
        );
        assert_eq!(
            page.feed.entries[0].published_at.as_deref(),
            Some("2026-06-20T07:30:00Z")
        );

        assert_eq!(
            page.next_page_url.as_deref(),
            Some("https://example.com/feed.json?page=2")
        );
        assert_eq!(
            page.diagnostics,
            vec![
                "json_feed_mapping:core",
                "pagination_next_url_detected:json",
                "pagination_metadata_parse_only:no_network_fetch"
            ]
        );

        let item = &page.items[0];
        assert_eq!(item.title, "JSON One");
        assert_eq!(item.link, "https://example.com/articles/json-one");
        assert_eq!(item.author.as_deref(), Some("Item Author"));
        assert_eq!(item.summary.as_deref(), Some("JSON summary"));
        assert_eq!(item.published_at.as_deref(), Some("2026-06-20T07:30:00Z"));
        assert_eq!(item.source_id, "json-source");
        assert_eq!(item.source_name.as_deref(), Some("JSON Source Override"));
        assert_eq!(
            item.unknown_fields.get("jsonFeedId"),
            Some(&serde_json::json!("json-one"))
        );
        assert_eq!(
            item.unknown_fields.get("contentText"),
            Some(&serde_json::json!("JSON content body"))
        );
        assert_eq!(
            item.unknown_fields.get("categories"),
            Some(&serde_json::json!(["updates", "core"]))
        );
        assert_eq!(
            item.unknown_fields.get("feedVersion"),
            Some(&serde_json::json!("https://jsonfeed.org/version/1.1"))
        );
    }

    #[test]
    fn json_feed_parse_feed_path_and_html_summary_fallback_match_core_boundary() {
        let json = r#"
            {
              "version": "https://jsonfeed.org/version/1",
              "title": "HTML JSON Feed",
              "feed_url": "https://example.com/feed.json",
              "next_url": "https://example.com/feed.json",
              "items": [
                {
                  "id": "html-one",
                  "external_url": "https://example.com/html-one",
                  "content_html": "<p>Hello &amp; <strong>Reader</strong></p>",
                  "date_modified": "2026-06-21T00:00:00Z",
                  "authors": [{"name": "A"}, {"name": "B"}]
                }
              ]
            }
        "#;

        let feed = parse_feed(json).unwrap();
        assert_eq!(feed.title, "HTML JSON Feed");
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].id, "html-one");
        assert_eq!(feed.entries[0].title, "Hello & Reader");
        assert_eq!(
            feed.entries[0].link.as_deref(),
            Some("https://example.com/html-one")
        );
        assert_eq!(feed.entries[0].summary.as_deref(), Some("Hello & Reader"));
        assert_eq!(
            feed.entries[0].published_at.as_deref(),
            Some("2026-06-21T00:00:00Z")
        );

        let page = parse_json_feed_page(json, Some("https://example.com/feed.json"), None).unwrap();
        assert_eq!(page.next_page_url, None);
        assert_eq!(
            page.diagnostics,
            vec![
                "json_feed_mapping:core",
                "pagination_next_url_rejected:self_reference"
            ]
        );
        assert_eq!(page.items[0].author.as_deref(), Some("A, B"));
        assert_eq!(
            page.items[0].unknown_fields.get("contentHTML"),
            Some(&serde_json::json!(
                "<p>Hello &amp; <strong>Reader</strong></p>"
            ))
        );
        assert_eq!(
            page.items[0].unknown_fields.get("authors"),
            Some(&serde_json::json!(["A", "B"]))
        );
    }

    #[test]
    fn rss_and_atom_xml_pagination_reject_self_reference_without_fetching() {
        let rss = r#"
            <rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
              <channel>
                <title>RSS Pagination</title>
                <link>https://example.com/rss</link>
                <atom:link rel="self" href="https://example.com/feed.xml" />
                <atom:link rel="next" href="https://example.com/feed.xml" />
                <item>
                  <title>Entry</title>
                  <guid>entry-1</guid>
                </item>
              </channel>
            </rss>
        "#;
        let atom = r#"
            <feed xmlns="http://www.w3.org/2005/Atom">
              <title>Atom Pagination</title>
              <link rel="self" href="https://example.com/atom.xml" />
              <link rel="next" href="https://example.com/atom.xml" />
              <entry>
                <id>atom-1</id>
                <title>Entry</title>
              </entry>
            </feed>
        "#;

        let rss_plan = plan_xml_feed_pagination(rss, Some("https://example.com/feed.xml")).unwrap();
        let atom_plan =
            plan_xml_feed_pagination(atom, Some("https://example.com/atom.xml")).unwrap();

        assert_eq!(rss_plan.next_page_url, None);
        assert_eq!(atom_plan.next_page_url, None);
        assert_eq!(
            rss_plan.diagnostics,
            vec!["pagination_next_url_rejected:self_reference"]
        );
        assert_eq!(
            atom_plan.diagnostics,
            vec!["pagination_next_url_rejected:self_reference"]
        );
        assert!(!rss_plan.network_fetch_executed);
        assert!(!atom_plan.network_fetch_executed);
        assert!(!rss_plan
            .diagnostics
            .contains(&"pagination_metadata_parse_only:no_network_fetch".to_string()));
    }

    #[test]
    fn rss_xml_pagination_detects_non_self_next_page_and_rejects_drift() {
        let rss = r#"
            <rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
              <channel>
                <title>RSS Pagination</title>
                <atom:link rel="self" href="https://example.com/feed.xml" />
                <atom:link rel="next" href="https://example.com/feed.xml?page=2" />
                <item>
                  <title>Entry</title>
                  <guid>entry-1</guid>
                </item>
              </channel>
            </rss>
        "#;

        let plan = plan_xml_feed_pagination(rss, Some("https://example.com/feed.xml")).unwrap();

        assert_eq!(
            plan.next_page_url.as_deref(),
            Some("https://example.com/feed.xml?page=2")
        );
        assert_eq!(
            plan.diagnostics,
            vec![
                "pagination_next_url_detected:xml",
                "pagination_metadata_parse_only:no_network_fetch"
            ]
        );
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            serde_json::json!({
                "nextPageUrl": "https://example.com/feed.xml?page=2",
                "diagnostics": [
                    "pagination_next_url_detected:xml",
                    "pagination_metadata_parse_only:no_network_fetch"
                ],
                "networkFetchExecuted": false
            })
        );
        assert!(
            serde_json::from_value::<RssXmlPaginationPlan>(serde_json::json!({
                "nextPageUrl": null,
                "diagnostics": [],
                "networkFetchExecuted": false,
                "hostFetchStarted": true
            }))
            .is_err()
        );
        assert_eq!(
            plan_xml_feed_pagination("", Some("https://example.com/feed.xml")).unwrap_err(),
            RssError::EmptyInput
        );
    }

    #[test]
    fn json_feed_preserves_attachment_metadata_without_downloads() {
        let json = r#"
            {
              "version": "https://jsonfeed.org/version/1.1",
              "title": "Attachment Feed",
              "feed_url": "https://example.com/feed.json",
              "items": [
                {
                  "id": "with-attachments",
                  "title": "Audio Item",
                  "url": "https://example.com/audio-item",
                  "attachments": [
                    {
                      "url": "https://cdn.example.com/audio.mp3",
                      "mime_type": "audio/mpeg",
                      "title": "Audio",
                      "size_in_bytes": 12345,
                      "duration_in_seconds": 67.5,
                      "metadata": {
                        "chapters": ["intro", "body"],
                        "explicit": false
                      }
                    },
                    {
                      "url": "https://cdn.example.com/transcript.json",
                      "mime_type": "application/json",
                      "empty": "   "
                    }
                  ]
                }
              ]
            }
        "#;

        let page = parse_json_feed_page(json, Some("attachment-source"), None).unwrap();
        let attachments = page.items[0]
            .unknown_fields
            .get("attachments")
            .and_then(serde_json::Value::as_array)
            .unwrap();

        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0]["url"], "https://cdn.example.com/audio.mp3");
        assert_eq!(attachments[0]["mime_type"], "audio/mpeg");
        assert_eq!(attachments[0]["title"], "Audio");
        assert_eq!(attachments[0]["size_in_bytes"], 12345);
        assert_eq!(attachments[0]["duration_in_seconds"], 67.5);
        assert_eq!(
            attachments[0]["metadata"]["chapters"],
            serde_json::json!(["intro", "body"])
        );
        assert_eq!(attachments[0]["metadata"]["explicit"], false);
        assert_eq!(
            attachments[1]["url"],
            "https://cdn.example.com/transcript.json"
        );
        assert!(attachments[1].get("empty").is_none());
        assert!(!page
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("download")));
    }

    #[test]
    fn json_feed_preserves_underscore_extensions_without_claiming_regular_unknowns() {
        let json = r#"
            {
              "version": "https://jsonfeed.org/version/1.1",
              "title": "Extension Feed",
              "feed_url": "https://example.com/feed.json",
              "_reader": {
                "source": "fixture",
                "priority": 2,
                "flags": ["one", "two"]
              },
              "regular_unknown": "not claimed",
              "items": [
                {
                  "id": "extension-item",
                  "title": "Extension Item",
                  "url": "https://example.com/extension-item",
                  "_microblog": {
                    "conversation": "thread-1",
                    "sensitive": false
                  },
                  "_score": 9.5,
                  "regular_item_unknown": "not claimed"
                }
              ]
            }
        "#;

        let page = parse_json_feed_page(json, Some("extension-source"), None).unwrap();
        let item = &page.items[0];
        let feed_extensions = item
            .unknown_fields
            .get("feedExtensions")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        let item_extensions = item
            .unknown_fields
            .get("extensions")
            .and_then(serde_json::Value::as_object)
            .unwrap();

        assert_eq!(feed_extensions["_reader"]["source"], "fixture");
        assert_eq!(feed_extensions["_reader"]["priority"], 2);
        assert_eq!(
            feed_extensions["_reader"]["flags"],
            serde_json::json!(["one", "two"])
        );
        assert_eq!(item_extensions["_microblog"]["conversation"], "thread-1");
        assert_eq!(item_extensions["_microblog"]["sensitive"], false);
        assert_eq!(item_extensions["_score"], 9.5);
        assert!(!item.unknown_fields.contains_key("regular_unknown"));
        assert!(!item.unknown_fields.contains_key("regular_item_unknown"));
    }

    #[test]
    fn json_feed_preserves_author_metadata_without_avatar_fetches() {
        let json = r#"
            {
              "version": "https://jsonfeed.org/version/1.1",
              "title": "Author Feed",
              "feed_url": "https://example.com/feed.json",
              "author": {
                "name": "Feed Author",
                "url": "https://example.com/authors/feed",
                "avatar": "https://cdn.example.com/feed-avatar.png"
              },
              "authors": [
                {
                  "name": "Feed A",
                  "url": "https://example.com/authors/feed-a"
                }
              ],
              "items": [
                {
                  "id": "author-item",
                  "title": "Author Item",
                  "url": "https://example.com/author-item",
                  "author": {
                    "name": "Item Author",
                    "url": "https://example.com/authors/item",
                    "avatar": "https://cdn.example.com/item-avatar.png"
                  },
                  "authors": [
                    {
                      "name": "Array Author",
                      "url": "https://example.com/authors/array"
                    }
                  ]
                }
              ]
            }
        "#;

        let page = parse_json_feed_page(json, Some("author-source"), None).unwrap();
        let item = &page.items[0];

        assert_eq!(item.author.as_deref(), Some("Item Author"));
        assert_eq!(
            item.unknown_fields["feedAuthorMetadata"]["url"],
            "https://example.com/authors/feed"
        );
        assert_eq!(
            item.unknown_fields["feedAuthorMetadata"]["avatar"],
            "https://cdn.example.com/feed-avatar.png"
        );
        assert_eq!(
            item.unknown_fields["feedAuthorsMetadata"][0]["url"],
            "https://example.com/authors/feed-a"
        );
        assert_eq!(
            item.unknown_fields["authorMetadata"]["url"],
            "https://example.com/authors/item"
        );
        assert_eq!(
            item.unknown_fields["authorMetadata"]["avatar"],
            "https://cdn.example.com/item-avatar.png"
        );
        assert_eq!(
            item.unknown_fields["authorsMetadata"][0]["url"],
            "https://example.com/authors/array"
        );
        assert_eq!(
            item.unknown_fields["authors"],
            serde_json::json!(["Array Author"])
        );
        assert!(!page
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("avatar")));
    }

    #[test]
    fn json_feed_preserves_status_and_hub_metadata_without_fetches() {
        let json = r#"
            {
              "version": "https://jsonfeed.org/version/1.1",
              "title": "Status Feed",
              "feed_url": "https://example.com/feed.json",
              "description": "Feed level description",
              "user_comment": "Reader-visible source note",
              "expired": true,
              "hubs": [
                {
                  "type": "rssCloud",
                  "url": "https://hub.example.com/rsscloud",
                  "metadata": {
                    "region": "global"
                  }
                },
                {
                  "type": "WebSub",
                  "url": "https://hub.example.com/websub"
                }
              ],
              "items": [
                {
                  "id": "status-item",
                  "title": "Status Item",
                  "url": "https://example.com/status-item"
                }
              ]
            }
        "#;

        let page = parse_json_feed_page(json, Some("status-source"), None).unwrap();
        let item = &page.items[0];

        assert_eq!(
            page.feed.description.as_deref(),
            Some("Feed level description")
        );
        assert_eq!(
            item.unknown_fields["feedDescription"],
            "Feed level description"
        );
        assert_eq!(
            item.unknown_fields["feedUserComment"],
            "Reader-visible source note"
        );
        assert_eq!(item.unknown_fields["feedExpired"], true);
        assert_eq!(item.unknown_fields["feedHubs"][0]["type"], "rssCloud");
        assert_eq!(
            item.unknown_fields["feedHubs"][0]["url"],
            "https://hub.example.com/rsscloud"
        );
        assert_eq!(
            item.unknown_fields["feedHubs"][0]["metadata"]["region"],
            "global"
        );
        assert_eq!(item.unknown_fields["feedHubs"][1]["type"], "WebSub");
        assert_eq!(
            item.unknown_fields["feedHubs"][1]["url"],
            "https://hub.example.com/websub"
        );
        assert!(!page
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("hub")));
    }

    #[test]
    fn json_feed_rejects_invalid_json_and_non_object_items() {
        assert!(matches!(
            parse_feed("{not json").unwrap_err(),
            RssError::InvalidJsonFeed { .. }
        ));

        assert_eq!(
            parse_json_feed_page(r#"{"title":"Bad","items":[1]}"#, None, None).unwrap_err(),
            RssError::InvalidJsonFeed {
                detail: "items must contain objects".into(),
            }
        );
    }

    #[test]
    fn parses_rss_channel_and_items() {
        let xml = r#"
            <rss version="2.0">
              <channel>
                <title>Reader &amp; Core</title>
                <link>https://example.test</link>
                <description><![CDATA[Daily updates]]></description>
                <item>
                  <title>Entry 1</title>
                  <link>https://example.test/1</link>
                  <guid isPermaLink="false">entry-1</guid>
                  <pubDate>Wed, 24 Jun 2026 10:00:00 GMT</pubDate>
                  <description><![CDATA[Summary &amp; details]]></description>
                </item>
                <item>
                  <title>Entry 2</title>
                  <link>https://example.test/2</link>
                </item>
              </channel>
            </rss>
        "#;

        let feed = parse_feed_with_url("https://example.test/feed.xml", xml).unwrap();
        assert_eq!(feed.title, "Reader & Core");
        assert_eq!(
            feed.feed_url.as_deref(),
            Some("https://example.test/feed.xml")
        );
        assert_eq!(feed.site_url.as_deref(), Some("https://example.test"));
        assert_eq!(feed.description.as_deref(), Some("Daily updates"));
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].id, "entry-1");
        assert_eq!(
            feed.entries[0].summary.as_deref(),
            Some("Summary & details")
        );
        assert_eq!(feed.entries[1].id, "https://example.test/2");
    }

    #[test]
    fn parses_atom_feed_and_link_attributes() {
        let xml = r#"
            <feed xmlns="http://www.w3.org/2005/Atom">
              <title>Atom Feed</title>
              <subtitle>Updates</subtitle>
              <link rel="self" href="https://example.test/atom.xml" />
              <link rel="alternate" href="https://example.test/" />
              <entry>
                <id>tag:example.test,2026:1</id>
                <title>Atom Entry</title>
                <link rel="alternate" href="https://example.test/a" />
                <updated>2026-06-24T10:00:00Z</updated>
                <summary>Atom summary</summary>
              </entry>
            </feed>
        "#;

        let feed = parse_feed(xml).unwrap();
        assert_eq!(feed.title, "Atom Feed");
        assert_eq!(
            feed.feed_url.as_deref(),
            Some("https://example.test/atom.xml")
        );
        assert_eq!(feed.site_url.as_deref(), Some("https://example.test/"));
        assert_eq!(feed.description.as_deref(), Some("Updates"));
        assert_eq!(feed.entries[0].id, "tag:example.test,2026:1");
        assert_eq!(
            feed.entries[0].link.as_deref(),
            Some("https://example.test/a")
        );
        assert_eq!(
            feed.entries[0].published_at.as_deref(),
            Some("2026-06-24T10:00:00Z")
        );
    }

    #[test]
    fn rss_and_atom_summary_html_text_normalization_preserves_content_field() {
        let rss = r#"
            <rss version="2.0">
              <channel>
                <title>Summary Feed</title>
                <item>
                  <title>RSS HTML</title>
                  <guid>rss-html</guid>
                  <description><![CDATA[
                    <p>Hello <strong>World</strong></p>
                    <script>hidden()</script>
                    <style>.hidden { display: none; }</style>
                    <br/>Next &#x4E2D;&#20013;
                  ]]></description>
                  <content:encoded><![CDATA[<article>Full &amp; body</article>]]></content:encoded>
                </item>
              </channel>
            </rss>
        "#;
        let rss_feed = parse_feed(rss).unwrap();
        let rss_entry = &rss_feed.entries[0];
        assert_eq!(rss_entry.summary.as_deref(), Some("Hello World Next 中中"));
        assert!(!rss_entry.summary.as_deref().unwrap().contains("hidden"));
        assert_eq!(
            rss_entry.unknown_fields["content"],
            "<article>Full &amp; body</article>"
        );

        let rss_item = RssSubscriptionItem::from_entry(rss_entry, "rss-source", Some("RSS".into()));
        assert_eq!(
            rss_item.unknown_fields["content"],
            "<article>Full &amp; body</article>"
        );
        assert_eq!(rss_item.unknown_fields["entryId"], "rss-html");

        let atom = r#"
            <feed xmlns="http://www.w3.org/2005/Atom">
              <title>Atom Feed</title>
              <entry>
                <id>atom-escaped</id>
                <title>Atom Escaped</title>
                <content>&lt;p&gt;Atom escaped text&lt;/p&gt;&lt;script&gt;hidden()&lt;/script&gt;</content>
              </entry>
            </feed>
        "#;
        let atom_feed = parse_feed(atom).unwrap();
        let atom_entry = &atom_feed.entries[0];
        assert_eq!(atom_entry.summary.as_deref(), Some("Atom escaped text"));
        assert!(!atom_entry.summary.as_deref().unwrap().contains("script"));
        assert_eq!(
            atom_entry.unknown_fields["content"],
            "<p>Atom escaped text</p><script>hidden()</script>"
        );
    }

    #[test]
    fn rss_summary_common_entity_variants_normalize_to_text() {
        let rss = r#"
            <rss version="2.0">
              <channel>
                <title>Entities</title>
                <item>
                  <title>RSS Entities</title>
                  <guid>rss-entities</guid>
                  <description><![CDATA[Caf&eacute; &COPY; &mdash; &hellip; &frac12;&nbsp;Price &deg C &euro;]]></description>
                </item>
              </channel>
            </rss>
        "#;
        let rss_feed = parse_feed(rss).unwrap();
        assert_eq!(
            rss_feed.entries[0].summary.as_deref(),
            Some("Café © — … ½ Price ° C €")
        );

        let atom = r#"
            <feed xmlns="http://www.w3.org/2005/Atom">
              <title>Atom Entities</title>
              <entry>
                <id>atom-entities</id>
                <title>Atom Entities</title>
                <content>Temp 21&deg C &AMP; Cr&egrave;me &Ntilde; &uuml;</content>
              </entry>
            </feed>
        "#;
        let atom_feed = parse_feed(atom).unwrap();
        assert_eq!(
            atom_feed.entries[0].summary.as_deref(),
            Some("Temp 21° C & Crème Ñ ü")
        );
    }

    #[test]
    fn xml_item_metadata_preserves_categories_media_and_enclosure() {
        let rss = r#"
            <rss version="2.0">
              <channel>
                <title>RSS Metadata</title>
                <item>
                  <title>Media Entry</title>
                  <guid>media-entry</guid>
                  <link>https://example.test/media</link>
                  <category>fiction</category>
                  <category><![CDATA[updates]]></category>
                  <media:thumbnail url="https://example.test/thumb.jpg" />
                  <media:content url="https://example.test/video.mp4" type="video/mp4" />
                  <enclosure url="https://example.test/audio.mp3" type="audio/mpeg" length="1024" />
                </item>
              </channel>
            </rss>
        "#;
        let rss_feed = parse_feed(rss).unwrap();
        let fields = &rss_feed.entries[0].unknown_fields;
        assert_eq!(fields["guid"], "media-entry");
        assert_eq!(
            fields["categories"],
            serde_json::json!(["fiction", "updates"])
        );
        assert_eq!(fields["image"], "https://example.test/thumb.jpg");
        assert_eq!(fields["mediaType"], "video/mp4");
        assert_eq!(fields["enclosure"]["url"], "https://example.test/audio.mp3");
        assert_eq!(fields["enclosure"]["type"], "audio/mpeg");
        assert_eq!(fields["enclosure"]["length"], 1024);

        let atom = r#"
            <feed xmlns="http://www.w3.org/2005/Atom">
              <title>Atom Categories</title>
              <entry>
                <id>atom-categories</id>
                <title>Entry with Categories</title>
                <link rel="alternate" href="https://example.com/articles/category" />
                <category term="fiction" />
                <category term="updates"></category>
              </entry>
            </feed>
        "#;
        let atom_feed = parse_feed(atom).unwrap();
        assert_eq!(
            atom_feed.entries[0].unknown_fields["categories"],
            serde_json::json!(["fiction", "updates"])
        );
    }

    #[test]
    fn feed_parser_rejects_empty_and_unknown_input() {
        assert_eq!(parse_feed("").unwrap_err(), RssError::EmptyInput);
        assert_eq!(
            parse_feed("<html><body>not a feed</body></html>").unwrap_err(),
            RssError::UnsupportedFormat
        );
    }

    #[test]
    fn feed_parser_requires_feed_title() {
        let err =
            parse_feed("<rss><channel><item><title>A</title></item></channel></rss>").unwrap_err();
        assert_eq!(
            err,
            RssError::MissingField {
                field: "feed.title".into()
            }
        );
    }

    #[test]
    fn feed_parser_requires_entry_identity_when_item_has_no_stable_fields() {
        let err = parse_feed(
            "<rss><channel><title>Feed</title><item><description>x</description></item></channel></rss>",
        )
        .unwrap_err();
        assert_eq!(
            err,
            RssError::MissingField {
                field: "entry.id".into()
            }
        );
    }

    #[test]
    fn feed_parser_deduplicates_entries_by_id() {
        let xml = r#"
            <rss><channel><title>Feed</title>
              <item><title>A</title><guid>same</guid></item>
              <item><title>B</title><guid>same</guid></item>
              <item><title>C</title><guid>other</guid></item>
            </channel></rss>
        "#;

        let feed = parse_feed(xml).unwrap();
        let titles: Vec<&str> = feed
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "C"]);
    }

    #[test]
    fn subscription_new_rejects_empty_required_fields() {
        assert!(matches!(
            RssSubscription::new("", "https://example.test/feed.xml", "Feed"),
            Err(RssError::InvalidSubscription { .. })
        ));
        assert!(matches!(
            RssSubscription::new("sub", "   ", "Feed"),
            Err(RssError::InvalidSubscription { .. })
        ));
    }

    #[test]
    fn subscription_apply_first_feed_marks_all_entries_unread() {
        let feed = RssFeed {
            title: "Feed".into(),
            feed_url: Some("https://example.test/feed.xml".into()),
            site_url: Some("https://example.test".into()),
            description: None,
            entries: vec![
                RssEntry {
                    id: "3".into(),
                    title: "Three".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                    unknown_fields: BTreeMap::new(),
                },
                RssEntry {
                    id: "2".into(),
                    title: "Two".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                    unknown_fields: BTreeMap::new(),
                },
            ],
        };
        let mut subscription =
            RssSubscription::new("sub", "https://old.test/feed.xml", "").unwrap();

        let result = subscription.apply_feed(&feed, 1700000000).unwrap();

        assert_eq!(result.new_entries.len(), 2);
        assert_eq!(subscription.feed_url, "https://example.test/feed.xml");
        assert_eq!(
            subscription.site_url.as_deref(),
            Some("https://example.test")
        );
        assert_eq!(subscription.last_entry_id.as_deref(), Some("3"));
        assert_eq!(subscription.last_fetch_at, Some(1700000000));
        assert_eq!(subscription.unread_count, 2);
    }

    #[test]
    fn subscription_apply_next_feed_counts_only_new_prefix() {
        let mut subscription =
            RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap();
        subscription.last_entry_id = Some("2".into());
        subscription.unread_count = 4;
        let feed = RssFeed {
            title: "Feed".into(),
            feed_url: None,
            site_url: None,
            description: None,
            entries: vec![
                RssEntry {
                    id: "4".into(),
                    title: "Four".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                    unknown_fields: BTreeMap::new(),
                },
                RssEntry {
                    id: "3".into(),
                    title: "Three".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                    unknown_fields: BTreeMap::new(),
                },
                RssEntry {
                    id: "2".into(),
                    title: "Two".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                    unknown_fields: BTreeMap::new(),
                },
            ],
        };

        let result = subscription.apply_feed(&feed, 1700001000).unwrap();

        let ids: Vec<&str> = result
            .new_entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(ids, vec!["4", "3"]);
        assert_eq!(subscription.last_entry_id.as_deref(), Some("4"));
        assert_eq!(subscription.unread_count, 6);
    }

    #[test]
    fn subscription_apply_feed_treats_missing_previous_id_as_all_new() {
        let mut subscription =
            RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap();
        subscription.last_entry_id = Some("old".into());
        let feed = RssFeed {
            title: "Feed".into(),
            feed_url: None,
            site_url: None,
            description: None,
            entries: vec![RssEntry {
                id: "new".into(),
                title: "New".into(),
                link: None,
                summary: None,
                published_at: None,
                unknown_fields: BTreeMap::new(),
            }],
        };

        let result = subscription.apply_feed(&feed, 1700002000).unwrap();

        assert_eq!(result.new_entries.len(), 1);
        assert_eq!(subscription.last_entry_id.as_deref(), Some("new"));
        assert_eq!(subscription.unread_count, 1);
    }

    #[test]
    fn subscription_mark_all_read_resets_unread_count() {
        let mut subscription =
            RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap();
        subscription.unread_count = 12;

        subscription.mark_all_read();

        assert_eq!(subscription.unread_count, 0);
    }

    fn entry(id: &str, title: &str) -> RssEntry {
        RssEntry {
            id: id.into(),
            title: title.into(),
            link: Some(format!("https://example.test/{id}")),
            summary: None,
            published_at: None,
            unknown_fields: BTreeMap::new(),
        }
    }

    fn feed(ids: &[&str]) -> RssFeed {
        RssFeed {
            title: "Feed".into(),
            feed_url: Some("https://example.test/feed.xml".into()),
            site_url: Some("https://example.test".into()),
            description: None,
            entries: ids
                .iter()
                .map(|id| entry(id, &format!("Entry {id}")))
                .collect(),
        }
    }

    fn populate_snapshot_library(library: &mut RssLibrary) {
        library
            .upsert_subscription(
                RssSubscription::new("b", "https://example.test/b.xml", "Beta").unwrap(),
            )
            .unwrap();
        library
            .upsert_subscription(
                RssSubscription::new("a", "https://example.test/a.xml", "Alpha").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("b", &feed(&["2", "1"]), 1000)
            .unwrap();
        library
            .refresh_subscription("a", &feed(&["9"]), 2000)
            .unwrap();
        library.mark_entry_read("b", "1", 1100).unwrap();
        library.set_entry_starred("b", "1", true).unwrap();
    }

    #[test]
    fn rss_snapshot_export_is_stable_and_json_round_trips() {
        let mut library = RssLibrary::new();
        populate_snapshot_library(&mut library);

        let snapshot = library.export_snapshot(42).unwrap();

        assert_eq!(snapshot.schema_version, RSS_LIBRARY_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.exported_at, 42);
        assert_eq!(
            snapshot
                .subscriptions
                .iter()
                .map(|subscription| subscription.subscription_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            snapshot
                .entries
                .iter()
                .map(|state| {
                    (
                        state.subscription_id.as_str(),
                        state.entry.id.as_str(),
                        state.read,
                        state.starred,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("a", "9", false, false),
                ("b", "1", true, true),
                ("b", "2", false, false)
            ]
        );

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains(r#""schemaVersion":1"#));
        let back: RssLibrarySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn rss_snapshot_replace_round_trips_state_and_recomputes_unread_count() {
        let mut source = RssLibrary::new();
        populate_snapshot_library(&mut source);
        let mut snapshot = source.export_snapshot(77).unwrap();
        for subscription in &mut snapshot.subscriptions {
            subscription.unread_count = 999;
        }

        let mut restored = RssLibrary::new();
        restored.replace_with_snapshot(snapshot).unwrap();

        assert_eq!(
            restored
                .get_subscription("a")
                .unwrap()
                .unwrap()
                .unread_count,
            1
        );
        assert_eq!(
            restored
                .get_subscription("b")
                .unwrap()
                .unwrap()
                .unread_count,
            1
        );
        let b_entries = restored.list_entries("b").unwrap();
        let one = b_entries
            .iter()
            .find(|state| state.entry.id == "1")
            .unwrap();
        assert!(one.read);
        assert_eq!(one.read_at, Some(1100));
        assert!(one.starred);
    }

    #[test]
    fn rss_snapshot_empty_replace_clears_existing_library() {
        let mut library = RssLibrary::new();
        populate_snapshot_library(&mut library);

        library
            .replace_with_snapshot(RssLibrarySnapshot::empty(100))
            .unwrap();

        assert!(library.list_subscriptions().is_empty());
        assert!(library.get_subscription("a").unwrap().is_none());
    }

    #[test]
    fn rss_snapshot_rejects_schema_duplicates_orphans_and_unknown_fields() {
        let mut wrong_schema = RssLibrarySnapshot::empty(1);
        wrong_schema.schema_version = 2;
        assert_eq!(
            wrong_schema.validate().unwrap_err(),
            RssError::InvalidSnapshot {
                field: "schema_version".into()
            }
        );

        let mut duplicate_subscription = RssLibrarySnapshot::empty(1);
        duplicate_subscription
            .subscriptions
            .push(RssSubscription::new("sub", "https://example.test/a.xml", "A").unwrap());
        duplicate_subscription
            .subscriptions
            .push(RssSubscription::new("sub", "https://example.test/b.xml", "B").unwrap());
        assert_eq!(
            duplicate_subscription.validate().unwrap_err(),
            RssError::InvalidSnapshot {
                field: "subscriptions".into()
            }
        );

        let mut orphan_entry = RssLibrarySnapshot::empty(1);
        orphan_entry.entries.push(RssEntryState {
            subscription_id: "missing".into(),
            entry: entry("1", "One"),
            first_seen_at: 1000,
            read: false,
            read_at: None,
            starred: false,
        });
        assert_eq!(
            orphan_entry.validate().unwrap_err(),
            RssError::InvalidSnapshot {
                field: "entries.subscription_id".into()
            }
        );

        let unknown =
            r#"{"schemaVersion":1,"exportedAt":1,"subscriptions":[],"entries":[],"bogus":true}"#;
        assert!(serde_json::from_str::<RssLibrarySnapshot>(unknown).is_err());
    }

    #[test]
    fn rss_snapshot_replace_is_atomic_on_validation_failure() {
        let mut library = RssLibrary::new();
        populate_snapshot_library(&mut library);
        let before = library.export_snapshot(1).unwrap();

        let mut invalid = RssLibrarySnapshot::empty(2);
        invalid
            .subscriptions
            .push(RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap());
        invalid.entries.push(RssEntryState {
            subscription_id: "sub".into(),
            entry: entry("", "Invalid"),
            first_seen_at: 1,
            read: false,
            read_at: None,
            starred: false,
        });

        assert!(matches!(
            library.replace_with_snapshot(invalid),
            Err(RssError::InvalidSubscription { .. })
        ));
        assert_eq!(library.export_snapshot(1).unwrap(), before);
    }

    #[test]
    fn rss_library_upserts_and_lists_subscriptions_deterministically() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("b", "https://example.test/b.xml", "Beta").unwrap(),
            )
            .unwrap();
        library
            .upsert_subscription(
                RssSubscription::new("a", "https://example.test/a.xml", "Alpha").unwrap(),
            )
            .unwrap();

        let ids: Vec<String> = library
            .list_subscriptions()
            .into_iter()
            .map(|subscription| subscription.subscription_id)
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(
            library.get_subscription("a").unwrap().unwrap().title,
            "Alpha"
        );
    }

    #[test]
    fn rss_library_refresh_inserts_entries_and_updates_unread_count() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://old.test/feed.xml", "Old").unwrap(),
            )
            .unwrap();

        let result = library
            .refresh_subscription("sub", &feed(&["3", "2", "1"]), 1000)
            .unwrap();

        assert_eq!(result.new_entries.len(), 3);
        assert_eq!(result.subscription.title, "Feed");
        assert_eq!(
            result.subscription.feed_url,
            "https://example.test/feed.xml"
        );
        assert_eq!(result.subscription.unread_count, 3);
        let states = library.list_entries("sub").unwrap();
        assert_eq!(states.len(), 3);
        assert!(states.iter().all(|state| !state.read));
        assert!(states.iter().all(|state| state.first_seen_at == 1000));
    }

    #[test]
    fn rss_library_refresh_preserves_read_and_starred_state() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("sub", &feed(&["2", "1"]), 1000)
            .unwrap();
        library.mark_entry_read("sub", "1", 1100).unwrap();
        library.set_entry_starred("sub", "1", true).unwrap();

        let result = library
            .refresh_subscription("sub", &feed(&["3", "2", "1"]), 2000)
            .unwrap();

        assert_eq!(
            result
                .new_entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["3"]
        );
        assert_eq!(result.subscription.unread_count, 2);
        let states = library.list_entries("sub").unwrap();
        let one = states.iter().find(|state| state.entry.id == "1").unwrap();
        assert!(one.read);
        assert_eq!(one.read_at, Some(1100));
        assert!(one.starred);
        assert_eq!(one.first_seen_at, 1000);
        let three = states.iter().find(|state| state.entry.id == "3").unwrap();
        assert!(!three.read);
        assert_eq!(three.first_seen_at, 2000);
    }

    #[test]
    fn rss_library_mark_unread_and_all_read_recompute_subscription_count() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("sub", &feed(&["2", "1"]), 1000)
            .unwrap();

        library.mark_entry_read("sub", "1", 1100).unwrap();
        assert_eq!(
            library
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .unread_count,
            1
        );
        library.mark_entry_unread("sub", "1").unwrap();
        assert_eq!(
            library
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .unread_count,
            2
        );
        library.mark_all_read("sub", 1200).unwrap();
        assert_eq!(
            library
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .unread_count,
            0
        );
        assert!(library
            .list_entries("sub")
            .unwrap()
            .iter()
            .all(|state| state.read_at == Some(1200)));
    }

    #[test]
    fn rss_library_remove_subscription_is_idempotent_and_clears_entries() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("sub", &feed(&["2", "1"]), 1000)
            .unwrap();

        assert_eq!(library.remove_subscription("sub").unwrap(), 2);
        assert!(library.get_subscription("sub").unwrap().is_none());
        assert_eq!(library.remove_subscription("sub").unwrap(), 0);
    }

    #[test]
    fn rss_library_reports_missing_subscription_and_entry() {
        let mut library = RssLibrary::new();
        assert_eq!(
            library
                .refresh_subscription("missing", &feed(&["1"]), 1000)
                .unwrap_err(),
            RssError::SubscriptionNotFound {
                subscription_id: "missing".into()
            }
        );
        assert_eq!(
            library.list_entries("missing").unwrap_err(),
            RssError::SubscriptionNotFound {
                subscription_id: "missing".into()
            }
        );

        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        assert_eq!(
            library.mark_entry_read("sub", "missing", 1).unwrap_err(),
            RssError::EntryNotFound {
                subscription_id: "sub".into(),
                entry_id: "missing".into()
            }
        );
        assert!(matches!(
            library.get_subscription(""),
            Err(RssError::InvalidSubscription { .. })
        ));
        assert!(matches!(
            library.mark_entry_read("sub", "", 1),
            Err(RssError::InvalidSubscription { .. })
        ));
    }
}

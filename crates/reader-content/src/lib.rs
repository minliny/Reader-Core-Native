//! Reader-Core content pipeline — search / detail / toc / chapter / normalization.
//!
//! V1 remote-reading vertical. This crate turns a [`Source`](reader_domain::Source)
//! definition plus raw HTML/JSON response text into structured books, tables of
//! contents, and chapter bodies, using the non-JS [`RuleEngine`](reader_rule::RuleEngine)
//! as the primary extraction engine.
//!
//! A minimal JS rule path is also supported: when a pipeline stage carries a
//! `jsRule` script, it is evaluated in a [`JsSandbox`](reader_js::JsSandbox).
//! Because V1 exposes no real network host API, a JS rule that calls `java.get`
//! / `java.post` without a registered host callback yields a structured
//! `unsupported` outcome instead of being silently treated as a network
//! capability.
//!
//! Content normalization helpers live in [`normalization`] and are applied to
//! chapter body text before it is returned to callers.

pub mod normalization;

use std::collections::HashMap;
use std::sync::Arc;

use reader_domain::{
    Book, BookInfoRule, ContentRule, ExploreRule, LegadoBookSource, ReadingProgress, SearchRule,
    Source, TocEntry, TocRule,
};
use reader_js::{JsError, JsErrorKind, JsEvaluation, JsSandbox as JsSandboxTrait, QuickJsSandbox};
use reader_rule::{CaptureGroup, LegadoRuleContext, RuleEngine, RuleError, RuleOutput, RuleStep};
use serde::{Deserialize, Serialize};

/// Current content library snapshot schema version.
pub const CONTENT_LIBRARY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// A JSON-serializable rule-step specification that mirrors the constructors on
/// [`RuleStep`]. `reader-rule` deliberately does not derive Serialize/Deserialize
/// (its public API is programmatic), so this adapter is the bridge between
/// source-definition JSON and the rule engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum RuleStepSpec {
    /// Extract a regex capture group. `group` is `"whole"`, an integer index,
    /// or a named group string.
    RegexExtract {
        pattern: String,
        #[serde(default)]
        group: CaptureGroupSpec,
        #[serde(default)]
        all: bool,
    },
    /// Replace matches of `pattern` with `replacement`.
    RegexReplace {
        pattern: String,
        replacement: String,
        #[serde(default)]
        all: bool,
    },
    /// Minimal JSONPath lookup (e.g. `$.books[*].title`).
    JsonPath {
        path: String,
    },
    /// CSS selector extraction.
    CssText {
        selector: String,
    },
    CssAttr {
        selector: String,
        attr: String,
    },
    /// XPath expression evaluation.
    XPath {
        expression: String,
    },
}

/// Capture-group selector for [`RuleStepSpec::RegexExtract`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CaptureGroupSpec {
    Whole { whole: bool },
    Index { index: u32 },
    Name { name: String },
}

impl Default for CaptureGroupSpec {
    fn default() -> Self {
        CaptureGroupSpec::Whole { whole: true }
    }
}

impl CaptureGroupSpec {
    fn into_capture_group(self) -> CaptureGroup {
        match self {
            CaptureGroupSpec::Whole { .. } => CaptureGroup::WholeMatch,
            CaptureGroupSpec::Index { index } => CaptureGroup::Index(index as usize),
            CaptureGroupSpec::Name { name } => CaptureGroup::Name(name),
        }
    }
}

impl RuleStepSpec {
    /// Convert this JSON spec into a concrete [`RuleStep`].
    pub fn into_rule_step(self) -> Result<RuleStep, ContentError> {
        Ok(match self {
            RuleStepSpec::RegexExtract {
                pattern,
                group,
                all,
            } => {
                let group = group.into_capture_group();
                if all {
                    RuleStep::regex_capture(pattern, group)
                } else {
                    RuleStep::regex_capture_first(pattern, group)
                }
            }
            RuleStepSpec::RegexReplace {
                pattern,
                replacement,
                all,
            } => {
                if all {
                    RuleStep::regex_replace(pattern, replacement)
                } else {
                    RuleStep::regex_replace_first(pattern, replacement)
                }
            }
            RuleStepSpec::JsonPath { path } => RuleStep::json_path(path),
            RuleStepSpec::CssText { selector } => RuleStep::css_text(selector),
            RuleStepSpec::CssAttr { selector, attr } => RuleStep::css_attr(selector, attr),
            RuleStepSpec::XPath { expression } => RuleStep::xpath(expression),
        })
    }
}

/// Errors raised by the content pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentError {
    /// A rule definition could not be parsed or converted.
    RuleSpec(String),
    /// The underlying rule engine failed (bad selector, regex, JSON parse, etc.).
    Rule(RuleError),
    /// A required field was missing from extracted data.
    MissingField { field: String },
    /// A JS rule referenced a host capability (e.g. `java.get`) that has no
    /// registered callback — i.e. real network is not available in V1.
    JsUnsupported { reason: String },
    /// A JS rule evaluated but produced a non-object / unusable value.
    JsResult(String),
    /// A JS rule raised an exception or timed out.
    Js(JsError),
    /// Content normalization or remapping received invalid chapter data.
    InvalidChapter { field: String },
    /// A persisted content document or library snapshot was invalid.
    InvalidDocument { field: String },
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentError::RuleSpec(m) => write!(f, "invalid rule spec: {m}"),
            ContentError::Rule(e) => write!(f, "rule engine error: {e}"),
            ContentError::MissingField { field } => write!(f, "missing field: {field}"),
            ContentError::JsUnsupported { reason } => {
                write!(f, "JS rule unsupported in V1: {reason}")
            }
            ContentError::JsResult(m) => write!(f, "JS rule produced unusable result: {m}"),
            ContentError::Js(e) => write!(f, "JS rule error: {e}"),
            ContentError::InvalidChapter { field } => {
                write!(f, "invalid chapter field: {field}")
            }
            ContentError::InvalidDocument { field } => {
                write!(f, "invalid content document field: {field}")
            }
        }
    }
}

impl std::error::Error for ContentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ContentError::Rule(e) => Some(e),
            ContentError::Js(e) => Some(e),
            _ => None,
        }
    }
}

impl From<RuleError> for ContentError {
    fn from(e: RuleError) -> Self {
        ContentError::Rule(e)
    }
}

/// Result of a JS rule evaluation, including the structured unsupported signal.
#[derive(Debug, Clone, PartialEq)]
pub enum JsOutcome {
    /// The JS rule succeeded and returned a JSON value.
    Ok(serde_json::Value),
    /// The JS rule is unsupported in V1 (e.g. it called `java.get` with no host
    /// callback registered).
    Unsupported { reason: String },
}

/// Normalized chapter body ready for cache/storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedChapter {
    pub source_id: String,
    pub book_id: String,
    pub chapter_index: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    /// Body text with normalized line endings and stable blank-line handling.
    pub content: String,
    /// Paragraph chunks split on blank lines after normalization.
    #[serde(default)]
    pub paragraphs: Vec<String>,
    /// Character count of `content`.
    pub char_len: usize,
    /// Stable FNV-1a fingerprint over normalized content.
    pub content_fingerprint: String,
    /// Stable cache key for `(source, book, chapter index, fingerprint)`.
    pub cache_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChapterIdentityKind {
    Url,
    Title,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TocRemapEntry {
    pub old_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_index: Option<u32>,
    pub identity_kind: ChapterIdentityKind,
    pub identity: String,
}

/// TOC refresh diff that maps old chapter indexes to new chapter indexes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TocRefreshDiff {
    pub old_len: usize,
    pub new_len: usize,
    #[serde(default)]
    pub mappings: Vec<TocRemapEntry>,
    #[serde(default)]
    pub inserted: Vec<TocEntry>,
    #[serde(default)]
    pub removed: Vec<TocEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgressRemapStatus {
    Unchanged,
    Remapped,
    ChapterRemovedClamped,
    EmptyToc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemappedReadingProgress {
    pub progress: ReadingProgress,
    pub status: ProgressRemapStatus,
}

/// Persistable content package for one source/book pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentDocument {
    pub source_id: String,
    pub book: Book,
    #[serde(default)]
    pub toc: Vec<TocEntry>,
    #[serde(default)]
    pub chapters: Vec<NormalizedChapter>,
    pub updated_at: i64,
    /// Stable fingerprint across TOC and chapter content.
    pub content_fingerprint: String,
}

impl ContentDocument {
    pub fn new(
        source_id: impl Into<String>,
        book: Book,
        toc: Vec<TocEntry>,
        chapters: Vec<NormalizedChapter>,
        updated_at: i64,
    ) -> Result<Self, ContentError> {
        let mut document = Self {
            source_id: source_id.into().trim().to_string(),
            book,
            toc,
            chapters,
            updated_at,
            content_fingerprint: String::new(),
        };
        document.content_fingerprint = content_document_fingerprint(&document);
        validate_content_document(&document)?;
        Ok(document)
    }
}

/// Complete export/import unit for content documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentLibrarySnapshot {
    pub schema_version: u32,
    pub exported_at: i64,
    #[serde(default)]
    pub documents: Vec<ContentDocument>,
}

impl ContentLibrarySnapshot {
    pub fn empty(exported_at: i64) -> Self {
        Self {
            schema_version: CONTENT_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            documents: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ContentError> {
        if self.schema_version != CONTENT_LIBRARY_SNAPSHOT_SCHEMA_VERSION {
            return Err(ContentError::InvalidDocument {
                field: "schema_version".into(),
            });
        }

        let mut keys = HashMap::<ContentDocumentKey, ()>::new();
        for document in &self.documents {
            validate_content_document(document)?;
            if keys.insert(document.document_key(), ()).is_some() {
                return Err(ContentError::InvalidDocument {
                    field: "documents".into(),
                });
            }
        }
        Ok(())
    }
}

/// In-memory content document library.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ContentDocumentLibrary {
    documents: HashMap<ContentDocumentKey, ContentDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ContentDocumentKey {
    source_id: String,
    book_id: String,
}

impl ContentDocument {
    fn document_key(&self) -> ContentDocumentKey {
        ContentDocumentKey {
            source_id: self.source_id.clone(),
            book_id: self.book.book_id.clone(),
        }
    }
}

impl ContentDocumentLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_document(
        &mut self,
        document: ContentDocument,
    ) -> Result<ContentDocument, ContentError> {
        validate_content_document(&document)?;
        self.documents
            .insert(document.document_key(), document.clone());
        Ok(document)
    }

    pub fn get_document(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<ContentDocument>, ContentError> {
        let key = content_document_key(source_id, book_id)?;
        Ok(self.documents.get(&key).cloned())
    }

    pub fn list_documents(&self) -> Vec<ContentDocument> {
        let mut documents = self.documents.values().cloned().collect::<Vec<_>>();
        documents.sort_by(compare_content_document_key);
        documents
    }

    pub fn get_chapter(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<NormalizedChapter>, ContentError> {
        let Some(document) = self.get_document(source_id, book_id)? else {
            return Ok(None);
        };
        Ok(document
            .chapters
            .iter()
            .find(|chapter| chapter.chapter_index == chapter_index)
            .cloned())
    }

    pub fn remove_document(
        &mut self,
        source_id: &str,
        book_id: &str,
    ) -> Result<bool, ContentError> {
        let key = content_document_key(source_id, book_id)?;
        Ok(self.documents.remove(&key).is_some())
    }

    pub fn export_snapshot(
        &self,
        exported_at: i64,
    ) -> Result<ContentLibrarySnapshot, ContentError> {
        let mut snapshot = ContentLibrarySnapshot {
            schema_version: CONTENT_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            documents: self.documents.values().cloned().collect(),
        };
        sort_content_snapshot(&mut snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn replace_with_snapshot(
        &mut self,
        snapshot: ContentLibrarySnapshot,
    ) -> Result<(), ContentError> {
        snapshot.validate()?;
        let mut documents = HashMap::new();
        for document in snapshot.documents {
            documents.insert(document.document_key(), document);
        }
        self.documents = documents;
        Ok(())
    }
}

/// The remote-reading content pipeline.
///
/// Holds a [`RuleEngine`] and an optional JS sandbox. The JS sandbox is shared
/// via `Arc` so the pipeline is cheap to clone per request.
#[derive(Clone)]
pub struct RemoteContentPipeline {
    engine: RuleEngine,
    js: Arc<QuickJsSandbox>,
}

impl Default for RemoteContentPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteContentPipeline {
    /// Create a pipeline with the default rule engine and a default
    /// (no host-callback) JS sandbox.
    pub fn new() -> Self {
        Self {
            engine: RuleEngine::new(),
            js: Arc::new(QuickJsSandbox::default()),
        }
    }

    /// Create a pipeline with a custom JS sandbox (e.g. one with host callbacks
    /// registered for tests).
    pub fn with_js_sandbox(js: QuickJsSandbox) -> Self {
        Self {
            engine: RuleEngine::new(),
            js: Arc::new(js),
        }
    }

    fn parse_steps(spec: &serde_json::Value) -> Result<Vec<RuleStep>, ContentError> {
        let steps: Vec<RuleStepSpec> = if spec.is_null() {
            Vec::new()
        } else {
            serde_json::from_value(spec.clone())
                .map_err(|e| ContentError::RuleSpec(e.to_string()))?
        };
        steps.into_iter().map(|s| s.into_rule_step()).collect()
    }

    /// Run a rule chain over `input` and return the flat output.
    pub fn run_chain(
        &self,
        input: &str,
        rule_spec: &serde_json::Value,
    ) -> Result<RuleOutput, ContentError> {
        if let Some(rule) = rule_spec.as_str() {
            return Ok(self.engine.execute_legado_css(input, rule)?);
        }

        let steps = Self::parse_steps(rule_spec)?;
        if steps.is_empty() {
            return Ok(RuleOutput::new(Vec::new()));
        }
        Ok(self.engine.execute_chain(input, &steps)?)
    }

    /// Run ordered raw Legado field rules against the same response while
    /// sharing Legado's `@put` / `@get` variable context.
    pub fn run_legado_field_rules_with_context<I, K, R>(
        &self,
        input: &str,
        rules: I,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<(String, RuleOutput)>, ContentError>
    where
        I: IntoIterator<Item = (K, R)>,
        K: Into<String>,
        R: AsRef<str>,
    {
        let mut outputs = Vec::new();
        for (field, rule) in rules {
            let output =
                self.engine
                    .execute_legado_css_with_context(input, rule.as_ref(), context)?;
            outputs.push((field.into(), output));
        }
        Ok(outputs)
    }

    /// Run a structured Legado `bookInfoRule` with Rust-owned field ordering.
    ///
    /// `init` is executed first so its `@put` bindings can seed later fields,
    /// but it is not returned as extracted book metadata.
    pub fn run_legado_book_info_rule_with_context(
        &self,
        input: &str,
        rule: &BookInfoRule,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<(String, RuleOutput)>, ContentError> {
        if let Some(init) = rule
            .r#init
            .as_deref()
            .filter(|rule| !rule.trim().is_empty())
        {
            self.engine
                .execute_legado_css_with_context(input, init, context)?;
        }

        self.run_legado_field_rules_with_context(input, legado_book_info_rule_fields(rule), context)
    }

    /// Run the Rust-owned extraction fields from a structured Legado
    /// `contentRule`.
    ///
    /// JS and host-side action fields remain outside this method; callers get a
    /// deterministic Core result for extractable text/URL fields only.
    pub fn run_legado_content_rule_with_context(
        &self,
        input: &str,
        rule: &ContentRule,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<(String, RuleOutput)>, ContentError> {
        let mut fields = self.run_legado_field_rules_with_context(
            input,
            legado_content_rule_fields(rule),
            context,
        )?;
        for (field, output) in &mut fields {
            if field == "content" {
                let content =
                    normalization::normalize_extracted_content(&output.values().join("\n"));
                *output = RuleOutput::new(vec![content]);
            }
        }
        Ok(fields)
    }

    /// Run a structured Legado `searchRule` by selecting book-list items and
    /// mapping supported item fields into [`Book`] values.
    pub fn run_legado_search_rule_with_context(
        &self,
        input: &str,
        rule: &SearchRule,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<Book>, ContentError> {
        let item_inputs =
            self.legado_list_item_inputs(input, rule.book_list.as_deref(), context)?;
        let mut books = Vec::new();
        for item_input in &item_inputs {
            let mut item_context = context.clone();
            let fields = self.run_legado_field_rules_with_context(
                item_input,
                legado_search_rule_fields(rule),
                &mut item_context,
            )?;
            books.push(book_from_legado_book_fields(&fields));
        }
        Ok(books)
    }

    /// Run a structured Legado `exploreRule` by selecting book-list items and
    /// mapping supported item fields into [`Book`] values.
    pub fn run_legado_explore_rule_with_context(
        &self,
        input: &str,
        rule: &ExploreRule,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<Book>, ContentError> {
        let item_inputs =
            self.legado_list_item_inputs(input, rule.book_list.as_deref(), context)?;
        let mut books = Vec::new();
        for item_input in &item_inputs {
            let mut item_context = context.clone();
            let fields = self.run_legado_field_rules_with_context(
                item_input,
                legado_explore_rule_fields(rule),
                &mut item_context,
            )?;
            books.push(book_from_legado_book_fields(&fields));
        }
        Ok(books)
    }

    /// Run a structured Legado `tocRule` by selecting chapter-list items and
    /// evaluating chapter fields inside each item context.
    pub fn run_legado_toc_rule_with_context(
        &self,
        input: &str,
        rule: &TocRule,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<TocEntry>, ContentError> {
        let item_inputs =
            self.legado_list_item_inputs(input, rule.chapter_list.as_deref(), context)?;
        let mut entries = Vec::new();
        for (index, item_input) in item_inputs.iter().enumerate() {
            let mut item_context = context.clone();
            let fields = self.run_legado_field_rules_with_context(
                item_input,
                legado_toc_rule_fields(rule),
                &mut item_context,
            )?;
            let title = first_field_value(&fields, "chapterName").unwrap_or_default();
            let url = first_field_value(&fields, "chapterUrl").unwrap_or_default();
            entries.push(TocEntry {
                index: index as u32,
                title,
                url,
            });
        }
        Ok(entries)
    }

    fn legado_list_item_inputs(
        &self,
        input: &str,
        list_rule: Option<&str>,
        context: &mut LegadoRuleContext,
    ) -> Result<Vec<String>, ContentError> {
        if let Some(list_rule) = list_rule.filter(|rule| !rule.trim().is_empty()) {
            Ok(self
                .engine
                .execute_legado_css_list_items_with_context(input, list_rule, context)?
                .into_values())
        } else {
            Ok(vec![input.to_string()])
        }
    }

    /// Extract a list of books from a search response.
    ///
    /// The rule chain is expected to yield, for each book, a JSON object string
    /// (or a value string) containing `bookId`/`title`/`author` fields. Non-JSON
    /// values are kept as the title with an empty book id.
    pub fn search(
        &self,
        source: &Source,
        search_response: &str,
    ) -> Result<Vec<Book>, ContentError> {
        if let Some(book_source) = legado_book_source(source)? {
            if let Some(search_rule) = book_source.search_rule.as_ref() {
                let mut context = legado_context_for_source(source, &book_source);
                return self.run_legado_search_rule_with_context(
                    search_response,
                    search_rule,
                    &mut context,
                );
            }
            if let Some(search_rule) = book_source
                .rule_search
                .as_ref()
                .and_then(|rule| rule.as_structured())
            {
                let mut context = legado_context_for_source(source, &book_source);
                return self.run_legado_search_rule_with_context(
                    search_response,
                    search_rule,
                    &mut context,
                );
            }
            if let Some(search_rule) = legacy_search_rule_from_book_source(&book_source) {
                let mut context = legado_context_for_source(source, &book_source);
                return self.run_legado_search_rule_with_context(
                    search_response,
                    &search_rule,
                    &mut context,
                );
            }
        }

        let out = self.run_chain(search_response, &source.rules.search)?;
        let mut books = Vec::new();
        for value in out.values() {
            books.push(parse_book_value(value));
        }
        Ok(books)
    }

    /// Extract an explore/discovery book list from a source response.
    ///
    /// Explore is a Legado BookSource capability rather than a V1 `SourceRules`
    /// stage, so sources without a structured `exploreRule` simply return an
    /// empty list.
    pub fn explore(
        &self,
        source: &Source,
        explore_response: &str,
    ) -> Result<Vec<Book>, ContentError> {
        if let Some(book_source) = legado_book_source(source)? {
            if let Some(explore_rule) = book_source.explore_rule.as_ref() {
                let mut context = legado_context_for_source(source, &book_source);
                return self.run_legado_explore_rule_with_context(
                    explore_response,
                    explore_rule,
                    &mut context,
                );
            }
        }

        Ok(Vec::new())
    }

    /// Merge detail metadata into a base book. Detail extraction yields key/value
    /// pairs that are folded into the book; the rule chain should produce an even
    /// list of `[key, value, key, value, ...]` strings, or a JSON object string.
    pub fn detail(
        &self,
        source: &Source,
        base: &Book,
        detail_response: &str,
    ) -> Result<Book, ContentError> {
        if let Some(book_source) = legado_book_source(source)? {
            if let Some(book_info_rule) = book_source.book_info_rule.as_ref() {
                let mut context = legado_context_for_source(source, &book_source);
                let fields = self.run_legado_book_info_rule_with_context(
                    detail_response,
                    book_info_rule,
                    &mut context,
                )?;
                let mut merged = base.clone();
                merge_legado_book_fields(&mut merged, &fields);
                return Ok(merged);
            }
        }

        let out = self.run_chain(detail_response, &source.rules.detail)?;
        let mut merged = base.clone();
        if let Some(first) = out.first() {
            if let Ok(map) = serde_json::from_str::<serde_json::Value>(first) {
                if let Some(obj) = map.as_object() {
                    merge_book(&mut merged, obj);
                    return Ok(merged);
                }
            }
        }
        // Fall back to flat key/value pairs.
        let vals = out.values();
        let mut iter = vals.iter();
        while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            match k.as_str() {
                "title" => merged.title = v.clone(),
                "author" => merged.author = v.clone(),
                "bookId" | "book_id" => merged.book_id = v.clone(),
                "coverUrl" | "cover_url" => merged.cover_url = Some(v.clone()),
                "intro" => merged.intro = Some(v.clone()),
                "kind" => merged.kind = Some(v.clone()),
                "lastChapter" | "last_chapter" => merged.last_chapter = Some(v.clone()),
                _ => {}
            }
        }
        Ok(merged)
    }

    /// Extract the table of contents. The rule chain should yield alternating
    /// `title`, `url` values, or a JSON array of `{title, url}` objects.
    pub fn toc(&self, source: &Source, toc_response: &str) -> Result<Vec<TocEntry>, ContentError> {
        if let Some(book_source) = legado_book_source(source)? {
            if let Some(toc_rule) = book_source.toc_rule.as_ref() {
                let mut context = legado_context_for_source(source, &book_source);
                return self.run_legado_toc_rule_with_context(toc_response, toc_rule, &mut context);
            }
        }

        let out = self.run_chain(toc_response, &source.rules.toc)?;
        let mut entries = Vec::new();

        // Try JSON array first.
        if let Some(first) = out.first() {
            if let Ok(arr) = serde_json::from_str::<serde_json::Value>(first) {
                if let Some(array) = arr.as_array() {
                    for (i, item) in array.iter().enumerate() {
                        entries.push(TocEntry {
                            index: i as u32,
                            title: item
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            url: item
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        });
                    }
                    return Ok(entries);
                }
            }
        }

        // Flat title/url pairs.
        let vals = out.values();
        let mut iter = vals.iter();
        let mut index = 0u32;
        while let (Some(title), Some(url)) = (iter.next(), iter.next()) {
            entries.push(TocEntry {
                index,
                title: title.clone(),
                url: url.clone(),
            });
            index += 1;
        }
        Ok(entries)
    }

    /// Extract chapter body text. Returns the joined output of the rule chain,
    /// normalized via [`normalization::normalize_extracted_content`] so raw HTML
    /// fragments and plain text both become stable chapter text.
    pub fn chapter_content(
        &self,
        source: &Source,
        chapter_response: &str,
    ) -> Result<String, ContentError> {
        if let Some(book_source) = legado_book_source(source)? {
            if let Some(content_rule) = book_source.content_rule.as_ref() {
                let mut context = legado_context_for_source(source, &book_source);
                let fields = self.run_legado_content_rule_with_context(
                    chapter_response,
                    content_rule,
                    &mut context,
                )?;
                return Ok(first_field_value(&fields, "content").unwrap_or_default());
            }
        }

        let out = self.run_chain(chapter_response, &source.rules.chapter)?;
        Ok(normalization::normalize_extracted_content(
            &out.values().join("\n"),
        ))
    }

    /// Extract and normalize one chapter body for cache/storage.
    pub fn chapter_document(
        &self,
        source: &Source,
        book: &Book,
        toc_entry: &TocEntry,
        chapter_response: &str,
    ) -> Result<NormalizedChapter, ContentError> {
        let content = self.chapter_content(source, chapter_response)?;
        normalize_chapter(source, book, toc_entry, &content)
    }

    /// Evaluate a JS rule against `input`. If the script calls a host capability
    /// (e.g. `java.get`) and no callback is registered, returns
    /// [`JsOutcome::Unsupported`] rather than pretending a network call happened.
    pub fn evaluate_js_rule(&self, script: &str) -> JsOutcome {
        match self.js.evaluate(script) {
            Ok(JsEvaluation { value, .. }) => JsOutcome::Ok(value),
            Err(e) => {
                if is_unsupported_host_call(&e) {
                    JsOutcome::Unsupported {
                        reason: e.message.clone(),
                    }
                } else {
                    // Non-unsupported JS errors are surfaced to the caller as a
                    // hard error via the Content pipeline; here we map to Ok(null)
                    // is wrong, so re-raise by returning Unsupported only for the
                    // host-call case. For other errors, wrap into a synthetic
                    // unsupported outcome with the real reason so the caller gets
                    // a structured signal instead of a panic.
                    JsOutcome::Unsupported {
                        reason: format!("{}: {}", js_kind_label(&e.kind), e.message),
                    }
                }
            }
        }
    }
}

fn legado_book_info_rule_fields(rule: &BookInfoRule) -> Vec<(&'static str, &str)> {
    let mut fields = Vec::new();
    push_optional_legado_field(&mut fields, "name", &rule.name);
    push_optional_legado_field(&mut fields, "author", &rule.author);
    push_optional_legado_field(&mut fields, "intro", &rule.intro);
    push_optional_legado_field(&mut fields, "kind", &rule.kind);
    push_optional_legado_field(&mut fields, "lastChapter", &rule.last_chapter);
    push_optional_legado_field(&mut fields, "updateTime", &rule.update_time);
    push_optional_legado_field(&mut fields, "coverUrl", &rule.cover_url);
    push_optional_legado_field(&mut fields, "tocUrl", &rule.toc_url);
    push_optional_legado_field(&mut fields, "wordCount", &rule.word_count);
    push_optional_legado_field(&mut fields, "canReName", &rule.can_re_name);
    push_optional_legado_field(&mut fields, "downloadUrls", &rule.download_urls);
    push_optional_legado_field(&mut fields, "reply", &rule.reply);
    fields
}

fn legado_search_rule_fields(rule: &SearchRule) -> Vec<(&'static str, &str)> {
    let mut fields = Vec::new();
    push_optional_legado_field(&mut fields, "name", &rule.name);
    push_optional_legado_field(&mut fields, "author", &rule.author);
    push_optional_legado_field(&mut fields, "intro", &rule.intro);
    push_optional_legado_field(&mut fields, "kind", &rule.kind);
    push_optional_legado_field(&mut fields, "lastChapter", &rule.last_chapter);
    push_optional_legado_field(&mut fields, "updateTime", &rule.update_time);
    push_optional_legado_field(&mut fields, "bookUrl", &rule.book_url);
    push_optional_legado_field(&mut fields, "coverUrl", &rule.cover_url);
    push_optional_legado_field(&mut fields, "wordCount", &rule.word_count);
    push_optional_legado_field(&mut fields, "checkKeyWord", &rule.check_key_word);
    push_optional_legado_field(&mut fields, "searchFields", &rule.search_fields);
    fields
}

fn legado_explore_rule_fields(rule: &ExploreRule) -> Vec<(&'static str, &str)> {
    let mut fields = Vec::new();
    push_optional_legado_field(&mut fields, "name", &rule.name);
    push_optional_legado_field(&mut fields, "author", &rule.author);
    push_optional_legado_field(&mut fields, "intro", &rule.intro);
    push_optional_legado_field(&mut fields, "kind", &rule.kind);
    push_optional_legado_field(&mut fields, "lastChapter", &rule.last_chapter);
    push_optional_legado_field(&mut fields, "bookUrl", &rule.book_url);
    push_optional_legado_field(&mut fields, "coverUrl", &rule.cover_url);
    push_optional_legado_field(&mut fields, "wordCount", &rule.word_count);
    fields
}

fn book_from_legado_book_fields(fields: &[(String, RuleOutput)]) -> Book {
    Book {
        book_id: first_field_value(fields, "bookUrl").unwrap_or_default(),
        title: first_field_value(fields, "name").unwrap_or_default(),
        author: first_field_value(fields, "author").unwrap_or_default(),
        cover_url: first_non_empty_field_value(fields, "coverUrl"),
        intro: first_non_empty_field_value(fields, "intro"),
        kind: first_non_empty_field_value(fields, "kind"),
        last_chapter: first_non_empty_field_value(fields, "lastChapter"),
    }
}

fn merge_legado_book_fields(book: &mut Book, fields: &[(String, RuleOutput)]) {
    for (field, output) in fields {
        let Some(value) = output.first().filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        match field.as_str() {
            "name" => book.title = value.to_string(),
            "author" => book.author = value.to_string(),
            "coverUrl" => book.cover_url = Some(value.to_string()),
            "intro" => book.intro = Some(value.to_string()),
            "kind" => book.kind = Some(value.to_string()),
            "lastChapter" => book.last_chapter = Some(value.to_string()),
            _ => {}
        }
    }
}

fn legado_content_rule_fields(rule: &ContentRule) -> Vec<(&'static str, &str)> {
    let mut fields = Vec::new();
    push_optional_legado_field(&mut fields, "title", &rule.title);
    push_optional_legado_field(&mut fields, "content", &rule.content);
    push_optional_legado_field(&mut fields, "nextContentUrl", &rule.next_content_url);
    fields
}

fn legado_toc_rule_fields(rule: &TocRule) -> Vec<(&'static str, &str)> {
    let mut fields = Vec::new();
    push_optional_legado_field(&mut fields, "chapterName", &rule.chapter_name);
    push_optional_legado_field(&mut fields, "chapterUrl", &rule.chapter_url);
    push_optional_legado_field(&mut fields, "isVolume", &rule.is_volume);
    push_optional_legado_field(&mut fields, "isVip", &rule.is_vip);
    push_optional_legado_field(&mut fields, "isPay", &rule.is_pay);
    push_optional_legado_field(&mut fields, "updateTime", &rule.update_time);
    push_optional_legado_field(&mut fields, "nextTocUrl", &rule.next_toc_url);
    fields
}

fn first_field_value(fields: &[(String, RuleOutput)], target: &str) -> Option<String> {
    fields
        .iter()
        .find(|(field, _)| field == target)
        .and_then(|(_, output)| output.first().map(ToString::to_string))
}

fn first_non_empty_field_value(fields: &[(String, RuleOutput)], target: &str) -> Option<String> {
    first_field_value(fields, target).filter(|value| !value.trim().is_empty())
}

fn push_optional_legado_field<'a>(
    fields: &mut Vec<(&'static str, &'a str)>,
    field: &'static str,
    rule: &'a Option<String>,
) {
    if let Some(rule) = rule.as_deref().filter(|rule| !rule.trim().is_empty()) {
        fields.push((field, rule));
    }
}

fn legado_book_source(source: &Source) -> Result<Option<LegadoBookSource>, ContentError> {
    if source.book_source.is_null() {
        return Ok(None);
    }

    serde_json::from_value(source.book_source.clone())
        .map(Some)
        .map_err(|e| ContentError::RuleSpec(format!("invalid Legado bookSource: {e}")))
}

fn legacy_search_rule_from_book_source(book_source: &LegadoBookSource) -> Option<SearchRule> {
    let rule = SearchRule {
        book_list: book_source
            .rule_search
            .as_ref()
            .and_then(|rule| rule.as_raw())
            .map(ToString::to_string)
            .filter(|value| !value.trim().is_empty()),
        name: non_empty_string(book_source.rule_search_name.clone()),
        author: non_empty_string(book_source.rule_search_author.clone()),
        book_url: non_empty_string(book_source.rule_search_note_url.clone())
            .or_else(|| non_empty_string(book_source.rule_search_url.clone())),
        ..SearchRule::default()
    };

    if rule.book_list.is_some()
        || rule.name.is_some()
        || rule.author.is_some()
        || rule.book_url.is_some()
    {
        Some(rule)
    } else {
        None
    }
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn legado_context_for_source(source: &Source, book_source: &LegadoBookSource) -> LegadoRuleContext {
    let mut context = LegadoRuleContext::new();
    if let Some(url) = book_source
        .book_source_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .or_else(|| (!source.base_url.trim().is_empty()).then_some(source.base_url.as_str()))
    {
        context.put_variable("sourceHost", url);
        context.put_variable("bookSourceUrl", url);
    }
    context
}

/// Normalize a chapter body independent of the extraction path.
pub fn normalize_chapter(
    source: &Source,
    book: &Book,
    toc_entry: &TocEntry,
    raw_content: &str,
) -> Result<NormalizedChapter, ContentError> {
    validate_non_empty("source_id", &source.source_id)?;
    validate_non_empty("book_id", &book.book_id)?;
    let content = normalize_chapter_content(raw_content);
    let content = remove_leading_duplicate_chapter_title(&content, &book.title, &toc_entry.title);
    if content.trim().is_empty() {
        return Err(ContentError::InvalidChapter {
            field: "content".into(),
        });
    }
    let paragraphs = split_paragraphs(&content);
    let fingerprint = stable_fingerprint(&content);
    let cache_key = format!(
        "{}:{}:{}:{}",
        source.source_id, book.book_id, toc_entry.index, fingerprint
    );
    Ok(NormalizedChapter {
        source_id: source.source_id.clone(),
        book_id: book.book_id.clone(),
        chapter_index: toc_entry.index,
        title: toc_entry.title.clone(),
        url: toc_entry.url.clone(),
        char_len: content.chars().count(),
        content,
        paragraphs,
        content_fingerprint: fingerprint,
        cache_key,
    })
}

/// Diff two TOCs and preserve chapter identity across refreshes.
///
/// Identity first uses URL/path when present, then title. Duplicate URLs or
/// titles are matched by occurrence order so repeated canonical locators do not
/// collapse into a single chapter.
pub fn diff_toc(old_toc: &[TocEntry], new_toc: &[TocEntry]) -> TocRefreshDiff {
    let old_identities = toc_identities(old_toc);
    let new_identities = toc_identities(new_toc);
    let old_title_identities = title_occurrences(old_toc);
    let mut url_map = HashMap::<(String, usize), u32>::new();
    let mut title_map = HashMap::<(String, usize), u32>::new();
    for (entry, identity) in new_toc.iter().zip(new_identities.iter()) {
        match identity.kind {
            ChapterIdentityKind::Url => {
                url_map.insert(
                    (identity.value.clone(), identity.occurrence),
                    identity.index,
                );
            }
            ChapterIdentityKind::Title => {
                title_map.insert(
                    (identity.value.clone(), identity.occurrence),
                    identity.index,
                );
            }
        }
        if let Some((title, occurrence)) = title_occurrence_for(new_toc, entry.index) {
            title_map.insert((title, occurrence), entry.index);
        }
    }

    let mut mapped_new_indexes = Vec::new();
    let mut mappings = Vec::new();
    let mut removed = Vec::new();
    for ((entry, identity), title_identity) in old_toc
        .iter()
        .zip(old_identities.iter())
        .zip(old_title_identities.iter())
    {
        let new_index = match identity.kind {
            ChapterIdentityKind::Url => url_map
                .get(&(identity.value.clone(), identity.occurrence))
                .copied()
                .or_else(|| {
                    title_identity
                        .clone()
                        .and_then(|title| title_map.get(&title).copied())
                }),
            ChapterIdentityKind::Title => title_map
                .get(&(identity.value.clone(), identity.occurrence))
                .copied(),
        };
        if let Some(index) = new_index {
            mapped_new_indexes.push(index);
        } else {
            removed.push(entry.clone());
        }
        mappings.push(TocRemapEntry {
            old_index: entry.index,
            new_index,
            identity_kind: identity.kind,
            identity: identity.value.clone(),
        });
    }

    let inserted = new_toc
        .iter()
        .filter(|entry| !mapped_new_indexes.contains(&entry.index))
        .cloned()
        .collect();

    TocRefreshDiff {
        old_len: old_toc.len(),
        new_len: new_toc.len(),
        mappings,
        inserted,
        removed,
    }
}

/// Remap reading progress after a TOC refresh.
pub fn remap_reading_progress(
    progress: &ReadingProgress,
    diff: &TocRefreshDiff,
) -> RemappedReadingProgress {
    if diff.new_len == 0 {
        return RemappedReadingProgress {
            progress: ReadingProgress {
                book_id: progress.book_id.clone(),
                chapter_index: 0,
                chapter_offset: 0,
                chapter_progress: 0.0,
            },
            status: ProgressRemapStatus::EmptyToc,
        };
    }

    if let Some(mapping) = diff
        .mappings
        .iter()
        .find(|mapping| mapping.old_index == progress.chapter_index)
    {
        if let Some(new_index) = mapping.new_index {
            let mut remapped = progress.clone();
            remapped.chapter_index = new_index;
            return RemappedReadingProgress {
                status: if new_index == progress.chapter_index {
                    ProgressRemapStatus::Unchanged
                } else {
                    ProgressRemapStatus::Remapped
                },
                progress: remapped,
            };
        }
    }

    let fallback_index = nearest_surviving_index(progress.chapter_index, &diff.mappings);
    RemappedReadingProgress {
        progress: ReadingProgress {
            book_id: progress.book_id.clone(),
            chapter_index: fallback_index,
            chapter_offset: 0,
            chapter_progress: 0.0,
        },
        status: ProgressRemapStatus::ChapterRemovedClamped,
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), ContentError> {
    if value.trim().is_empty() {
        return Err(ContentError::InvalidChapter {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_document_non_empty(field: &str, value: &str) -> Result<(), ContentError> {
    if value.trim().is_empty() {
        return Err(ContentError::InvalidDocument {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_content_document(document: &ContentDocument) -> Result<(), ContentError> {
    validate_document_non_empty("source_id", &document.source_id)?;
    validate_document_non_empty("book.book_id", &document.book.book_id)?;
    if document.toc.is_empty() || document.chapters.is_empty() {
        return Err(ContentError::InvalidDocument {
            field: "chapters".into(),
        });
    }
    if document.toc.len() != document.chapters.len() {
        return Err(ContentError::InvalidDocument {
            field: "toc".into(),
        });
    }

    for (expected_index, (toc, chapter)) in document
        .toc
        .iter()
        .zip(document.chapters.iter())
        .enumerate()
    {
        let expected_index = expected_index as u32;
        if toc.index != expected_index {
            return Err(ContentError::InvalidDocument {
                field: "toc.index".into(),
            });
        }
        if chapter.chapter_index != expected_index {
            return Err(ContentError::InvalidDocument {
                field: "chapters.index".into(),
            });
        }
        validate_normalized_chapter_for_document(document, toc, chapter)?;
    }

    let expected_fingerprint = content_document_fingerprint(document);
    if document.content_fingerprint != expected_fingerprint {
        return Err(ContentError::InvalidDocument {
            field: "content_fingerprint".into(),
        });
    }
    Ok(())
}

fn validate_normalized_chapter_for_document(
    document: &ContentDocument,
    toc: &TocEntry,
    chapter: &NormalizedChapter,
) -> Result<(), ContentError> {
    if chapter.source_id != document.source_id {
        return Err(ContentError::InvalidDocument {
            field: "chapters.source_id".into(),
        });
    }
    if chapter.book_id != document.book.book_id {
        return Err(ContentError::InvalidDocument {
            field: "chapters.book_id".into(),
        });
    }
    if chapter.title != toc.title {
        return Err(ContentError::InvalidDocument {
            field: "chapters.title".into(),
        });
    }
    if chapter.url != toc.url {
        return Err(ContentError::InvalidDocument {
            field: "chapters.url".into(),
        });
    }
    if chapter.content.trim().is_empty() {
        return Err(ContentError::InvalidDocument {
            field: "chapters.content".into(),
        });
    }
    if chapter.char_len != chapter.content.chars().count() {
        return Err(ContentError::InvalidDocument {
            field: "chapters.char_len".into(),
        });
    }
    let expected_fingerprint = stable_fingerprint(&chapter.content);
    if chapter.content_fingerprint != expected_fingerprint {
        return Err(ContentError::InvalidDocument {
            field: "chapters.content_fingerprint".into(),
        });
    }
    let expected_cache_key = format!(
        "{}:{}:{}:{}",
        chapter.source_id, chapter.book_id, chapter.chapter_index, chapter.content_fingerprint
    );
    if chapter.cache_key != expected_cache_key {
        return Err(ContentError::InvalidDocument {
            field: "chapters.cache_key".into(),
        });
    }
    Ok(())
}

fn content_document_key(
    source_id: &str,
    book_id: &str,
) -> Result<ContentDocumentKey, ContentError> {
    validate_document_non_empty("source_id", source_id)?;
    validate_document_non_empty("book_id", book_id)?;
    Ok(ContentDocumentKey {
        source_id: source_id.trim().to_string(),
        book_id: book_id.trim().to_string(),
    })
}

fn sort_content_snapshot(snapshot: &mut ContentLibrarySnapshot) {
    snapshot.documents.sort_by(compare_content_document_key);
}

fn compare_content_document_key(a: &ContentDocument, b: &ContentDocument) -> std::cmp::Ordering {
    a.source_id
        .cmp(&b.source_id)
        .then_with(|| a.book.book_id.cmp(&b.book.book_id))
}

fn content_document_fingerprint(document: &ContentDocument) -> String {
    let mut parts = Vec::new();
    parts.push(document.source_id.as_str());
    parts.push(document.book.book_id.as_str());
    for toc in &document.toc {
        parts.push(&toc.title);
        parts.push(&toc.url);
    }
    for chapter in &document.chapters {
        parts.push(&chapter.content_fingerprint);
    }
    stable_fingerprint(&parts.join("\u{1f}"))
}

fn normalize_chapter_content(raw: &str) -> String {
    let normalized = raw
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut output = Vec::new();
    let mut blank_run = 0usize;
    for line in normalized.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                output.push(String::new());
            }
        } else {
            blank_run = 0;
            output.push(line.to_string());
        }
    }
    while output.first().is_some_and(|line| line.is_empty()) {
        output.remove(0);
    }
    while output.last().is_some_and(|line| line.is_empty()) {
        output.pop();
    }
    output.join("\n")
}

fn remove_leading_duplicate_chapter_title(
    content: &str,
    book_title: &str,
    chapter_title: &str,
) -> String {
    let Some(prefix_end) = leading_duplicate_chapter_title_end(content, book_title, chapter_title)
    else {
        return content.to_string();
    };
    normalize_chapter_content(&content[prefix_end..])
}

fn leading_duplicate_chapter_title_end(
    content: &str,
    book_title: &str,
    chapter_title: &str,
) -> Option<usize> {
    let chapter_title = chapter_title.trim();
    if chapter_title.is_empty() {
        return None;
    }

    let mut cursor = 0usize;
    let book_title = book_title.trim();
    loop {
        let next = skip_duplicate_title_prefix_tokens(content, cursor, book_title);
        if next == cursor {
            break;
        }
        cursor = next;
    }

    let mut cursor = match_flexible_title_at(content, cursor, chapter_title)?;
    if let Some(next) = content[cursor..].chars().next() {
        if !next.is_whitespace() {
            return None;
        }
    }
    while cursor < content.len() {
        let next = content[cursor..].chars().next()?;
        if !next.is_whitespace() {
            break;
        }
        cursor += next.len_utf8();
    }
    Some(cursor)
}

fn skip_duplicate_title_prefix_tokens(content: &str, mut cursor: usize, book_title: &str) -> usize {
    while cursor < content.len() {
        let Some(next) = content[cursor..].chars().next() else {
            break;
        };
        if !is_duplicate_title_prefix_separator(next) {
            break;
        }
        cursor += next.len_utf8();
    }

    if !book_title.is_empty() && content[cursor..].starts_with(book_title) {
        cursor + book_title.len()
    } else {
        cursor
    }
}

fn match_flexible_title_at(content: &str, mut cursor: usize, title: &str) -> Option<usize> {
    for expected in title.chars() {
        if expected.is_whitespace() {
            while cursor < content.len() {
                let next = content[cursor..].chars().next()?;
                if !next.is_whitespace() {
                    break;
                }
                cursor += next.len_utf8();
            }
            continue;
        }

        let actual = content[cursor..].chars().next()?;
        if actual != expected {
            return None;
        }
        cursor += actual.len_utf8();
    }
    Some(cursor)
}

fn is_duplicate_title_prefix_separator(value: char) -> bool {
    value.is_whitespace()
        || value.is_ascii_punctuation()
        || matches!(
            value,
            '　' | '。'
                | '，'
                | '：'
                | '；'
                | '、'
                | '！'
                | '？'
                | '《'
                | '》'
                | '「'
                | '」'
                | '『'
                | '』'
                | '（'
                | '）'
                | '【'
                | '】'
                | '—'
                | '–'
                | '…'
                | '·'
        )
}

fn split_paragraphs(content: &str) -> Vec<String> {
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| paragraph.replace('\n', "\n"))
        .collect()
}

fn stable_fingerprint(content: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChapterIdentity {
    index: u32,
    value: String,
    occurrence: usize,
    kind: ChapterIdentityKind,
}

fn toc_identities(toc: &[TocEntry]) -> Vec<ChapterIdentity> {
    let mut url_counts = HashMap::<String, usize>::new();
    let mut title_counts = HashMap::<String, usize>::new();
    toc.iter()
        .map(|entry| {
            if let Some(url) = normalize_identity(&entry.url) {
                let occurrence = *url_counts.get(&url).unwrap_or(&0);
                url_counts.insert(url.clone(), occurrence + 1);
                ChapterIdentity {
                    index: entry.index,
                    value: url,
                    occurrence,
                    kind: ChapterIdentityKind::Url,
                }
            } else {
                let title =
                    normalize_identity(&entry.title).unwrap_or_else(|| entry.index.to_string());
                let occurrence = *title_counts.get(&title).unwrap_or(&0);
                title_counts.insert(title.clone(), occurrence + 1);
                ChapterIdentity {
                    index: entry.index,
                    value: title,
                    occurrence,
                    kind: ChapterIdentityKind::Title,
                }
            }
        })
        .collect()
}

fn title_occurrences(toc: &[TocEntry]) -> Vec<Option<(String, usize)>> {
    toc.iter()
        .map(|entry| title_occurrence_for(toc, entry.index))
        .collect()
}

fn title_occurrence_for(toc: &[TocEntry], index: u32) -> Option<(String, usize)> {
    let mut counts = HashMap::<String, usize>::new();
    for entry in toc {
        let Some(title) = normalize_identity(&entry.title) else {
            continue;
        };
        let occurrence = *counts.get(&title).unwrap_or(&0);
        if entry.index == index {
            return Some((title, occurrence));
        }
        counts.insert(title, occurrence + 1);
    }
    None
}

fn normalize_identity(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.split('#').next().unwrap_or(value).trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn nearest_surviving_index(old_index: u32, mappings: &[TocRemapEntry]) -> u32 {
    if let Some(next) = mappings
        .iter()
        .filter(|mapping| mapping.old_index > old_index)
        .filter_map(|mapping| mapping.new_index)
        .next()
    {
        return next;
    }
    mappings
        .iter()
        .rev()
        .filter(|mapping| mapping.old_index < old_index)
        .filter_map(|mapping| mapping.new_index)
        .next()
        .unwrap_or(0)
}

fn js_kind_label(kind: &JsErrorKind) -> &'static str {
    match kind {
        JsErrorKind::Cancelled => "cancelled",
        JsErrorKind::Exception => "exception",
        JsErrorKind::HostCallback => "host-callback",
        JsErrorKind::Internal => "internal",
        JsErrorKind::MemoryLimit => "memory-limit",
        JsErrorKind::NonJsonValue => "non-json-value",
        JsErrorKind::Syntax => "syntax",
        JsErrorKind::Timeout => "timeout",
        JsErrorKind::Unsupported => "unsupported",
    }
}

/// A JS error counts as "unsupported in V1" when it stems from an unregistered
/// host callback (the only way JS rules reach outside the sandbox in V1).
fn is_unsupported_host_call(e: &JsError) -> bool {
    matches!(e.kind, JsErrorKind::HostCallback | JsErrorKind::Unsupported)
        && e.message.contains("unregistered host callback")
}

fn parse_book_value(value: &str) -> Book {
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(value) {
        if let Some(map) = obj.as_object() {
            let mut book = Book {
                book_id: String::new(),
                title: String::new(),
                author: String::new(),
                cover_url: None,
                intro: None,
                kind: None,
                last_chapter: None,
            };
            merge_book(&mut book, map);
            return book;
        }
    }
    // Non-JSON: treat the whole string as the title.
    Book {
        book_id: String::new(),
        title: value.to_string(),
        author: String::new(),
        cover_url: None,
        intro: None,
        kind: None,
        last_chapter: None,
    }
}

fn merge_book(book: &mut Book, map: &serde_json::Map<String, serde_json::Value>) {
    if let Some(v) = map.get("bookId").and_then(|v| v.as_str()) {
        book.book_id = v.to_string();
    }
    if let Some(v) = map.get("book_id").and_then(|v| v.as_str()) {
        book.book_id = v.to_string();
    }
    if let Some(v) = map.get("title").and_then(|v| v.as_str()) {
        book.title = v.to_string();
    }
    if let Some(v) = map.get("author").and_then(|v| v.as_str()) {
        book.author = v.to_string();
    }
    if let Some(v) = map.get("coverUrl").and_then(|v| v.as_str()) {
        book.cover_url = Some(v.to_string());
    }
    if let Some(v) = map.get("intro").and_then(|v| v.as_str()) {
        book.intro = Some(v.to_string());
    }
    if let Some(v) = map.get("kind").and_then(|v| v.as_str()) {
        book.kind = Some(v.to_string());
    }
    if let Some(v) = map.get("lastChapter").and_then(|v| v.as_str()) {
        book.last_chapter = Some(v.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_domain::{BookInfoRule, ContentRule, SearchRule, SourceRules, TocRule};
    use reader_js::{HostCallbackRegistry, JsRuntimeConfig};

    fn sample_source() -> Source {
        Source {
            source_id: "src1".into(),
            name: "Sample".into(),
            base_url: "https://example.test".into(),
            rules: SourceRules::default(),
            book_source: serde_json::Value::Null,
        }
    }

    #[test]
    fn rule_step_spec_round_trips_json_path() {
        let spec: RuleStepSpec =
            serde_json::from_str(r#"{"kind":"jsonPath","path":"$.books[*].title"}"#).unwrap();
        let step = spec.into_rule_step().unwrap();
        let engine = RuleEngine::new();
        let out = engine
            .execute_step(
                r#"{"books":[{"title":"Dune"},{"title":"Foundation"}]}"#,
                &step,
            )
            .unwrap();
        assert_eq!(out.values(), &["Dune", "Foundation"]);
    }

    #[test]
    fn rule_step_spec_rejects_raw_legado_dsl_strings() {
        let err = serde_json::from_str::<RuleStepSpec>(r#""div.list&&div.item;div.name&&a@text""#)
            .expect_err("raw Legado DSL must stay outside RuleStepSpec");

        assert!(
            err.to_string().contains("invalid type"),
            "unexpected raw DSL parse error: {err}"
        );
    }

    #[test]
    fn search_extracts_books_from_json() {
        let mut source = sample_source();
        source.rules.search = serde_json::json!([
            { "kind": "jsonPath", "path": "$.books[*]" }
        ]);
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"{"books":[
            {"bookId":"1","title":"Dune","author":"Herbert"},
            {"bookId":"2","title":"Foundation","author":"Asimov"}
        ]}"#;
        let books = pipeline.search(&source, resp).unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].book_id, "1");
        assert_eq!(books[1].author, "Asimov");
    }

    #[test]
    fn search_with_empty_rules_returns_empty() {
        let source = sample_source();
        let pipeline = RemoteContentPipeline::new();
        let books = pipeline.search(&source, "anything").unwrap();
        assert!(books.is_empty());
    }

    #[test]
    fn search_accepts_raw_legado_css_rule_string() {
        let mut source = sample_source();
        source.rules.search = serde_json::json!("div.list&&div.item;div.name&&a@text");
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <main>
                <div class="list">
                    <div class="item"><div class="name"><a href="/b/1">Dune</a></div></div>
                    <div class="item"><div class="name"><a href="/b/2">Foundation</a></div></div>
                </div>
            </main>
        "#;

        let books = pipeline.search(&source, resp).unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[1].title, "Foundation");
    }

    #[test]
    fn search_uses_structured_legado_book_source_search_rule() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legado Compat",
            "bookSourceUrl": "https://books.example.test",
            "searchRule": {
                "bookList": "section.results&&article.book",
                "name": "a.title@text@put:{href:\"a.title@href\"}",
                "author": "span.author@text",
                "bookUrl": "@get:{sourceHost}@get:{href}",
                "coverUrl": "img.cover@src",
                "lastChapter": "span.latest@text"
            }
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <section class="results">
                <article class="book">
                    <a class="title" href="/book/1">Dune</a>
                    <span class="author">Frank Herbert</span>
                    <img class="cover" src="/covers/dune.jpg">
                    <span class="latest">Chapter 7</span>
                </article>
                <article class="book">
                    <a class="title" href="/book/2">Foundation</a>
                    <span class="author">Isaac Asimov</span>
                    <img class="cover" src="/covers/foundation.jpg">
                    <span class="latest">Chapter 3</span>
                </article>
            </section>
        "#;

        let books = pipeline.search(&source, resp).unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(books[0].book_id, "https://books.example.test/book/1");
        assert_eq!(books[0].cover_url.as_deref(), Some("/covers/dune.jpg"));
        assert_eq!(books[0].last_chapter.as_deref(), Some("Chapter 7"));
        assert_eq!(books[1].title, "Foundation");
        assert_eq!(books[1].author, "Isaac Asimov");
        assert_eq!(books[1].book_id, "https://books.example.test/book/2");
        assert_eq!(
            books[1].cover_url.as_deref(),
            Some("/covers/foundation.jpg")
        );
        assert_eq!(books[1].last_chapter.as_deref(), Some("Chapter 3"));
    }

    #[test]
    fn search_uses_legado_rule_search_object() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legado Compat",
            "bookSourceUrl": "https://books.example.test",
            "ruleSearch": {
                "bookList": "section.results&&article.book",
                "name": "a.title@text@put:{href:\"a.title@href\"}",
                "author": "span.author@text",
                "bookUrl": "@get:{sourceHost}@get:{href}",
                "coverUrl": "img.cover@src",
                "lastChapter": "span.latest@text"
            }
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <section class="results">
                <article class="book">
                    <a class="title" href="/book/1">Dune</a>
                    <span class="author">Frank Herbert</span>
                    <img class="cover" src="/covers/dune.jpg">
                    <span class="latest">Chapter 7</span>
                </article>
                <article class="book">
                    <a class="title" href="/book/2">Foundation</a>
                    <span class="author">Isaac Asimov</span>
                    <img class="cover" src="/covers/foundation.jpg">
                    <span class="latest">Chapter 3</span>
                </article>
            </section>
        "#;

        let books = pipeline.search(&source, resp).unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(books[0].book_id, "https://books.example.test/book/1");
        assert_eq!(books[0].cover_url.as_deref(), Some("/covers/dune.jpg"));
        assert_eq!(books[0].last_chapter.as_deref(), Some("Chapter 7"));
        assert_eq!(books[1].title, "Foundation");
        assert_eq!(books[1].author, "Isaac Asimov");
        assert_eq!(books[1].book_id, "https://books.example.test/book/2");
        assert_eq!(
            books[1].cover_url.as_deref(),
            Some("/covers/foundation.jpg")
        );
        assert_eq!(books[1].last_chapter.as_deref(), Some("Chapter 3"));
    }

    #[test]
    fn search_uses_legacy_legado_rule_search_fields() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legacy Legado Compat",
            "bookSourceUrl": "https://legacy.example.test",
            "ruleSearch": "css:.item",
            "ruleSearchName": "css:a.title",
            "ruleSearchAuthor": "css:p.author",
            "ruleSearchUrl": "css:a.title@href"
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <section class="results">
                <div class="item">
                    <a class="title" href="/book/1">Dune</a>
                    <p class="author">Frank Herbert</p>
                </div>
                <div class="item">
                    <a class="title" href="/book/2">Foundation</a>
                    <p class="author">Isaac Asimov</p>
                </div>
            </section>
        "#;

        let books = pipeline.search(&source, resp).unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(books[0].book_id, "/book/1");
        assert_eq!(books[1].title, "Foundation");
        assert_eq!(books[1].author, "Isaac Asimov");
        assert_eq!(books[1].book_id, "/book/2");
    }

    #[test]
    fn search_uses_legacy_legado_rule_search_note_url_for_book_url() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legacy Legado Compat",
            "bookSourceUrl": "https://legacy.example.test",
            "ruleSearchUrl": "/search?keyword={{key}}",
            "ruleSearch": "css:.item",
            "ruleSearchName": "css:a.title",
            "ruleSearchAuthor": "css:p.author",
            "ruleSearchNoteUrl": "css:a.title@href"
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <section class="results">
                <div class="item">
                    <a class="title" href="/book/1">Dune</a>
                    <p class="author">Frank Herbert</p>
                </div>
            </section>
        "#;

        let books = pipeline.search(&source, resp).unwrap();

        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(books[0].book_id, "/book/1");
    }

    #[test]
    fn explore_uses_structured_legado_book_source_explore_rule() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legado Compat",
            "bookSourceUrl": "https://books.example.test",
            "enabledExplore": true,
            "exploreRule": {
                "bookList": "div.explore&&article.book",
                "name": "a.title@text@put:{href:\"a.title@href\"}",
                "author": "span.author@text",
                "bookUrl": "@get:{sourceHost}@get:{href}",
                "coverUrl": "img.cover@src",
                "kind": "span.kind@text",
                "intro": "p.intro@text",
                "lastChapter": "span.latest@text"
            }
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <div class="explore">
                <article class="book">
                    <a class="title" href="/explore/book/1">Dune</a>
                    <span class="author">Frank Herbert</span>
                    <img class="cover" src="/covers/dune.jpg">
                    <span class="kind">Sci-Fi</span>
                    <p class="intro">A spice novel</p>
                    <span class="latest">Chapter 7</span>
                </article>
                <article class="book">
                    <a class="title" href="/explore/book/2">Foundation</a>
                    <span class="author">Isaac Asimov</span>
                    <img class="cover" src="/covers/foundation.jpg">
                    <span class="kind">Sci-Fi</span>
                    <p class="intro">A psychohistory novel</p>
                    <span class="latest">Chapter 3</span>
                </article>
            </div>
        "#;

        let books = pipeline.explore(&source, resp).unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(
            books[0].book_id,
            "https://books.example.test/explore/book/1"
        );
        assert_eq!(books[0].cover_url.as_deref(), Some("/covers/dune.jpg"));
        assert_eq!(books[0].kind.as_deref(), Some("Sci-Fi"));
        assert_eq!(books[0].intro.as_deref(), Some("A spice novel"));
        assert_eq!(books[0].last_chapter.as_deref(), Some("Chapter 7"));
        assert_eq!(books[1].title, "Foundation");
        assert_eq!(books[1].author, "Isaac Asimov");
        assert_eq!(
            books[1].book_id,
            "https://books.example.test/explore/book/2"
        );
        assert_eq!(books[1].intro.as_deref(), Some("A psychohistory novel"));
        assert_eq!(books[1].last_chapter.as_deref(), Some("Chapter 3"));
    }

    #[test]
    fn legado_search_rule_runs_fields_per_book_list_item() {
        let pipeline = RemoteContentPipeline::new();
        let mut context = reader_rule::LegadoRuleContext::new();
        context.put_variable("sourceHost", "https://books.example.test");
        let rule = SearchRule {
            book_list: Some("div.list&&article.book".into()),
            name: Some(r#"a.title@text@put:{href:"a.title@href"}"#.into()),
            author: Some("span.author@text".into()),
            book_url: Some("@get:{sourceHost}@get:{href}".into()),
            cover_url: Some("img.cover@src".into()),
            kind: Some("span.kind@text".into()),
            last_chapter: Some("span.latest@text".into()),
            ..SearchRule::default()
        };
        let html = r#"
            <div class="list">
                <article class="book">
                    <a class="title" href="/book/1">Dune</a>
                    <span class="author">Frank Herbert</span>
                    <img class="cover" src="/covers/dune.jpg">
                    <span class="kind">Sci-Fi</span>
                    <span class="latest">Chapter 7</span>
                </article>
                <article class="book">
                    <a class="title" href="/book/2">Foundation</a>
                    <span class="author">Isaac Asimov</span>
                    <img class="cover" src="/covers/foundation.jpg">
                    <span class="kind">Sci-Fi</span>
                    <span class="latest">Chapter 3</span>
                </article>
            </div>
        "#;

        let books = pipeline
            .run_legado_search_rule_with_context(html, &rule, &mut context)
            .unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(books[0].book_id, "https://books.example.test/book/1");
        assert_eq!(books[0].cover_url.as_deref(), Some("/covers/dune.jpg"));
        assert_eq!(books[0].kind.as_deref(), Some("Sci-Fi"));
        assert_eq!(books[0].last_chapter.as_deref(), Some("Chapter 7"));
        assert_eq!(books[1].title, "Foundation");
        assert_eq!(books[1].author, "Isaac Asimov");
        assert_eq!(books[1].book_id, "https://books.example.test/book/2");
        assert_eq!(
            books[1].cover_url.as_deref(),
            Some("/covers/foundation.jpg")
        );
        assert_eq!(books[1].last_chapter.as_deref(), Some("Chapter 3"));
    }

    #[test]
    fn detail_merges_metadata_from_json_object() {
        let mut source = sample_source();
        source.rules.detail = serde_json::json!([{ "kind": "cssText", "selector": "meta.detail" }]);
        let pipeline = RemoteContentPipeline::new();
        // The cssText selector will not produce JSON, so the fallback path runs.
        // We feed a response whose chain yields a JSON object string via cssText
        // is hard; instead test the JSON-object branch directly via jsonPath.
        source.rules.detail = serde_json::json!([{ "kind": "jsonPath", "path": "$.book" }]);
        let base = Book {
            book_id: "1".into(),
            title: "Dune".into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        };
        let resp = r#"{"book":{"author":"Herbert","intro":"A spice novel"}}"#;
        let merged = pipeline.detail(&source, &base, resp).unwrap();
        assert_eq!(merged.author, "Herbert");
        assert_eq!(merged.intro.as_deref(), Some("A spice novel"));
        assert_eq!(merged.title, "Dune");
    }

    #[test]
    fn detail_flat_key_value_fallback() {
        let mut source = sample_source();
        source.rules.detail = serde_json::json!([
            { "kind": "cssText", "selector": "span.k" },
            { "kind": "cssText", "selector": "span.v" }
        ]);
        let pipeline = RemoteContentPipeline::new();
        // Two css selectors in a chain fan out; to get flat key/value pairs we
        // instead exercise the fallback via a single cssText that yields nothing
        // useful — assert it returns the base book unchanged.
        let base = Book {
            book_id: "1".into(),
            title: "Dune".into(),
            author: "Herbert".into(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        };
        let resp = "<html></html>";
        let merged = pipeline.detail(&source, &base, resp).unwrap();
        assert_eq!(merged.title, "Dune");
    }

    #[test]
    fn detail_accepts_raw_legado_css_rule_string() {
        let mut source = sample_source();
        source.rules.detail = serde_json::json!(".detail span@text");
        let pipeline = RemoteContentPipeline::new();
        let base = Book {
            book_id: "1".into(),
            title: "Dune".into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        };
        let resp = r#"
            <section class="detail">
                <span>author</span><span>Herbert</span>
                <span>intro</span><span>A spice novel</span>
                <span>lastChapter</span><span>Arrakis</span>
            </section>
        "#;

        let merged = pipeline.detail(&source, &base, resp).unwrap();

        assert_eq!(merged.author, "Herbert");
        assert_eq!(merged.intro.as_deref(), Some("A spice novel"));
        assert_eq!(merged.last_chapter.as_deref(), Some("Arrakis"));
    }

    #[test]
    fn detail_uses_structured_legado_book_source_book_info_rule() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legado Compat",
            "bookSourceUrl": "https://books.example.test",
            "bookInfoRule": {
                "init": "@put:{a:\"span.author@text\",k:\"span.kind@text\"}",
                "name": "h1.name@text",
                "author": "@get:{a}",
                "intro": "p.intro@text",
                "kind": "@get:{k}",
                "coverUrl": "img.cover@src",
                "lastChapter": "a.latest@text"
            }
        });
        let pipeline = RemoteContentPipeline::new();
        let base = Book {
            book_id: "/book/1".into(),
            title: "Old title".into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        };
        let resp = r#"
            <article class="detail">
                <h1 class="name">Dune</h1>
                <span class="author">Frank Herbert</span>
                <p class="intro">A spice novel</p>
                <span class="kind">Sci-Fi</span>
                <img class="cover" src="/covers/dune.jpg">
                <a class="latest" href="/c/7">Chapter 7</a>
            </article>
        "#;

        let merged = pipeline.detail(&source, &base, resp).unwrap();

        assert_eq!(merged.book_id, "/book/1");
        assert_eq!(merged.title, "Dune");
        assert_eq!(merged.author, "Frank Herbert");
        assert_eq!(merged.intro.as_deref(), Some("A spice novel"));
        assert_eq!(merged.kind.as_deref(), Some("Sci-Fi"));
        assert_eq!(merged.cover_url.as_deref(), Some("/covers/dune.jpg"));
        assert_eq!(merged.last_chapter.as_deref(), Some("Chapter 7"));
    }

    #[test]
    fn legado_field_rules_share_context_across_content_pipeline_calls() {
        let pipeline = RemoteContentPipeline::new();
        let mut context = reader_rule::LegadoRuleContext::new();

        let book_json = r#"{
            "id": "book-42",
            "name": "Dune",
            "author": "Frank Herbert"
        }"#;

        let detail = pipeline
            .run_legado_field_rules_with_context(
                book_json,
                [
                    ("name", "$.name@put:{id:id,name:name,author:author}"),
                    ("author", "@get:{author}"),
                    (
                        "tocUrl",
                        "https://api.example.test/books/@get:{id}/toc##$##?webView=true",
                    ),
                ],
                &mut context,
            )
            .unwrap();

        assert_eq!(detail[0].0, "name");
        assert_eq!(detail[0].1.values(), &["Dune".to_string()]);
        assert_eq!(detail[1].0, "author");
        assert_eq!(detail[1].1.values(), &["Frank Herbert".to_string()]);
        assert_eq!(detail[2].0, "tocUrl");
        assert_eq!(
            detail[2].1.values(),
            &["https://api.example.test/books/book-42/toc?webView=true".to_string()]
        );

        let chapter_json = r#"{ "chapter_id": "chapter-7" }"#;
        let chapter = pipeline
            .run_legado_field_rules_with_context(
                chapter_json,
                [(
                    "chapterUrl",
                    "https://api.example.test/books/@get:{id}/chapters/{{$.chapter_id}}",
                )],
                &mut context,
            )
            .unwrap();

        assert_eq!(chapter[0].0, "chapterUrl");
        assert_eq!(
            chapter[0].1.values(),
            &["https://api.example.test/books/book-42/chapters/chapter-7".to_string()]
        );
    }

    #[test]
    fn legado_book_info_rule_init_populates_context_before_fields() {
        let pipeline = RemoteContentPipeline::new();
        let mut context = reader_rule::LegadoRuleContext::new();
        let rule = BookInfoRule {
            r#init: Some(r#"@put:{n:"h1.name@text",a:"span.author@text",t:"a.toc@href"}"#.into()),
            name: Some("@get:{n}".into()),
            author: Some("@get:{a}".into()),
            toc_url: Some("https://api.example.test@get:{t}".into()),
            ..BookInfoRule::default()
        };
        let html = r#"
            <article>
                <h1 class="name">Dune</h1>
                <span class="author">Frank Herbert</span>
                <a class="toc" href="/books/book-42/toc">toc</a>
            </article>
        "#;

        let fields = pipeline
            .run_legado_book_info_rule_with_context(html, &rule, &mut context)
            .unwrap();

        let values: HashMap<_, _> = fields
            .iter()
            .map(|(field, output)| (field.as_str(), output.first().unwrap_or("")))
            .collect();
        assert_eq!(values.get("name").copied(), Some("Dune"));
        assert_eq!(values.get("author").copied(), Some("Frank Herbert"));
        assert_eq!(
            values.get("tocUrl").copied(),
            Some("https://api.example.test/books/book-42/toc")
        );
        assert!(!values.contains_key("init"));
        assert_eq!(context.get_variable("n"), Some("Dune"));
        assert_eq!(context.get_variable("a"), Some("Frank Herbert"));
        assert_eq!(context.get_variable("t"), Some("/books/book-42/toc"));
    }

    #[test]
    fn legado_content_rule_fields_share_context_for_next_content_url() {
        let pipeline = RemoteContentPipeline::new();
        let mut context = reader_rule::LegadoRuleContext::new();
        context.put_variable("bookId", "book-42");
        let rule = ContentRule {
            content: Some(r#"article.chapter@html@put:{next:"a.next@href"}"#.into()),
            title: Some("h1.title@text".into()),
            next_content_url: Some(
                "https://api.example.test/books/@get:{bookId}/chapters@get:{next}".into(),
            ),
            ..ContentRule::default()
        };
        let html = r#"
            <main>
                <h1 class="title">Chapter 7</h1>
                <article class="chapter">
                    <p>First paragraph.</p>
                    <p>Second paragraph.</p>
                </article>
                <a class="next" href="/chapter-8">next</a>
            </main>
        "#;

        let fields = pipeline
            .run_legado_content_rule_with_context(html, &rule, &mut context)
            .unwrap();

        let values: HashMap<_, _> = fields
            .iter()
            .map(|(field, output)| (field.as_str(), output.first().unwrap_or("")))
            .collect();
        assert_eq!(values.get("title").copied(), Some("Chapter 7"));
        assert_eq!(
            values.get("content").copied(),
            Some("First paragraph.\nSecond paragraph.")
        );
        assert_eq!(
            values.get("nextContentUrl").copied(),
            Some("https://api.example.test/books/book-42/chapters/chapter-8")
        );
        assert_eq!(context.get_variable("next"), Some("/chapter-8"));
    }

    #[test]
    fn legado_toc_rule_runs_fields_per_chapter_list_item() {
        let pipeline = RemoteContentPipeline::new();
        let mut context = reader_rule::LegadoRuleContext::new();
        context.put_variable("bookId", "book-42");
        let rule = TocRule {
            chapter_list: Some("ul.toc&&li.chapter".into()),
            chapter_name: Some(r#"a@text@put:{href:"a@href"}"#.into()),
            chapter_url: Some("https://api.example.test/books/@get:{bookId}@get:{href}".into()),
            ..TocRule::default()
        };
        let html = r#"
            <ul class="toc">
                <li class="chapter"><a href="/c/1">Chapter 1</a></li>
                <li class="chapter"><a href="/c/2">Chapter 2</a></li>
            </ul>
        "#;

        let toc = pipeline
            .run_legado_toc_rule_with_context(html, &rule, &mut context)
            .unwrap();

        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].index, 0);
        assert_eq!(toc[0].title, "Chapter 1");
        assert_eq!(toc[0].url, "https://api.example.test/books/book-42/c/1");
        assert_eq!(toc[1].index, 1);
        assert_eq!(toc[1].title, "Chapter 2");
        assert_eq!(toc[1].url, "https://api.example.test/books/book-42/c/2");
    }

    #[test]
    fn toc_extracts_from_json_array() {
        let mut source = sample_source();
        source.rules.toc = serde_json::json!([{ "kind": "jsonPath", "path": "$.chapters" }]);
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"{"chapters":[
            {"title":"Ch 1","url":"/c/1"},
            {"title":"Ch 2","url":"/c/2"}
        ]}"#;
        let toc = pipeline.toc(&source, resp).unwrap();
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].title, "Ch 1");
        assert_eq!(toc[0].url, "/c/1");
        assert_eq!(toc[1].index, 1);
    }

    #[test]
    fn toc_accepts_raw_legado_css_rule_string() {
        let mut source = sample_source();
        source.rules.toc = serde_json::json!("div.toc@text");
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <html>
                <div class="toc">
                    [
                        {"title":"Ch 1","url":"/c/1"},
                        {"title":"Ch 2","url":"/c/2"}
                    ]
                </div>
            </html>
        "#;

        let toc = pipeline.toc(&source, resp).unwrap();

        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].title, "Ch 1");
        assert_eq!(toc[0].url, "/c/1");
        assert_eq!(toc[1].index, 1);
    }

    #[test]
    fn toc_uses_structured_legado_book_source_toc_rule() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legado Compat",
            "bookSourceUrl": "https://books.example.test",
            "tocRule": {
                "chapterList": "ol.toc&&li.chapter",
                "chapterName": "a@text@put:{href:\"a@href\"}",
                "chapterUrl": "@get:{sourceHost}@get:{href}"
            }
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <ol class="toc">
                <li class="chapter"><a href="/book/1/c/1">Chapter 1</a></li>
                <li class="chapter"><a href="/book/1/c/2">Chapter 2</a></li>
            </ol>
        "#;

        let toc = pipeline.toc(&source, resp).unwrap();

        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].index, 0);
        assert_eq!(toc[0].title, "Chapter 1");
        assert_eq!(toc[0].url, "https://books.example.test/book/1/c/1");
        assert_eq!(toc[1].index, 1);
        assert_eq!(toc[1].title, "Chapter 2");
        assert_eq!(toc[1].url, "https://books.example.test/book/1/c/2");
    }

    #[test]
    fn chapter_content_extracts_text() {
        let mut source = sample_source();
        source.rules.chapter = serde_json::json!([{ "kind": "cssText", "selector": "p.body" }]);
        let pipeline = RemoteContentPipeline::new();
        let resp = "<html><body><p class=\"body\">Para one.</p><p class=\"body\">Para two.</p></body></html>";
        let content = pipeline.chapter_content(&source, resp).unwrap();
        assert_eq!(content, "Para one.\nPara two.");
    }

    #[test]
    fn chapter_content_accepts_raw_legado_css_rule_string() {
        let mut source = sample_source();
        source.rules.chapter = serde_json::json!("article.chapter@html");
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <html>
                <article class="chapter">
                    <p>First&nbsp;&amp; <em>bold</em> line.</p>
                    <p>Second<br/>line.</p>
                </article>
            </html>
        "#;

        let content = pipeline.chapter_content(&source, resp).unwrap();

        assert_eq!(content, "First & bold line.\nSecond\nline.");
    }

    #[test]
    fn chapter_content_uses_structured_legado_book_source_content_rule() {
        let mut source = sample_source();
        source.book_source = serde_json::json!({
            "bookSourceName": "Legado Compat",
            "bookSourceUrl": "https://books.example.test",
            "contentRule": {
                "title": "h1.title@text",
                "content": "article.chapter@html"
            }
        });
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <main>
                <h1 class="title">Chapter 7</h1>
                <article class="chapter">
                    <p>First&nbsp;&amp; <em>bold</em> line.</p>
                    <p>Second<br/>line.</p>
                </article>
            </main>
        "#;

        let content = pipeline.chapter_content(&source, resp).unwrap();

        assert_eq!(content, "First & bold line.\nSecond\nline.");
    }

    #[test]
    fn chapter_content_formats_raw_html_fragment_like_legado() {
        let mut source = sample_source();
        source.rules.chapter = serde_json::json!([
            {
                "kind": "regexExtract",
                "pattern": "(?s)<article id=\"chapter\">(.*?)</article>",
                "group": { "index": 1 }
            }
        ]);
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <html>
                <article id="chapter">
                    <p>First&nbsp;&amp; <em>bold</em> line.</p>
                    <p>Second<br/>line.</p>
                    <!-- remove me -->
                </article>
            </html>
        "#;

        let content = pipeline.chapter_content(&source, resp).unwrap();

        assert_eq!(content, "First & bold line.\nSecond\nline.");
    }

    fn sample_book() -> Book {
        Book {
            book_id: "book-1".into(),
            title: "Dune".into(),
            author: "Herbert".into(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        }
    }

    fn toc_entry(index: u32, title: &str, url: &str) -> TocEntry {
        TocEntry {
            index,
            title: title.into(),
            url: url.into(),
        }
    }

    fn content_document(source_id: &str, book_id: &str, updated_at: i64) -> ContentDocument {
        let mut source = sample_source();
        source.source_id = source_id.into();
        let mut book = sample_book();
        book.book_id = book_id.into();
        book.title = format!("Book {book_id}");
        let toc = vec![toc_entry(0, "A", "/a"), toc_entry(1, "B", "/b")];
        let chapters = vec![
            normalize_chapter(&source, &book, &toc[0], "Alpha body").unwrap(),
            normalize_chapter(&source, &book, &toc[1], "Beta body").unwrap(),
        ];
        ContentDocument::new(source.source_id, book, toc, chapters, updated_at).unwrap()
    }

    #[test]
    fn content_document_library_upserts_lists_and_reads_chapters() {
        let mut library = ContentDocumentLibrary::new();
        library
            .upsert_document(content_document("src2", "book-1", 2000))
            .unwrap();
        library
            .upsert_document(content_document("src1", "book-1", 1000))
            .unwrap();

        let keys = library
            .list_documents()
            .into_iter()
            .map(|document| (document.source_id, document.book.book_id))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                ("src1".to_string(), "book-1".to_string()),
                ("src2".to_string(), "book-1".to_string())
            ]
        );

        let chapter = library.get_chapter("src1", "book-1", 1).unwrap().unwrap();
        assert_eq!(chapter.title, "B");
        assert_eq!(chapter.content, "Beta body");
        assert!(library.get_chapter("src1", "book-1", 99).unwrap().is_none());
        assert!(library.get_document("src2", "book-1").unwrap().is_some());
        assert!(library.remove_document("src2", "book-1").unwrap());
        assert!(!library.remove_document("src2", "book-1").unwrap());
    }

    #[test]
    fn content_document_upsert_replaces_same_source_book() {
        let mut library = ContentDocumentLibrary::new();
        library
            .upsert_document(content_document("src1", "book-1", 1000))
            .unwrap();
        library
            .upsert_document(content_document("src1", "book-1", 2000))
            .unwrap();

        assert_eq!(library.list_documents().len(), 1);
        assert_eq!(
            library
                .get_document("src1", "book-1")
                .unwrap()
                .unwrap()
                .updated_at,
            2000
        );
        assert!(matches!(
            library.get_document("", "book-1"),
            Err(ContentError::InvalidDocument { .. })
        ));
    }

    #[test]
    fn content_library_snapshot_export_is_stable_and_json_round_trips() {
        let mut library = ContentDocumentLibrary::new();
        library
            .upsert_document(content_document("src2", "book-2", 2000))
            .unwrap();
        library
            .upsert_document(content_document("src1", "book-1", 1000))
            .unwrap();

        let snapshot = library.export_snapshot(42).unwrap();

        assert_eq!(
            snapshot.schema_version,
            CONTENT_LIBRARY_SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(snapshot.exported_at, 42);
        assert_eq!(
            snapshot
                .documents
                .iter()
                .map(|document| (document.source_id.as_str(), document.book.book_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("src1", "book-1"), ("src2", "book-2")]
        );
        assert_eq!(snapshot.documents[0].toc.len(), 2);
        assert_eq!(
            snapshot.documents[0].chapters[0]
                .cache_key
                .split(':')
                .count(),
            4
        );

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains(r#""schemaVersion":1"#));
        let back: ContentLibrarySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn content_library_snapshot_replace_round_trips_and_empty_clears() {
        let mut source = ContentDocumentLibrary::new();
        source
            .upsert_document(content_document("src1", "book-1", 1000))
            .unwrap();
        let snapshot = source.export_snapshot(77).unwrap();

        let mut restored = ContentDocumentLibrary::new();
        restored.replace_with_snapshot(snapshot.clone()).unwrap();

        assert_eq!(restored.export_snapshot(77).unwrap(), snapshot);
        assert_eq!(
            restored
                .get_chapter("src1", "book-1", 0)
                .unwrap()
                .unwrap()
                .content,
            "Alpha body"
        );

        restored
            .replace_with_snapshot(ContentLibrarySnapshot::empty(100))
            .unwrap();
        assert!(restored.list_documents().is_empty());
        assert!(restored.get_document("src1", "book-1").unwrap().is_none());
    }

    #[test]
    fn content_library_snapshot_rejects_schema_duplicates_invalid_documents_and_unknown_fields() {
        let mut wrong_schema = ContentLibrarySnapshot::empty(1);
        wrong_schema.schema_version = 2;
        assert_eq!(
            wrong_schema.validate().unwrap_err(),
            ContentError::InvalidDocument {
                field: "schema_version".into()
            }
        );

        let mut duplicate = ContentLibrarySnapshot::empty(1);
        duplicate
            .documents
            .push(content_document("src1", "book-1", 1000));
        duplicate
            .documents
            .push(content_document("src1", "book-1", 2000));
        assert_eq!(
            duplicate.validate().unwrap_err(),
            ContentError::InvalidDocument {
                field: "documents".into()
            }
        );

        let mut invalid = ContentLibrarySnapshot::empty(1);
        let mut broken = content_document("src1", "book-1", 1000);
        broken.chapters[0].content_fingerprint = "wrong".into();
        invalid.documents.push(broken);
        assert_eq!(
            invalid.validate().unwrap_err(),
            ContentError::InvalidDocument {
                field: "chapters.content_fingerprint".into()
            }
        );

        let unknown = r#"{"schemaVersion":1,"exportedAt":1,"documents":[],"bogus":true}"#;
        assert!(serde_json::from_str::<ContentLibrarySnapshot>(unknown).is_err());
    }

    #[test]
    fn content_library_snapshot_replace_is_atomic_on_validation_failure() {
        let mut library = ContentDocumentLibrary::new();
        library
            .upsert_document(content_document("src1", "book-1", 1000))
            .unwrap();
        let before = library.export_snapshot(1).unwrap();

        let mut invalid = ContentLibrarySnapshot::empty(2);
        let mut broken = content_document("src2", "book-2", 2000);
        broken.toc.pop();
        invalid.documents.push(broken);

        assert!(matches!(
            library.replace_with_snapshot(invalid),
            Err(ContentError::InvalidDocument { .. })
        ));
        assert_eq!(library.export_snapshot(1).unwrap(), before);
    }

    #[test]
    fn normalize_chapter_trims_bom_line_endings_and_blank_runs() {
        let source = sample_source();
        let book = sample_book();
        let toc = toc_entry(2, "Chapter 3", "/c/3");

        let chapter = normalize_chapter(
            &source,
            &book,
            &toc,
            "\u{feff}\r\nPara one.  \r\n\r\n\r\nPara two.\r\n",
        )
        .unwrap();

        assert_eq!(chapter.source_id, "src1");
        assert_eq!(chapter.book_id, "book-1");
        assert_eq!(chapter.chapter_index, 2);
        assert_eq!(chapter.content, "Para one.\n\nPara two.");
        assert_eq!(chapter.paragraphs, vec!["Para one.", "Para two."]);
        assert_eq!(chapter.char_len, "Para one.\n\nPara two.".chars().count());
        assert!(chapter.cache_key.starts_with("src1:book-1:2:"));
        assert_eq!(chapter.content_fingerprint.len(), 16);
    }

    #[test]
    fn normalize_chapter_removes_leading_duplicate_chapter_title() {
        let source = sample_source();
        let book = sample_book();
        let toc = toc_entry(2, "Chapter 3", "/c/3");

        let chapter =
            normalize_chapter(&source, &book, &toc, "Dune: Chapter 3\n\nPara one.").unwrap();

        assert_eq!(chapter.content, "Para one.");
        assert_eq!(chapter.paragraphs, vec!["Para one."]);
        assert_eq!(chapter.char_len, "Para one.".chars().count());
    }

    #[test]
    fn chapter_document_extracts_then_normalizes_for_cache() {
        let mut source = sample_source();
        source.rules.chapter = serde_json::json!([{ "kind": "cssText", "selector": "p.body" }]);
        let book = sample_book();
        let toc = toc_entry(0, "Start", "/start");
        let pipeline = RemoteContentPipeline::new();
        let resp = "<html><p class=\"body\"> First </p><p class=\"body\">Second</p></html>";

        let chapter = pipeline
            .chapter_document(&source, &book, &toc, resp)
            .unwrap();

        assert_eq!(chapter.content, "First\nSecond");
        assert_eq!(chapter.title, "Start");
        assert_eq!(chapter.url, "/start");
    }

    #[test]
    fn normalize_chapter_rejects_missing_keys_and_empty_content() {
        let source = sample_source();
        let book = sample_book();
        let toc = toc_entry(0, "Start", "/start");

        let err = normalize_chapter(&source, &book, &toc, " \n\t ").unwrap_err();
        assert_eq!(
            err,
            ContentError::InvalidChapter {
                field: "content".into()
            }
        );

        let mut bad_source = source.clone();
        bad_source.source_id.clear();
        let err = normalize_chapter(&bad_source, &book, &toc, "body").unwrap_err();
        assert_eq!(
            err,
            ContentError::InvalidChapter {
                field: "source_id".into()
            }
        );
    }

    #[test]
    fn normalized_chapter_json_denies_unknown_fields() {
        let source = sample_source();
        let book = sample_book();
        let toc = toc_entry(0, "Start", "/start");
        let chapter = normalize_chapter(&source, &book, &toc, "body").unwrap();
        let json = serde_json::to_string(&chapter).unwrap();
        let back: NormalizedChapter = serde_json::from_str(&json).unwrap();
        assert_eq!(back, chapter);

        let err_json = r#"{"sourceId":"s","bookId":"b","chapterIndex":0,"content":"x","paragraphs":["x"],"charLen":1,"contentFingerprint":"abc","cacheKey":"k","bogus":true}"#;
        assert!(serde_json::from_str::<NormalizedChapter>(err_json).is_err());
    }

    #[test]
    fn toc_diff_maps_url_changes_by_title_fallback() {
        let old = vec![
            toc_entry(0, "Chapter 1", "/old/1"),
            toc_entry(1, "Chapter 2", "/old/2"),
        ];
        let new = vec![
            toc_entry(0, "Preface", "/new/preface"),
            toc_entry(1, "Chapter 1", "/new/1"),
            toc_entry(2, "Chapter 2", "/new/2"),
        ];

        let diff = diff_toc(&old, &new);

        assert_eq!(diff.inserted, vec![toc_entry(0, "Preface", "/new/preface")]);
        assert!(diff.removed.is_empty());
        assert_eq!(diff.mappings[0].new_index, Some(1));
        assert_eq!(diff.mappings[1].new_index, Some(2));
    }

    #[test]
    fn toc_diff_handles_duplicate_urls_by_occurrence() {
        let old = vec![
            toc_entry(0, "A", "/dup"),
            toc_entry(1, "B", "/dup"),
            toc_entry(2, "C", "/c"),
        ];
        let new = vec![
            toc_entry(0, "A updated", "/dup"),
            toc_entry(1, "Inserted", "/inserted"),
            toc_entry(2, "B updated", "/dup"),
            toc_entry(3, "C", "/c"),
        ];

        let diff = diff_toc(&old, &new);

        assert_eq!(diff.mappings[0].new_index, Some(0));
        assert_eq!(diff.mappings[1].new_index, Some(2));
        assert_eq!(diff.mappings[2].new_index, Some(3));
        assert_eq!(diff.inserted, vec![toc_entry(1, "Inserted", "/inserted")]);
    }

    #[test]
    fn remap_reading_progress_preserves_offset_when_chapter_survives() {
        let old = vec![toc_entry(0, "A", "/a"), toc_entry(1, "B", "/b")];
        let new = vec![
            toc_entry(0, "New", "/new"),
            toc_entry(1, "A", "/a"),
            toc_entry(2, "B", "/b"),
        ];
        let diff = diff_toc(&old, &new);
        let progress = ReadingProgress {
            book_id: "book-1".into(),
            chapter_index: 1,
            chapter_offset: 256,
            chapter_progress: 0.4,
        };

        let remapped = remap_reading_progress(&progress, &diff);

        assert_eq!(remapped.status, ProgressRemapStatus::Remapped);
        assert_eq!(remapped.progress.chapter_index, 2);
        assert_eq!(remapped.progress.chapter_offset, 256);
        assert_eq!(remapped.progress.chapter_progress, 0.4);
    }

    #[test]
    fn remap_reading_progress_resets_offset_when_chapter_removed() {
        let old = vec![
            toc_entry(0, "A", "/a"),
            toc_entry(1, "Removed", "/removed"),
            toc_entry(2, "C", "/c"),
        ];
        let new = vec![toc_entry(0, "A", "/a"), toc_entry(1, "C", "/c")];
        let diff = diff_toc(&old, &new);
        let progress = ReadingProgress {
            book_id: "book-1".into(),
            chapter_index: 1,
            chapter_offset: 999,
            chapter_progress: 0.9,
        };

        let remapped = remap_reading_progress(&progress, &diff);

        assert_eq!(remapped.status, ProgressRemapStatus::ChapterRemovedClamped);
        assert_eq!(remapped.progress.chapter_index, 1);
        assert_eq!(remapped.progress.chapter_offset, 0);
        assert_eq!(remapped.progress.chapter_progress, 0.0);
    }

    #[test]
    fn remap_reading_progress_handles_empty_new_toc() {
        let diff = diff_toc(&[toc_entry(0, "A", "/a")], &[]);
        let progress = ReadingProgress {
            book_id: "book-1".into(),
            chapter_index: 0,
            chapter_offset: 42,
            chapter_progress: 0.2,
        };

        let remapped = remap_reading_progress(&progress, &diff);

        assert_eq!(remapped.status, ProgressRemapStatus::EmptyToc);
        assert_eq!(remapped.progress.chapter_index, 0);
        assert_eq!(remapped.progress.chapter_offset, 0);
        assert_eq!(remapped.progress.chapter_progress, 0.0);
    }

    #[test]
    fn js_rule_success_path() {
        let pipeline = RemoteContentPipeline::new();
        let outcome = pipeline.evaluate_js_rule("({ title: 'Dune', ok: true })");
        match outcome {
            JsOutcome::Ok(v) => {
                assert_eq!(v["title"], "Dune");
                assert_eq!(v["ok"], true);
            }
            JsOutcome::Unsupported { reason } => panic!("expected ok, got unsupported: {reason}"),
        }
    }

    #[test]
    fn js_rule_host_call_without_callback_is_unsupported() {
        let pipeline = RemoteContentPipeline::new();
        let outcome = pipeline.evaluate_js_rule(r#"java.get("https://example.test")"#);
        match outcome {
            JsOutcome::Unsupported { .. } => {}
            other => panic!("expected unsupported, got {other:?}"),
        }
    }

    #[test]
    fn js_rule_host_call_with_registered_callback_succeeds() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", |_call| {
            Ok(serde_json::json!({ "status": "stubbed", "body": "<html></html>" }))
        });
        let js = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let pipeline = RemoteContentPipeline::with_js_sandbox(js);
        let outcome = pipeline.evaluate_js_rule(r#"java.get("https://example.test")"#);
        match outcome {
            JsOutcome::Ok(v) => assert_eq!(v["status"], "stubbed"),
            other => panic!("expected ok, got {other:?}"),
        }
    }
}

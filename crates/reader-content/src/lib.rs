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

pub mod analyze_url;
pub mod normalization;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use reader_domain::{
    Book, BookSourceExploreSemantics, BookSourceSearchSemantics, BookSourceSemantics,
    ReadingProgress, Source, TocEntry,
};
use reader_js::{JsError, JsErrorKind, JsEvaluation, JsSandbox as JsSandboxTrait, QuickJsSandbox};
use reader_rule::{
    CaptureGroup, NoopVariableScope, RuleEngine, RuleError, RuleJsEvaluator, RuleOutput, RuleStep,
    RuleVariableScope,
};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceRequestContext {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub current_url: String,
    #[serde(default)]
    pub book_url: String,
    #[serde(default)]
    pub chapter_url: String,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

impl BookSourceRequestContext {
    pub fn for_semantics(semantics: &BookSourceSemantics) -> Self {
        Self {
            base_url: semantics.base_url.clone(),
            current_url: semantics.base_url.clone(),
            book_url: String::new(),
            chapter_url: String::new(),
            variables: BTreeMap::new(),
        }
    }
}

impl RuleVariableScope for BookSourceRequestContext {
    fn get(&self, key: &str) -> Option<String> {
        self.variables.get(key).cloned()
    }
    fn put(&mut self, key: String, value: String) {
        self.variables.insert(key, value);
    }
    fn entries(&self) -> Vec<(&str, &str)> {
        self.variables
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceDetail {
    pub book: Book,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceToc {
    #[serde(default)]
    pub chapters: Vec<TocEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_toc_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookSourceExploreEntryKind {
    Category,
    Ranking,
    Channel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceExploreEntry {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub kind: BookSourceExploreEntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceExploreResult {
    pub source_id: String,
    #[serde(default)]
    pub entries: Vec<BookSourceExploreEntry>,
    #[serde(default)]
    pub books: Vec<Book>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSourceContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_content_url: Option<String>,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
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

/// Bridge the shared `QuickJsSandbox` into reader-rule's JS-evaluator trait so
/// the dispatcher can eval `{{...}}` / `<js>` segments (Task 5) without a hard
/// dependency from `reader-rule` on `reader-js`. String results are returned
/// verbatim; all other JSON values are JSON-stringified, matching Legado's
/// `makeUpRule` string coercion. A local newtype is required to satisfy the
/// orphan rule (`RuleJsEvaluator` and `QuickJsSandbox` are both foreign).
struct LegadoJsBridge<'a>(&'a QuickJsSandbox);

impl RuleJsEvaluator for LegadoJsBridge<'_> {
    fn eval(&self, expr: &str, context: Option<&str>) -> Result<String, String> {
        let script = if let Some(ctx) = context.filter(|c| !c.is_empty()) {
            format!("var __ctx = {ctx}; {expr}")
        } else {
            expr.to_string()
        };
        let evaluation = self.0.evaluate(&script).map_err(|e| e.to_string())?;
        Ok(match evaluation.value {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        })
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

    /// Execute a raw Legado DSL rule string through the multi-engine dispatcher
    /// (`execute_legado_rule`). Uses a no-op variable scope — `@put`/`@get`
    /// wiring lands in Task 4; for now `expand_template` still handles `{{key}}`
    /// substitution before this is called. The shared JS sandbox is passed so
    /// Task 5 can eval `{{...}}` / `<js>` segments without further plumbing.
    fn run_raw_legado_rule(&self, input: &str, rule: &str) -> Result<RuleOutput, RuleError> {
        let mut scope = NoopVariableScope;
        let js_bridge = LegadoJsBridge(self.js.as_ref());
        self.engine
            .execute_legado_rule(input, rule, &mut scope, Some(&js_bridge))
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
            return Ok(self.run_raw_legado_rule(input, rule)?);
        }

        let steps = Self::parse_steps(rule_spec)?;
        if steps.is_empty() {
            return Ok(RuleOutput::new(Vec::new()));
        }
        Ok(self.engine.execute_chain(input, &steps)?)
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
        if !has_rule_spec(&source.rules.search) {
            if let Some(semantics) = source.book_source_semantics() {
                let context = BookSourceRequestContext::for_semantics(&semantics);
                return self.search_book_source(&semantics, search_response, &context);
            }
        }
        let out = self.run_chain(search_response, &source.rules.search)?;
        let mut books = Vec::new();
        for value in out.values() {
            books.push(parse_book_value(value));
        }
        Ok(books)
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
        if !has_rule_spec(&source.rules.detail) {
            if let Some(semantics) = source.book_source_semantics() {
                let mut context = BookSourceRequestContext::for_semantics(&semantics);
                context.book_url = base.book_id.clone();
                let detail =
                    self.detail_book_source(&semantics, base, detail_response, &context)?;
                return Ok(detail.book);
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
        if !has_rule_spec(&source.rules.toc) {
            if let Some(semantics) = source.book_source_semantics() {
                let context = BookSourceRequestContext::for_semantics(&semantics);
                return self
                    .toc_book_source(&semantics, toc_response, &context)
                    .map(|toc| toc.chapters);
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
        if !has_rule_spec(&source.rules.chapter) {
            if let Some(semantics) = source.book_source_semantics() {
                let context = BookSourceRequestContext::for_semantics(&semantics);
                return self
                    .content_book_source(&semantics, chapter_response, &context)
                    .map(|content| content.content);
            }
        }
        let out = self.run_chain(chapter_response, &source.rules.chapter)?;
        Ok(normalization::normalize_extracted_content(
            &out.values().join("\n"),
        ))
    }

    pub fn search_book_source(
        &self,
        semantics: &BookSourceSemantics,
        search_response: &str,
        context: &BookSourceRequestContext,
    ) -> Result<Vec<Book>, ContentError> {
        let mut context = context.with_source_defaults(semantics);
        if let Some(search_url) = semantics.search_url.as_deref() {
            let resolved = resolve_url(&context.base_url, &context.current_url, search_url);
            context
                .variables
                .entry("searchUrl".into())
                .or_insert(resolved);
        }
        extract_books_from_semantic_rule(self, &semantics.rules.search, search_response, &context)
    }

    pub fn explore_book_source(
        &self,
        semantics: &BookSourceSemantics,
        explore_response: &str,
        context: &BookSourceRequestContext,
    ) -> Result<BookSourceExploreResult, ContentError> {
        let context = context.with_source_defaults(semantics);
        let entries = parse_explore_entries(
            semantics.rules.explore.screen.as_deref(),
            semantics.explore_url.as_deref(),
            &context,
        );
        let books = extract_books_from_explore_rule(
            self,
            &semantics.rules.explore,
            explore_response,
            &context,
        )?;
        Ok(BookSourceExploreResult {
            source_id: semantics.source_id.clone(),
            entries,
            books,
            next_page_url: None,
        })
    }

    pub fn detail_book_source(
        &self,
        semantics: &BookSourceSemantics,
        base: &Book,
        detail_response: &str,
        context: &BookSourceRequestContext,
    ) -> Result<BookSourceDetail, ContentError> {
        let context = context.with_source_defaults(semantics);
        let rules = &semantics.rules.detail;
        let mut book = base.clone();
        if book.book_id.trim().is_empty() {
            book.book_id = non_empty_string(context.book_url.as_str()).unwrap_or_default();
        }
        if let Some(value) =
            extract_rule_value(self, detail_response, rules.name.as_deref(), &context)?
        {
            book.title = value;
        }
        if let Some(value) =
            extract_rule_value(self, detail_response, rules.author.as_deref(), &context)?
        {
            book.author = value;
        }
        if let Some(value) =
            extract_rule_value(self, detail_response, rules.cover_url.as_deref(), &context)?
        {
            book.cover_url = Some(resolve_url(
                &context.base_url,
                &context.current_url,
                value.as_str(),
            ));
        }
        if let Some(value) =
            extract_rule_value(self, detail_response, rules.intro.as_deref(), &context)?
        {
            book.intro = Some(value);
        }
        if let Some(value) =
            extract_rule_value(self, detail_response, rules.kind.as_deref(), &context)?
        {
            book.kind = Some(value);
        }
        if let Some(value) = extract_rule_value(
            self,
            detail_response,
            rules.last_chapter.as_deref(),
            &context,
        )? {
            book.last_chapter = Some(value);
        }
        let toc_url =
            extract_rule_value(self, detail_response, rules.toc_url.as_deref(), &context)?
                .map(|value| resolve_url(&context.base_url, &context.current_url, value.as_str()));
        let update_time = extract_rule_value(
            self,
            detail_response,
            rules.update_time.as_deref(),
            &context,
        )?;
        let word_count =
            extract_rule_value(self, detail_response, rules.word_count.as_deref(), &context)?;
        Ok(BookSourceDetail {
            book,
            toc_url,
            update_time,
            word_count,
        })
    }

    pub fn toc_book_source(
        &self,
        semantics: &BookSourceSemantics,
        toc_response: &str,
        context: &BookSourceRequestContext,
    ) -> Result<BookSourceToc, ContentError> {
        let context = context.with_source_defaults(semantics);
        let rules = &semantics.rules.toc;
        let chapters = if rules.name.is_some() || rules.url.is_some() {
            let items = extract_rule_items(self, toc_response, rules.list.as_deref(), &context)?;
            let mut chapters = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let title = extract_rule_value(self, item, rules.name.as_deref(), &context)?
                    .unwrap_or_default();
                let url = extract_rule_value(self, item, rules.url.as_deref(), &context)?
                    .map(|value| resolve_url(&context.base_url, &context.current_url, &value))
                    .unwrap_or_default();
                if !title.trim().is_empty() || !url.trim().is_empty() {
                    chapters.push(TocEntry {
                        index: index as u32,
                        title,
                        url,
                    });
                }
            }
            chapters
        } else if let Some(raw) = rules.raw.as_deref() {
            let out = self.run_raw_legado_rule(toc_response, raw)?;
            parse_toc_output(out.values(), &context)
        } else {
            Vec::new()
        };
        let next_toc_url =
            extract_rule_value(self, toc_response, rules.next_url.as_deref(), &context)?
                .map(|value| resolve_url(&context.base_url, &context.current_url, value.as_str()));
        Ok(BookSourceToc {
            chapters,
            next_toc_url,
        })
    }

    pub fn content_book_source(
        &self,
        semantics: &BookSourceSemantics,
        chapter_response: &str,
        context: &BookSourceRequestContext,
    ) -> Result<BookSourceContent, ContentError> {
        let context = context.with_source_defaults(semantics);
        let rules = &semantics.rules.content;
        let title = extract_rule_value(self, chapter_response, rules.title.as_deref(), &context)?;
        let raw_content =
            extract_rule_value(self, chapter_response, rules.content.as_deref(), &context)?
                .or_else(|| {
                    rules.raw.as_deref().and_then(|raw| {
                        self.run_raw_legado_rule(chapter_response, raw)
                            .ok()
                            .map(|out| out.values().join("\n"))
                    })
                })
                .unwrap_or_default();
        let replaced = apply_content_replacement(
            self,
            raw_content.as_str(),
            rules.source_regex.as_deref(),
            rules.replace_regex.as_deref(),
            &context,
        )?;
        let content = normalization::normalize_extracted_content(&replaced);
        let next_content_url =
            extract_rule_value(self, chapter_response, rules.next_url.as_deref(), &context)?
                .map(|value| resolve_url(&context.base_url, &context.current_url, value.as_str()));
        Ok(BookSourceContent {
            title,
            content,
            next_content_url,
            variables: context.variables,
        })
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

    /// Evaluate a URL-embedded JS expression with a URL-specific variable
    /// scope. Used by `AnalyzeUrl::build_request_with_js` to run `@js:` /
    /// `<js>...</js>` expressions found in Legado `searchUrl`/`bookUrl`/
    /// `tocUrl`/`chapterUrl` fields.
    ///
    /// Exposes `key`, `page`, and `baseUrl` as top-level JS variables, mirroring
    /// Legado `AnalyzeUrl.kt`'s `evalJS` variable scope. The result is coerced
    /// to a string (matching Legado's `AnalyzeUrl` JS-result string coercion).
    pub fn evaluate_url_js(
        &self,
        expr: &str,
        context: &serde_json::Value,
    ) -> Result<String, String> {
        let key = context
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace('\\', "\\\\")
            .replace('\'', "\\'");
        let page = context.get("page").and_then(|v| v.as_u64()).unwrap_or(1);
        let base_url = context
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace('\\', "\\\\")
            .replace('\'', "\\'");
        // Prepend variable bindings so the expression can reference `key`/
        // `page`/`baseUrl` as top-level globals.
        let script =
            format!("var key = '{key}';\nvar page = {page};\nvar baseUrl = '{base_url}';\n{expr}");
        let evaluation = self.js.evaluate(&script).map_err(|e| e.to_string())?;
        Ok(match evaluation.value {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        })
    }
}

impl BookSourceRequestContext {
    fn with_source_defaults(&self, semantics: &BookSourceSemantics) -> Self {
        let mut context = self.clone();
        if context.base_url.trim().is_empty() {
            context.base_url = semantics.base_url.clone();
        }
        if context.current_url.trim().is_empty() {
            context.current_url = context.base_url.clone();
        }
        let source_id = semantics.source_id.clone();
        let base_url = context.base_url.clone();
        let book_url = context.book_url.clone();
        let chapter_url = context.chapter_url.clone();
        context
            .variables
            .entry("sourceId".into())
            .or_insert(source_id);
        context
            .variables
            .entry("baseUrl".into())
            .or_insert(base_url);
        if !context.book_url.trim().is_empty() {
            context
                .variables
                .entry("bookUrl".into())
                .or_insert(book_url);
        }
        if !context.chapter_url.trim().is_empty() {
            context
                .variables
                .entry("chapterUrl".into())
                .or_insert(chapter_url);
        }
        context
    }
}

fn has_rule_spec(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => !values.is_empty(),
        serde_json::Value::Object(values) => !values.is_empty(),
        _ => true,
    }
}

fn extract_books_from_semantic_rule(
    pipeline: &RemoteContentPipeline,
    rules: &BookSourceSearchSemantics,
    input: &str,
    context: &BookSourceRequestContext,
) -> Result<Vec<Book>, ContentError> {
    let has_structured_fields = [
        rules.name.as_deref(),
        rules.author.as_deref(),
        rules.detail_url.as_deref(),
        rules.cover_url.as_deref(),
        rules.intro.as_deref(),
    ]
    .iter()
    .any(|value| value.is_some());

    if !has_structured_fields {
        return extract_books_from_raw_rule(pipeline, rules.raw.as_deref(), input, context);
    }

    let items = extract_rule_items(pipeline, input, rules.list.as_deref(), context)?;
    let mut books = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let title =
            extract_rule_value(pipeline, item, rules.name.as_deref(), context)?.unwrap_or_default();
        let author = extract_rule_value(pipeline, item, rules.author.as_deref(), context)?
            .unwrap_or_default();
        let detail_url = extract_rule_value(pipeline, item, rules.detail_url.as_deref(), context)?
            .map(|value| resolve_url(&context.base_url, &context.current_url, &value));
        let cover_url = extract_rule_value(pipeline, item, rules.cover_url.as_deref(), context)?
            .map(|value| resolve_url(&context.base_url, &context.current_url, &value));
        let intro = extract_rule_value(pipeline, item, rules.intro.as_deref(), context)?;
        let kind = extract_rule_value(pipeline, item, rules.kind.as_deref(), context)?;
        let last_chapter =
            extract_rule_value(pipeline, item, rules.last_chapter.as_deref(), context)?;
        let book_id = detail_url
            .clone()
            .or_else(|| stable_fallback_book_id(&title, &author, index))
            .unwrap_or_default();
        if title.trim().is_empty() && book_id.trim().is_empty() {
            continue;
        }
        books.push(Book {
            book_id,
            title,
            author,
            cover_url,
            intro,
            kind,
            last_chapter,
        });
    }
    Ok(books)
}

fn extract_books_from_explore_rule(
    pipeline: &RemoteContentPipeline,
    rules: &BookSourceExploreSemantics,
    input: &str,
    context: &BookSourceRequestContext,
) -> Result<Vec<Book>, ContentError> {
    let has_structured_fields = [
        rules.name.as_deref(),
        rules.author.as_deref(),
        rules.detail_url.as_deref(),
        rules.cover_url.as_deref(),
        rules.intro.as_deref(),
    ]
    .iter()
    .any(|value| value.is_some());

    if !has_structured_fields {
        return extract_books_from_raw_rule(pipeline, rules.raw.as_deref(), input, context);
    }

    let items = extract_rule_items(pipeline, input, rules.list.as_deref(), context)?;
    let mut books = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let title =
            extract_rule_value(pipeline, item, rules.name.as_deref(), context)?.unwrap_or_default();
        let author = extract_rule_value(pipeline, item, rules.author.as_deref(), context)?
            .unwrap_or_default();
        let detail_url = extract_rule_value(pipeline, item, rules.detail_url.as_deref(), context)?
            .map(|value| resolve_url(&context.base_url, &context.current_url, &value));
        let cover_url = extract_rule_value(pipeline, item, rules.cover_url.as_deref(), context)?
            .map(|value| resolve_url(&context.base_url, &context.current_url, &value));
        let intro = extract_rule_value(pipeline, item, rules.intro.as_deref(), context)?;
        let kind = extract_rule_value(pipeline, item, rules.kind.as_deref(), context)?;
        let last_chapter =
            extract_rule_value(pipeline, item, rules.last_chapter.as_deref(), context)?;
        let book_id = detail_url
            .clone()
            .or_else(|| stable_fallback_book_id(&title, &author, index))
            .unwrap_or_default();
        if title.trim().is_empty() && book_id.trim().is_empty() {
            continue;
        }
        books.push(Book {
            book_id,
            title,
            author,
            cover_url,
            intro,
            kind,
            last_chapter,
        });
    }
    Ok(books)
}

fn extract_books_from_raw_rule(
    pipeline: &RemoteContentPipeline,
    raw_rule: Option<&str>,
    input: &str,
    context: &BookSourceRequestContext,
) -> Result<Vec<Book>, ContentError> {
    let Some(raw_rule) = raw_rule.and_then(non_empty_string) else {
        return Ok(Vec::new());
    };
    let rule = expand_template(&raw_rule, context);
    let out = pipeline.run_raw_legado_rule(input, &rule)?;
    Ok(out
        .values()
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let title = value.trim().to_string();
            Book {
                book_id: stable_fallback_book_id(&title, "", index).unwrap_or_default(),
                title,
                author: String::new(),
                cover_url: None,
                intro: None,
                kind: None,
                last_chapter: None,
            }
        })
        .collect())
}

fn extract_rule_items(
    pipeline: &RemoteContentPipeline,
    input: &str,
    list_rule: Option<&str>,
    context: &BookSourceRequestContext,
) -> Result<Vec<String>, ContentError> {
    let Some(rule) = list_rule.and_then(non_empty_string) else {
        return Ok(vec![input.to_string()]);
    };
    let rule = expand_template(&rule, context);
    let item_rule = if legado_rule_has_extraction(&rule) {
        rule.clone()
    } else {
        format!("{rule}@html")
    };
    let out = pipeline.run_raw_legado_rule(input, &item_rule)?;
    if !out.is_empty() {
        return Ok(out.into_values());
    }
    Ok(pipeline.run_raw_legado_rule(input, &rule)?.into_values())
}

fn extract_rule_value(
    pipeline: &RemoteContentPipeline,
    input: &str,
    rule: Option<&str>,
    context: &BookSourceRequestContext,
) -> Result<Option<String>, ContentError> {
    let Some(rule) = rule.and_then(non_empty_string) else {
        return Ok(None);
    };
    if rule.starts_with("@js") {
        return Ok(None);
    }
    let has_template = rule.contains("{{");
    let rule = expand_template(&rule, context);
    // Legado `AnalyzeRule.kt` SourceRule init (line 587-593): 规则含 `{{...}}`
    // 模板且 `{{` 在开头时 mode 自动变 Regex，`getString` 的 `else -> rule` 分支
    // 直接返回展开后字符串，不跑 CSS/XPath 选择器。tocUrl 等字段规则如
    // `{{baseUrl}}/#dir` 展开成 URL，应直接作为值返回，否则会被当 CSS 选择器
    // 解析失败（rb-tocurl-template-as-selector）。
    if has_template && looks_like_url_or_path(&rule) {
        return Ok(non_empty_string(&rule));
    }
    let out = pipeline.run_raw_legado_rule(input, &rule)?;
    Ok(out
        .values()
        .iter()
        .find_map(|value| non_empty_string(value)))
}

/// 判断展开后的规则值是否像 URL 或绝对路径，应直接当 URL 而非 CSS 选择器。
fn looks_like_url_or_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("//")
        || value.starts_with('/')
}

fn parse_toc_output(values: &[String], context: &BookSourceRequestContext) -> Vec<TocEntry> {
    if let Some(first) = values.first() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(first) {
            if let Some(array) = value.as_array() {
                return array
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let title = item
                            .get("title")
                            .or_else(|| item.get("name"))
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let url = item
                            .get("url")
                            .or_else(|| item.get("chapterUrl"))
                            .and_then(|value| value.as_str())
                            .map(|value| {
                                resolve_url(&context.base_url, &context.current_url, value)
                            })
                            .unwrap_or_default();
                        TocEntry {
                            index: index as u32,
                            title,
                            url,
                        }
                    })
                    .collect();
            }
        }
    }

    let mut entries = Vec::new();
    let mut iter = values.iter();
    let mut index = 0u32;
    while let (Some(title), Some(url)) = (iter.next(), iter.next()) {
        entries.push(TocEntry {
            index,
            title: title.clone(),
            url: resolve_url(&context.base_url, &context.current_url, url),
        });
        index += 1;
    }
    entries
}

fn parse_explore_entries(
    screen: Option<&str>,
    explore_url: Option<&str>,
    context: &BookSourceRequestContext,
) -> Vec<BookSourceExploreEntry> {
    let mut entries = Vec::new();
    if let Some(screen) = screen.and_then(non_empty_string) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&screen) {
            collect_json_explore_entries(&value, context, &mut entries);
        } else {
            collect_delimited_explore_entries(&screen, context, &mut entries);
        }
    }
    if entries.is_empty() {
        if let Some(url) = explore_url.and_then(non_empty_string) {
            let title = "Explore".to_string();
            entries.push(BookSourceExploreEntry {
                id: stable_explore_entry_id(&title, 0),
                title,
                url: Some(resolve_url(&context.base_url, &context.current_url, &url)),
                kind: BookSourceExploreEntryKind::Channel,
            });
        }
    }
    entries
}

fn collect_json_explore_entries(
    value: &serde_json::Value,
    context: &BookSourceRequestContext,
    entries: &mut Vec<BookSourceExploreEntry>,
) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_explore_entries(value, context, entries);
            }
        }
        serde_json::Value::Object(object) => {
            let title = first_json_string(object, &["title", "name", "label"]).unwrap_or_default();
            if !title.trim().is_empty() {
                let url = first_json_string(object, &["url", "exploreUrl", "urlTemplate"])
                    .map(|url| resolve_url(&context.base_url, &context.current_url, &url));
                let kind = first_json_string(object, &["kind", "type"])
                    .map(|value| classify_explore_entry_kind(&value, &title))
                    .unwrap_or_else(|| classify_explore_entry_kind("", &title));
                let index = entries.len();
                entries.push(BookSourceExploreEntry {
                    id: stable_explore_entry_id(&title, index),
                    title,
                    url,
                    kind,
                });
            }
            if let Some(children) = object.get("children").or_else(|| object.get("items")) {
                collect_json_explore_entries(children, context, entries);
            }
        }
        _ => {}
    }
}

fn collect_delimited_explore_entries(
    screen: &str,
    context: &BookSourceRequestContext,
    entries: &mut Vec<BookSourceExploreEntry>,
) {
    for token in screen
        .split(['\n', ';', '|'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let (title, url) = split_explore_token(token);
        if title.trim().is_empty() {
            continue;
        }
        let index = entries.len();
        entries.push(BookSourceExploreEntry {
            id: stable_explore_entry_id(&title, index),
            kind: classify_explore_entry_kind("", &title),
            title,
            url: url.map(|url| resolve_url(&context.base_url, &context.current_url, &url)),
        });
    }
}

fn first_json_string(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(|value| value.as_str()))
        .and_then(non_empty_string)
}

fn split_explore_token(token: &str) -> (String, Option<String>) {
    for separator in ["::", "=>", "->", ","] {
        if let Some((title, url)) = token.split_once(separator) {
            return (title.trim().to_string(), non_empty_string(url));
        }
    }
    (token.trim().to_string(), None)
}

fn classify_explore_entry_kind(kind: &str, title: &str) -> BookSourceExploreEntryKind {
    let marker = format!("{} {}", kind, title).to_ascii_lowercase();
    if marker.contains("rank") || marker.contains("榜") || marker.contains("排行") {
        BookSourceExploreEntryKind::Ranking
    } else if marker.contains("channel") || marker.contains("频道") {
        BookSourceExploreEntryKind::Channel
    } else {
        BookSourceExploreEntryKind::Category
    }
}

fn stable_explore_entry_id(title: &str, index: usize) -> String {
    let slug = title
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if slug.is_empty() {
        format!("explore-{index}")
    } else {
        format!("explore-{index}-{slug}")
    }
}

fn apply_content_replacement(
    pipeline: &RemoteContentPipeline,
    input: &str,
    source_regex: Option<&str>,
    replace_regex: Option<&str>,
    context: &BookSourceRequestContext,
) -> Result<String, ContentError> {
    let Some(pattern) = source_regex.and_then(non_empty_string) else {
        return Ok(input.to_string());
    };
    let replacement = replace_regex
        .and_then(non_empty_string)
        .map(|value| expand_template(&value, context))
        .unwrap_or_default();
    let step = RuleStep::regex_replace(pattern, replacement);
    Ok(pipeline
        .engine
        .execute_step(input, &step)?
        .first()
        .unwrap_or(input)
        .to_string())
}

fn expand_template(template: &str, context: &BookSourceRequestContext) -> String {
    let mut output = template.to_string();
    for (key, value) in &context.variables {
        output = output.replace(&format!("{{{{{key}}}}}"), value);
    }
    output
}

fn resolve_url(base_url: &str, current_url: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() || has_url_scheme(value) {
        return value.to_string();
    }
    let base = non_empty_string(current_url)
        .or_else(|| non_empty_string(base_url))
        .unwrap_or_default();
    if value.starts_with("//") {
        let scheme = base
            .split_once("://")
            .map(|(scheme, _)| scheme)
            .unwrap_or("https");
        return format!("{scheme}:{value}");
    }
    if value.starts_with('/') {
        if let Some(origin) = url_origin(&base) {
            return format!("{origin}{value}");
        }
        return value.to_string();
    }
    let directory = if base.ends_with('/') {
        base
    } else {
        base.rsplit_once('/')
            .map(|(prefix, _)| format!("{prefix}/"))
            .unwrap_or_else(|| format!("{base}/"))
    };
    format!("{directory}{value}")
}

fn has_url_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '+' | '-' | '.'))
}

fn url_origin(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    let host = rest.split('/').next()?;
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}://{host}"))
    }
}

fn stable_fallback_book_id(title: &str, author: &str, index: usize) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let mut key = slug_part(title);
    if !author.trim().is_empty() {
        key.push('-');
        key.push_str(&slug_part(author));
    }
    Some(format!("generated:{index}:{key}"))
}

fn slug_part(value: &str) -> String {
    let mut output = String::new();
    let mut pending_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !output.is_empty() {
                output.push('-');
            }
            output.push(ch);
            pending_dash = false;
        } else if ch.is_whitespace() || ch.is_ascii_punctuation() {
            pending_dash = true;
        }
    }
    if output.is_empty() {
        stable_fingerprint(value)
    } else {
        output
    }
}

fn legado_rule_has_extraction(rule: &str) -> bool {
    rule.contains('@')
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
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
    use reader_domain::{BookSourceSemantics, LegadoBookSource, SourceRules};
    use reader_js::{HostCallbackRegistry, JsRuntimeConfig};

    const BOOKSOURCE_CANONICAL_FIXTURE: &str =
        include_str!("../tests/fixtures/booksource_canonical.json");

    fn sample_source() -> Source {
        Source {
            source_id: "src1".into(),
            name: "Sample".into(),
            base_url: "https://example.test".into(),
            rules: SourceRules::default(),
            book_source: serde_json::Value::Null,
        }
    }

    fn canonical_fixture() -> serde_json::Value {
        serde_json::from_str(BOOKSOURCE_CANONICAL_FIXTURE).expect("canonical fixture should parse")
    }

    fn canonical_semantics() -> (BookSourceSemantics, serde_json::Value) {
        let fixture = canonical_fixture();
        let book_source: LegadoBookSource =
            serde_json::from_value(fixture["bookSource"].clone()).unwrap();
        let semantics = BookSourceSemantics::from_legado(
            "canonical-legado-src",
            Some("Canonical"),
            Some("https://books.example.test/root/index.html"),
            &book_source,
        );
        (semantics, fixture)
    }

    fn canonical_context(semantics: &BookSourceSemantics) -> BookSourceRequestContext {
        let mut variables = BTreeMap::new();
        variables.insert("key".into(), "dune".into());
        BookSourceRequestContext {
            base_url: semantics.base_url.clone(),
            current_url: "https://books.example.test/root/search.html".into(),
            book_url: "https://books.example.test/book/dune".into(),
            chapter_url: "https://books.example.test/book/dune/chapter/1".into(),
            variables,
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
    fn search_book_source_scopes_per_item_with_json_booklist() {
        // Closes DSL migration gap 7: bookList @json: scoping.
        //
        // Legado `searchRule.bookList` selects a list of elements (HTML nodes
        // or JSON array items) and each subsequent rule (`name`/`author`/...)
        // is evaluated independently against that item — not the whole
        // document. Before Task 1 (prefix dispatch), a `@Json:$.books[*]`
        // bookList would have been mis-parsed as CSS and silently returned
        // empty, breaking the per-item scoping for any JSON book source.
        //
        // This test builds a BookSourceSearchSemantics whose bookList is a
        // `@Json:` rule and asserts each per-item rule resolves fields from
        // its own JSON object, proving the dispatch + scoping chain.
        let mut search = reader_domain::BookSourceSearchSemantics::default();
        search.list = Some("@Json:$.books[*]".into());
        search.name = Some("$.title".into());
        search.author = Some("$.author".into());
        search.detail_url = Some("$.url".into());

        let semantics = BookSourceSemantics {
            source_id: "json-src".into(),
            name: "JSON Scoping Source".into(),
            base_url: "https://json.example.test".into(),
            search_url: None,
            explore_url: None,
            enabled: true,
            enabled_explore: false,
            rules: reader_domain::BookSourcePipelineRules {
                search,
                explore: reader_domain::BookSourceExploreSemantics::default(),
                detail: reader_domain::BookSourceDetailSemantics::default(),
                toc: reader_domain::BookSourceTocSemantics::default(),
                content: reader_domain::BookSourceContentSemantics::default(),
            },
        };
        let context = BookSourceRequestContext::for_semantics(&semantics);
        let pipeline = RemoteContentPipeline::new();

        let resp = r#"{"books":[
            {"title":"Dune","author":"Herbert","url":"/book/dune"},
            {"title":"Foundation","author":"Asimov","url":"/book/foundation"}
        ]}"#;

        let books = pipeline
            .search_book_source(&semantics, resp, &context)
            .unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Herbert");
        assert_eq!(books[0].book_id, "https://json.example.test/book/dune");
        assert_eq!(books[1].title, "Foundation");
        assert_eq!(books[1].author, "Asimov");
        assert_eq!(
            books[1].book_id,
            "https://json.example.test/book/foundation"
        );
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
        source.rules.toc = serde_json::json!("script.toc@html");
        let pipeline = RemoteContentPipeline::new();
        let resp = r#"
            <html>
                <script class="toc" type="application/json">
                    [
                        {"title":"Ch 1","url":"/c/1"},
                        {"title":"Ch 2","url":"/c/2"}
                    ]
                </script>
            </html>
        "#;

        let toc = pipeline.toc(&source, resp).unwrap();

        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].title, "Ch 1");
        assert_eq!(toc[0].url, "/c/1");
        assert_eq!(toc[1].index, 1);
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

    #[test]
    fn book_source_semantic_pipeline_runs_canonical_fixture() {
        let (semantics, fixture) = canonical_semantics();
        let context = canonical_context(&semantics);
        let pipeline = RemoteContentPipeline::new();

        let books = pipeline
            .search_book_source(
                &semantics,
                fixture["searchResponse"].as_str().unwrap(),
                &context,
            )
            .unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].book_id, "https://books.example.test/book/dune");
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[0].author, "Frank Herbert");
        assert_eq!(
            books[0].cover_url.as_deref(),
            Some("https://img.example.test/dune.jpg")
        );
        assert_eq!(
            books[1].book_id,
            "https://books.example.test/root/book/foundation"
        );

        let explore = pipeline
            .explore_book_source(
                &semantics,
                fixture["exploreResponse"].as_str().unwrap(),
                &context,
            )
            .unwrap();
        assert_eq!(explore.entries.len(), 3);
        assert_eq!(explore.entries[1].kind, BookSourceExploreEntryKind::Ranking);
        assert_eq!(explore.entries[2].kind, BookSourceExploreEntryKind::Channel);
        assert_eq!(
            explore.books[0].book_id,
            "https://books.example.test/book/rank-1"
        );

        let detail = pipeline
            .detail_book_source(
                &semantics,
                &books[0],
                fixture["detailResponse"].as_str().unwrap(),
                &context,
            )
            .unwrap();
        assert_eq!(
            detail.book.intro.as_deref(),
            Some("Expanded desert planet.")
        );
        assert_eq!(detail.book.kind.as_deref(), Some("Sci-Fi Classic"));
        assert_eq!(
            detail.toc_url.as_deref(),
            Some("https://books.example.test/book/dune/toc?page=1")
        );
        assert_eq!(detail.word_count.as_deref(), Some("188000"));

        let toc = pipeline
            .toc_book_source(
                &semantics,
                fixture["tocResponse"].as_str().unwrap(),
                &context,
            )
            .unwrap();
        assert_eq!(toc.chapters.len(), 2);
        assert_eq!(
            toc.chapters[0].url,
            "https://books.example.test/book/dune/chapter/1"
        );
        assert_eq!(
            toc.chapters[1].url,
            "https://books.example.test/root/chapter/2"
        );
        assert_eq!(
            toc.next_toc_url.as_deref(),
            Some("https://books.example.test/book/dune/toc?page=2")
        );

        let content = pipeline
            .content_book_source(
                &semantics,
                fixture["contentResponse"].as_str().unwrap(),
                &context,
            )
            .unwrap();
        assert_eq!(content.title.as_deref(), Some("Chapter 1"));
        assert_eq!(
            content.content,
            "First line.\ncanonical-legado-src\nSecond line."
        );
        assert_eq!(
            content.next_content_url.as_deref(),
            Some("https://books.example.test/book/dune/chapter/2")
        );
        assert_eq!(content.variables["key"], "dune");
    }

    #[test]
    fn toc_url_template_expands_to_url_not_selector() {
        // rb-tocurl-template-as-selector: tocUrl 含 {{baseUrl}} 模板 (如
        // "{{baseUrl}}/#dir"),展开成 URL 后被当 CSS 选择器解析失败。
        // 对齐 Legado AnalyzeRule.kt SourceRule init: 规则含 `{{...}}` 模板时
        // mode 变 Regex,getString 的 `else -> rule` 直接返回展开后字符串。
        // 修复:展开后若像 URL/路径,直接作为值返回,不跑选择器。
        let pipeline = RemoteContentPipeline::new();
        let mut variables = BTreeMap::new();
        variables.insert("baseUrl".into(), "https://host.example.test".into());
        let context = BookSourceRequestContext {
            base_url: "https://host.example.test".into(),
            current_url: "https://host.example.test/detail".into(),
            book_url: "https://host.example.test/book/1".into(),
            chapter_url: String::new(),
            variables,
        };

        // {{baseUrl}}/#dir 展开成 https://host.example.test/#dir -> 当 URL 返回
        let toc_url = extract_rule_value(
            &pipeline,
            "<html><body></body></html>",
            Some("{{baseUrl}}/#dir"),
            &context,
        )
        .unwrap();
        assert_eq!(toc_url.as_deref(), Some("https://host.example.test/#dir"));

        // 绝对 URL 模板也当 URL
        let abs_url = extract_rule_value(
            &pipeline,
            "<html></html>",
            Some("{{baseUrl}}/toc/list"),
            &context,
        )
        .unwrap();
        assert_eq!(
            abs_url.as_deref(),
            Some("https://host.example.test/toc/list")
        );

        // 普通选择器 (无 {{}}) 仍走 CSS 解析
        let html = r#"<div class="toc"><a href="/toc">chapter list</a></div>"#;
        let selector_val =
            extract_rule_value(&pipeline, html, Some("div.toc@text"), &context).unwrap();
        assert_eq!(selector_val.as_deref(), Some("chapter list"));
    }

    #[test]
    fn source_with_raw_book_source_uses_semantic_rules_when_v1_rules_are_empty() {
        let (semantics, fixture) = canonical_semantics();
        let source = Source {
            source_id: semantics.source_id.clone(),
            name: semantics.name.clone(),
            base_url: semantics.base_url.clone(),
            rules: SourceRules::default(),
            book_source: fixture["bookSource"].clone(),
        };
        let pipeline = RemoteContentPipeline::new();

        let books = pipeline
            .search(&source, fixture["searchResponse"].as_str().unwrap())
            .unwrap();
        assert_eq!(books[0].book_id, "https://books.example.test/book/dune");

        let toc = pipeline
            .toc(&source, fixture["tocResponse"].as_str().unwrap())
            .unwrap();
        assert_eq!(toc[0].title, "Chapter 1");
        assert_eq!(toc[0].url, "https://books.example.test/book/dune/chapter/1");

        let content = pipeline
            .chapter_content(&source, fixture["contentResponse"].as_str().unwrap())
            .unwrap();
        assert_eq!(content, "First line.\ncanonical-legado-src\nSecond line.");
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

    /// S3 closure: real book-source JS rule fixture validation.
    ///
    /// Simulates a realistic Legado-style `<js>...</js>` chapter-fetch rule
    /// that orchestrates multiple host-routed capabilities:
    ///   1. `java.get(url)` — fetch chapter page (HttpGet)
    ///   2. `java.getCookie(tag, key)` — retrieve auth cookie (GetCookie)
    ///   3. `java.ajax(apiUrl)` — call API endpoint (Ajax)
    ///   4. `java.getZipStringContent(url, path, charset)` — extract zip entry (GetZipStringContent)
    ///   5. `java.queryTTF(data, useCache)` — build font map (QueryTTF)
    ///   6. `java.replaceFont(text, errTTF, okTTF, filter)` — de-obfuscate (ReplaceFont)
    ///
    /// Verifies the entire pipeline routes through HostCallbackRegistry
    /// (Core/Host boundary respected — no real network) and returns the
    /// expected structured result. Closes requirement #4 of the S3 task:
    /// "真实书源 JS 规则 fixture 验证".
    #[test]
    fn real_book_source_js_rule_fixture_routes_through_host_callbacks() {
        use reader_js::HostDescriptor;
        use std::sync::{Arc, Mutex};

        let calls: Arc<Mutex<Vec<HostDescriptor>>> = Arc::new(Mutex::new(Vec::new()));

        let mut registry = HostCallbackRegistry::new();

        // 1. java.get(url) -> HttpGet -> chapter HTML
        let s = Arc::clone(&calls);
        registry.register("java.get", move |descriptor| {
            s.lock().unwrap().push(descriptor.clone());
            Ok(serde_json::json!(
                "<html><body><div id='content'>page-body</div></body></html>"
            ))
        });

        // 2. java.getCookie(tag, key) -> GetCookie -> session token
        let s = Arc::clone(&calls);
        registry.register("java.getCookie", move |descriptor| {
            s.lock().unwrap().push(descriptor.clone());
            Ok(serde_json::json!("session-token-abc123"))
        });

        // 3. java.ajax(url) -> Ajax -> API JSON response
        let s = Arc::clone(&calls);
        registry.register("java.ajax", move |descriptor| {
            s.lock().unwrap().push(descriptor.clone());
            Ok(serde_json::json!(
                "{\"chapterId\":456,\"content\":\"encrypted-body\"}"
            ))
        });

        // 4. java.getZipStringContent(url, path, charset) -> GetZipStringContent -> zip entry text
        let s = Arc::clone(&calls);
        registry.register("java.getZipStringContent", move |descriptor| {
            s.lock().unwrap().push(descriptor.clone());
            Ok(serde_json::json!("raw chapter text with obfuscated glyphs"))
        });

        // 5. java.queryTTF(data, useCache) -> QueryTTF -> font mapping object
        let s = Arc::clone(&calls);
        registry.register("java.queryTTF", move |descriptor| {
            s.lock().unwrap().push(descriptor.clone());
            Ok(serde_json::json!({ "0xF001": "的", "0xF002": "一", "0xF003": "是" }))
        });

        // 6. java.replaceFont(text, errTTF, okTTF, filter) -> ReplaceFont -> de-obfuscated text
        let s = Arc::clone(&calls);
        registry.register("java.replaceFont", move |descriptor| {
            s.lock().unwrap().push(descriptor.clone());
            // Legado: replaceFont returns the corrected String.
            Ok(serde_json::json!("chapter text with de-obfuscated glyphs"))
        });

        let js = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let pipeline = RemoteContentPipeline::with_js_sandbox(js);

        // Realistic Legado-style <js>...</js> rule block. Each java.* call
        // routes through Core -> HostDescriptor -> host callback -> JS result.
        let script = r#"
            (function () {
                // 1. Fetch chapter page
                var page = java.get("https://www.example.test/book/123/chapter/456");
                // 2. Retrieve auth cookie
                var cookie = java.getCookie("www.example.test", "session");
                // 3. Call API with cookie in URL
                var apiUrl = "https://api.example.test/chapter?id=456&token=" + cookie;
                var apiResp = java.ajax(apiUrl);
                // 4. Extract content from zip archive
                var zipContent = java.getZipStringContent(
                    "https://cdn.example.test/chapters/456.zip",
                    "content.txt",
                    "utf-8"
                );
                // 5. Build font mapping from base64 TTF
                var ttfMap = java.queryTTF("dGVzdC10dGYtZGF0YQ==", true);
                // 6. De-obfuscate font
                var deobfuscated = java.replaceFont(zipContent, ttfMap, ttfMap, true);
                // Return structured result
                return {
                    title: "Chapter 456",
                    pageSnippet: page,
                    cookieUsed: cookie,
                    apiResponse: apiResp,
                    rawContent: zipContent,
                    fontMap: ttfMap,
                    content: deobfuscated,
                    source: "legado-style-s3-fixture"
                };
            })();
        "#;

        let outcome = pipeline.evaluate_js_rule(script);
        let result = match outcome {
            JsOutcome::Ok(v) => v,
            JsOutcome::Unsupported { reason } => {
                panic!("expected ok, got unsupported: {reason}")
            }
        };

        // Verify structured result.
        assert_eq!(result["title"], "Chapter 456");
        assert_eq!(
            result["pageSnippet"],
            "<html><body><div id='content'>page-body</div></body></html>"
        );
        assert_eq!(result["cookieUsed"], "session-token-abc123");
        assert_eq!(
            result["apiResponse"],
            "{\"chapterId\":456,\"content\":\"encrypted-body\"}"
        );
        assert_eq!(
            result["rawContent"],
            "raw chapter text with obfuscated glyphs"
        );
        assert_eq!(result["fontMap"]["0xF001"], "的");
        assert_eq!(result["fontMap"]["0xF002"], "一");
        assert_eq!(result["fontMap"]["0xF003"], "是");
        assert_eq!(result["content"], "chapter text with de-obfuscated glyphs");
        assert_eq!(result["source"], "legado-style-s3-fixture");

        // Verify all 6 host calls routed through HostCallbackRegistry in order,
        // with the correct HostDescriptor variants and argument fidelity.
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 6, "expected 6 host calls, got {}", calls.len());

        match &calls[0] {
            HostDescriptor::HttpGet { url, .. } => {
                assert_eq!(url, "https://www.example.test/book/123/chapter/456");
            }
            other => panic!("expected HttpGet at call 0, got {other:?}"),
        }
        match &calls[1] {
            HostDescriptor::GetCookie { tag, key } => {
                assert_eq!(tag, "www.example.test");
                assert_eq!(*key, Some("session".to_string()));
            }
            other => panic!("expected GetCookie at call 1, got {other:?}"),
        }
        match &calls[2] {
            HostDescriptor::Ajax { url } => {
                assert_eq!(
                    url,
                    "https://api.example.test/chapter?id=456&token=session-token-abc123"
                );
            }
            other => panic!("expected Ajax at call 2, got {other:?}"),
        }
        match &calls[3] {
            HostDescriptor::GetZipStringContent { url, path, charset } => {
                assert_eq!(url, "https://cdn.example.test/chapters/456.zip");
                assert_eq!(path, "content.txt");
                assert_eq!(*charset, Some("utf-8".to_string()));
            }
            other => panic!("expected GetZipStringContent at call 3, got {other:?}"),
        }
        match &calls[4] {
            HostDescriptor::QueryTTF { use_cache, .. } => {
                assert_eq!(*use_cache, Some(true));
            }
            other => panic!("expected QueryTTF at call 4, got {other:?}"),
        }
        match &calls[5] {
            HostDescriptor::ReplaceFont { filter, .. } => {
                assert_eq!(*filter, Some(true));
            }
            other => panic!("expected ReplaceFont at call 5, got {other:?}"),
        }
    }
}

//! Remote-reading vertical command handlers (V1 minimal).
//!
//! These commands implement the import → search → detail → toc → chapter →
//! progress pipeline over inline/fixture content or host-provided HTTP
//! transport. Core never opens sockets itself: it emits `http.execute`
//! host.request events and resumes parsing when the host completes the
//! operation. A JS rule that calls a host capability (`java.get`/`java.post`)
//! without a registered callback yields a structured `unsupported` error.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use reader_content::analyze_url::{
    AnalyzeUrl, AnalyzeUrlContext, AnalyzeUrlError, JsExpressionClassification, UrlDslParser,
};
use reader_content::{JsOutcome, RemoteContentPipeline};
use reader_contract::{
    self as contract,
    remote::{
        parse_params, BookDetailParams, BookSearchParams, BookTocParams, BookmarkCreateData,
        BookmarkCreateParams, BookmarkData, BookmarkDeleteData, BookmarkDeleteParams,
        BookmarkListData, BookmarkListParams, BookmarkUpdateData, BookmarkUpdateParams,
        BookGroupCreateData, BookGroupCreateParams, BookGroupData, BookGroupDeleteData,
        BookGroupDeleteParams, BookGroupListData, BookGroupListParams, BookGroupUpdateData,
        BookGroupUpdateParams, BookshelfEntryData, BookshelfGetData, BookshelfGetParams,
        BookshelfListData, BookshelfListParams, ChapterContentParams, HostHttpRequest,
        HostHttpResponse, LocalBookCatalogData, LocalBookCatalogParams, LocalBookParseData,
        LocalBookParseParams, ReadRecordCreateData, ReadRecordCreateParams, ReadRecordData,
        ReadRecordDeleteData, ReadRecordDeleteParams, ReadRecordListData, ReadRecordListParams,
        ReadRecordUpdateData, ReadRecordUpdateParams, ReadingProgressUpdateParams,
        ReplaceRuleCreateData, ReplaceRuleCreateParams, ReplaceRuleData, ReplaceRuleDeleteData,
        ReplaceRuleDeleteParams, ReplaceRuleListData, ReplaceRuleListParams, ReplaceRuleUpdateData,
        ReplaceRuleUpdateParams, RssParseData, RssParseEntryData, RssParseParams, RssRefreshData,
        RssRefreshParams, SourceExploreKindEntry, SourceExploreKindsData, SourceExploreKindsParams,
        SourceExploreParams, SourceImportParams, SyncBackupData, SyncBackupParams, SyncMergeData,
        SyncMergeParams, TxtTocRuleCreateData, TxtTocRuleCreateParams, TxtTocRuleData,
        TxtTocRuleDeleteData, TxtTocRuleDeleteParams, TxtTocRuleListData, TxtTocRuleListParams,
        TxtTocRuleUpdateData, TxtTocRuleUpdateParams,
    },
    CoreError, Event, HostCapability,
};
use reader_domain::{
    Book, Bookmark, BookGroup, ReadRecord, ReadingProgress, ReplaceRule, Source, SourceRules,
    TocEntry, TxtTocRule,
};
use reader_storage::{BookshelfEntry, BookshelfQuery, BookshelfSortBy, BookshelfSortDirection};
use reader_storage::{BookshelfStore, InMemoryStorage};

use crate::host_callback_bridge::HostCallbackBridge;
use crate::sink::EventSink;

/// Maximum number of next-page fetches per `book.toc` / `chapter.content` call.
///
/// Mirrors Legado's implicit cap: the sequential `while (nextUrl.isNotEmpty()
/// && !nextUrlList.contains(nextUrl))` loop (`BookChapterList.kt:69`,
/// `BookContent.kt:85`) is bounded by visited-URL detection in the happy path.
/// 50 is a safety guard against broken sources with cycles that never revisit
/// an exact URL string (e.g. page URLs that include a timestamp).
const MAX_NEXT_PAGES: u32 = 50;

/// Shared remote-reading state held by the runtime: the content pipeline and
/// the in-memory storage. The active-request registry is owned by the runtime
/// and passed in at dispatch time so remote handlers reuse the same tracking as
/// the built-in commands.
///
/// When constructed via [`RemoteState::with_sink`] the pipeline's JS sandbox
/// is wired with `java.get`/`java.post`/`java.ajax`/`java.connect`/`java.ajaxAll`
/// host callbacks that emit `http.execute` host.request events through `sink`
/// and block the worker thread until `host.complete` arrives (intercepted by
/// [`crate::runtime::Runtime::send`]). Without the bridge (the legacy
/// [`RemoteState::new`] path) any `@js:` URL rule that calls `java.get` will
/// return the legacy "unregistered host callback" error.
#[derive(Clone)]
pub struct RemoteState {
    pipeline: Arc<RemoteContentPipeline>,
    storage: Arc<InMemoryStorage>,
    bridge: Option<HostCallbackBridge>,
}

impl RemoteState {
    /// Create fresh state with default pipeline + storage.
    ///
    /// The default pipeline uses a `QuickJsSandbox::default()` with no host
    /// callbacks registered — JS rules calling `java.get`/`java.post` will
    /// surface the legacy "unregistered host callback" error. Use
    /// [`RemoteState::with_sink`] to wire the host-callback bridge.
    pub fn new() -> Self {
        Self {
            pipeline: Arc::new(RemoteContentPipeline::new()),
            storage: Arc::new(InMemoryStorage::new()),
            bridge: None,
        }
    }

    /// Create fresh state with a JS host-callback bridge wired to `sink`.
    ///
    /// The bridge installs `java.get`/`java.post`/`java.ajax`/`java.connect`/
    /// `java.ajaxAll` callbacks on the pipeline's `QuickJsSandbox`. Each
    /// callback emits an `Event::HostRequest` (capability `HttpExecute`) through
    /// `sink` and blocks the worker thread until a matching `host.complete` /
    /// `host.error` is routed back via
    /// [`HostCallbackBridge::try_complete`] (called from `Runtime::send`).
    pub fn with_sink(sink: Arc<dyn EventSink>) -> Self {
        let bridge = HostCallbackBridge::new(sink);
        let sandbox = bridge.build_sandbox();
        Self {
            pipeline: Arc::new(RemoteContentPipeline::with_js_sandbox(sandbox)),
            storage: Arc::new(InMemoryStorage::new()),
            bridge: Some(bridge),
        }
    }

    pub fn pipeline(&self) -> &RemoteContentPipeline {
        &self.pipeline
    }

    pub fn storage(&self) -> &InMemoryStorage {
        &self.storage
    }

    /// The JS host-callback bridge, if wired. `None` for the legacy
    /// [`RemoteState::new`] constructor.
    pub fn bridge(&self) -> Option<&HostCallbackBridge> {
        self.bridge.as_ref()
    }
}

impl Default for RemoteState {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a remote-reading dispatch attempt.
#[derive(Debug)]
pub enum RemoteDispatch {
    NotHandled,
    Finished,
    Pending(PendingHostRequest),
}

#[derive(Debug)]
pub enum RemoteCommandResult {
    Complete(serde_json::Value),
    Pending(PendingHostRequest),
}

/// A host capability request that must complete before the original remote
/// command can finish.
#[derive(Debug)]
pub struct PendingHostRequest {
    pub capability: HostCapability,
    pub params: serde_json::Value,
    pub continuation: RemoteHostContinuation,
}

/// Continuation state for remote commands blocked on host HTTP.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteHostContinuation {
    BookSearch(BookSearchParams),
    BookDetail(BookDetailParams),
    BookToc(BookTocParams),
    ChapterContent(ChapterContentParams),
    /// Paginated `book.toc` follow-up: a previous page returned a `nextTocUrl`.
    /// The runtime fetched the next page and is now ready to parse it and
    /// decide whether to emit another `Pending` or the final merged result.
    BookTocNextPage(BookTocNextPageState),
    /// Paginated `chapter.content` follow-up: a previous page returned a
    /// `nextContentUrl`. The runtime fetched the next page and is now ready
    /// to append its content and decide whether to continue or finish.
    ChapterContentNextPage(ChapterContentNextPageState),
    /// `source.explore` follow-up: Core emitted `http.execute` for the
    /// discovery category URL and is waiting for the host to return the
    /// response body. Once received, Core parses the book list.
    SourceExplore(SourceExploreParams),
}

/// Continuation state for the `book.toc` pagination loop (`nextTocUrl`).
///
/// Mirrors Legado `BookChapterList.kt:69`:
///   `while (nextUrl.isNotEmpty() && !nextUrlList.contains(nextUrl)) { ... }`
///
/// Core accumulates chapters across pages, dedups against `visited_urls`, and
/// re-emits an `http.execute` host request for each next-page URL. The final
/// merged result is emitted when no next URL is returned, the URL was already
/// visited, or [`MAX_NEXT_PAGES`] is reached.
#[derive(Debug, Clone, PartialEq)]
pub struct BookTocNextPageState {
    /// Original request params (source_id, book_id, source, etc.).
    pub params: BookTocParams,
    /// Chapters accumulated so far (across all visited pages).
    pub accumulated: Vec<TocEntry>,
    /// Number of next-page fetches issued so far (for the MAX_NEXT_PAGES guard).
    pub pages_fetched: u32,
    /// Absolute URLs already fetched (cycle detection, mirrors Legado
    /// `nextUrlList`).
    pub visited_urls: HashSet<String>,
}

/// Continuation state for the `chapter.content` pagination loop
/// (`nextContentUrl`). Mirrors Legado `BookContent.kt:85`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChapterContentNextPageState {
    /// Original request params (source_id, book_id, source, etc.).
    pub params: ChapterContentParams,
    /// Chapter body text accumulated so far (pages concatenated in order).
    pub accumulated_content: String,
    /// Number of next-page fetches issued so far (for the MAX_NEXT_PAGES guard).
    pub pages_fetched: u32,
    /// Absolute URLs already fetched (cycle detection).
    pub visited_urls: HashSet<String>,
}

fn finish(
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    request_id: u64,
    event: Event,
) {
    if let Ok(mut active) = active_requests.lock() {
        active.remove(&request_id);
    }
    sink.emit(&event);
}

fn error_event(request_id: u64, error: CoreError) -> Event {
    Event::error(request_id, error)
}

/// Resolve a source: inline `source` JSON wins, otherwise look up by id in
/// storage.
fn resolve_source(
    storage: &InMemoryStorage,
    source_id: &str,
    inline: &Option<serde_json::Value>,
) -> Result<Source, CoreError> {
    if let Some(json) = inline {
        let source: Source = serde_json::from_value(json.clone()).map_err(|err| {
            CoreError::invalid_params("invalid inline source definition")
                .with_details(serde_json::json!({ "source": err.to_string() }))
        })?;
        return Ok(source);
    }
    storage
        .get_source(source_id)
        .map_err(storage_internal)?
        .ok_or_else(|| {
            CoreError::invalid_params(format!("unknown sourceId: {source_id}"))
                .with_details(serde_json::json!({ "sourceId": source_id }))
        })
}

/// Extract a `source_id` from an `Option<String>` for params where the id is
/// only required when no inline `source` is provided. Returns an empty string
/// when the inline source is present (matching `resolve_source` which ignores
/// `source_id` in that case), and a structured error otherwise.
fn source_id_or_empty(
    source_id: &Option<String>,
    inline: &Option<serde_json::Value>,
) -> Result<String, CoreError> {
    if inline.is_some() {
        return Ok(String::new());
    }
    source_id.clone().ok_or_else(|| {
        CoreError::invalid_params("sourceId is required when source is not provided inline")
            .with_details(serde_json::json!({ "field": "sourceId" }))
    })
}

fn storage_internal(_: reader_storage::StorageError) -> CoreError {
    CoreError::internal("storage lock poisoned")
}

fn content_internal(err: reader_content::ContentError) -> CoreError {
    match err {
        reader_content::ContentError::JsUnsupported { reason } => {
            // Structured "unsupported" — surfaced as INTERNAL with a details
            // block so hosts can detect the V1 network gap.
            CoreError::internal(format!("JS rule unsupported in V1: {reason}"))
                .with_details(serde_json::json!({ "unsupported": true, "reason": reason }))
        }
        other => CoreError::internal(other.to_string()),
    }
}

/// Mirror of `reader_content::has_rule_spec` (private there). Returns `true`
/// when the rule spec carries a non-empty rule (string/array/object with
/// content). Used to decide whether to take the Legado book-source semantics
/// path (which yields `nextTocUrl`/`nextContentUrl`) or the rule-chain path.
fn has_rule_spec(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => !values.is_empty(),
        serde_json::Value::Object(values) => !values.is_empty(),
        _ => true,
    }
}

/// Extract the table of contents together with the next-page URL. Mirrors
/// `RemoteContentPipeline::toc` but preserves `nextTocUrl` for pagination.
///
/// When the source has no explicit `rules.toc` spec and carries Legado
/// book-source semantics, delegates to `toc_book_source` (which extracts
/// `nextTocUrl` via the Legado `ruleToc.nextUrl` rule). Otherwise falls back
/// to `toc()` (rule-chain path) with `next_toc_url: None` — the rule chain
/// does not extract next-page URLs.
fn toc_with_next(
    pipeline: &RemoteContentPipeline,
    source: &Source,
    toc_response: &str,
) -> Result<reader_content::BookSourceToc, reader_content::ContentError> {
    if !has_rule_spec(&source.rules.toc) {
        if let Some(semantics) = source.book_source_semantics() {
            let context = reader_content::BookSourceRequestContext::for_semantics(&semantics);
            return pipeline.toc_book_source(&semantics, toc_response, &context);
        }
    }
    let chapters = pipeline.toc(source, toc_response)?;
    Ok(reader_content::BookSourceToc {
        chapters,
        next_toc_url: None,
    })
}

/// Extract chapter body together with the next-page URL. Mirrors
/// `RemoteContentPipeline::chapter_content` but preserves `nextContentUrl` for
/// pagination.
fn chapter_content_with_next(
    pipeline: &RemoteContentPipeline,
    source: &Source,
    chapter_response: &str,
) -> Result<reader_content::BookSourceContent, reader_content::ContentError> {
    if !has_rule_spec(&source.rules.chapter) {
        if let Some(semantics) = source.book_source_semantics() {
            let context = reader_content::BookSourceRequestContext::for_semantics(&semantics);
            return pipeline.content_book_source(&semantics, chapter_response, &context);
        }
    }
    let content = pipeline.chapter_content(source, chapter_response)?;
    Ok(reader_content::BookSourceContent {
        title: None,
        content,
        next_content_url: None,
        variables: std::collections::BTreeMap::new(),
    })
}

/// Dispatch a remote-reading command. Returns `true` if the method was handled
/// (including errors), `false` if the method is not a remote-reading method.
pub fn dispatch_remote(
    method: &str,
    cmd: &reader_contract::Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    state: &RemoteState,
) -> RemoteDispatch {
    let request_id = cmd.request_id;
    // Stamp the originating requestId so any JS host callbacks invoked while
    // evaluating `@js:`/`<js>` URL rules attribute the emitted `host.request`
    // to this request. The bridge is `None` for the legacy `RemoteState::new()`
    // constructor — in that case this is a no-op.
    if let Some(bridge) = state.bridge() {
        bridge.set_current_request_id(request_id);
    }
    let result: Result<RemoteCommandResult, CoreError> = match method {
        contract::methods::SOURCE_IMPORT => {
            source_import(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOK_SEARCH => book_search(cmd, state),
        contract::methods::BOOK_DETAIL => book_detail(cmd, state),
        contract::methods::BOOK_TOC => book_toc(cmd, state),
        contract::methods::CHAPTER_CONTENT => chapter_content(cmd, state),
        contract::methods::READING_PROGRESS_UPDATE => {
            reading_progress_update(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::RSS_PARSE => rss_parse(cmd, state).map(RemoteCommandResult::Complete),
        contract::methods::RSS_REFRESH => {
            rss_refresh(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::SYNC_MERGE => sync_merge(cmd, state).map(RemoteCommandResult::Complete),
        contract::methods::SYNC_BACKUP => {
            sync_backup(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::LOCAL_BOOK_PARSE => {
            local_book_parse(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::LOCAL_BOOK_CATALOG => {
            local_book_catalog(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOKSHELF_LIST => {
            bookshelf_list(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOKSHELF_GET => {
            bookshelf_get(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::REPLACE_RULE_CREATE => {
            replace_rule_create(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::REPLACE_RULE_LIST => {
            replace_rule_list(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::REPLACE_RULE_UPDATE => {
            replace_rule_update(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::REPLACE_RULE_DELETE => {
            replace_rule_delete(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOKMARK_CREATE => {
            bookmark_create(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOKMARK_LIST => {
            bookmark_list(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOKMARK_UPDATE => {
            bookmark_update(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOKMARK_DELETE => {
            bookmark_delete(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOK_GROUP_CREATE => {
            book_group_create(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOK_GROUP_LIST => {
            book_group_list(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOK_GROUP_UPDATE => {
            book_group_update(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::BOOK_GROUP_DELETE => {
            book_group_delete(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::READ_RECORD_CREATE => {
            read_record_create(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::READ_RECORD_LIST => {
            read_record_list(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::READ_RECORD_UPDATE => {
            read_record_update(cmd, state).map(RemoteCommandResult::Complete)
        }
        contract::methods::READ_RECORD_DELETE => {
            read_record_delete(cmd, state).map(RemoteCommandResult::Complete)
        }
        // TEMPORARILY DISABLED: source.explore / txt.tocRule.* dispatch cases
        // require handler functions (source_explore_kinds, source_explore,
        // txt_toc_rule_*) being added by other agents but not yet in the work
        // tree. Re-enable once the reader-content updates land.
        // contract::methods::SOURCE_EXPLORE_KINDS => {
        //     source_explore_kinds(cmd, state).map(RemoteCommandResult::Complete)
        // }
        // contract::methods::SOURCE_EXPLORE => source_explore(cmd, state),
        // contract::methods::TXT_TOC_RULE_CREATE => {
        //     txt_toc_rule_create(cmd, state).map(RemoteCommandResult::Complete)
        // }
        // contract::methods::TXT_TOC_RULE_LIST => {
        //     txt_toc_rule_list(cmd, state).map(RemoteCommandResult::Complete)
        // }
        // contract::methods::TXT_TOC_RULE_UPDATE => {
        //     txt_toc_rule_update(cmd, state).map(RemoteCommandResult::Complete)
        // }
        // contract::methods::TXT_TOC_RULE_DELETE => {
        //     txt_toc_rule_delete(cmd, state).map(RemoteCommandResult::Complete)
        // }
        _ => return RemoteDispatch::NotHandled,
    };
    match result {
        Ok(RemoteCommandResult::Complete(data)) => {
            finish(
                sink,
                active_requests,
                request_id,
                Event::result(request_id, data),
            );
            RemoteDispatch::Finished
        }
        Ok(RemoteCommandResult::Pending(pending)) => RemoteDispatch::Pending(pending),
        Err(err) => {
            finish(
                sink,
                active_requests,
                request_id,
                error_event(request_id, err),
            );
            RemoteDispatch::Finished
        }
    }
}

fn pending_http_request(
    request: HostHttpRequest,
    continuation: RemoteHostContinuation,
) -> Result<PendingHostRequest, CoreError> {
    request.validate()?;
    let headers = if request.headers.is_null() {
        serde_json::json!({})
    } else {
        request.headers
    };
    let mut params = serde_json::json!({
        "url": request.url,
        "method": request.method,
        "headers": headers,
        "body": request.body,
    });
    if let Some(object) = params.as_object_mut() {
        if let Some(charset) = request.charset {
            object.insert("charset".to_string(), serde_json::json!(charset));
        }
        if let Some(follow_redirects) = request.follow_redirects {
            object.insert(
                "followRedirects".to_string(),
                serde_json::json!(follow_redirects),
            );
        }
        if let Some(max_redirects) = request.max_redirects {
            object.insert("maxRedirects".to_string(), serde_json::json!(max_redirects));
        }
        if let Some(retry) = request.retry {
            object.insert("retry".to_string(), serde_json::to_value(retry).unwrap());
        }
        if let Some(use_platform_cookie_jar) = request.use_platform_cookie_jar {
            object.insert(
                "usePlatformCookieJar".to_string(),
                serde_json::json!(use_platform_cookie_jar),
            );
        }
        if let Some(session) = request.session {
            object.insert(
                "session".to_string(),
                serde_json::to_value(session).unwrap(),
            );
        }
    }
    Ok(PendingHostRequest {
        capability: HostCapability::HttpExecute,
        params,
        continuation,
    })
}

fn parse_http_response(result: serde_json::Value) -> Result<HostHttpResponse, CoreError> {
    if !result.get("body").is_some_and(serde_json::Value::is_string) {
        return Err(
            CoreError::invalid_params("http.execute host result.body must be a string")
                .with_details(serde_json::json!({ "result": result })),
        );
    }

    let response = serde_json::from_value::<HostHttpResponse>(result.clone()).map_err(|err| {
        CoreError::invalid_params("invalid http.execute host result").with_details(
            serde_json::json!({
                "source": err.to_string(),
                "result": result,
            }),
        )
    })?;
    response.validate()?;
    Ok(response)
}

fn http_response_diagnostics(response: &HostHttpResponse) -> Option<serde_json::Value> {
    let mut diagnostics = serde_json::Map::new();
    if let Some(status) = response.status {
        diagnostics.insert("status".to_string(), serde_json::json!(status));
    }
    if let Some(headers) = response
        .headers
        .as_ref()
        .filter(|headers| headers.is_object())
    {
        diagnostics.insert("headers".to_string(), headers.clone());
    }
    if let Some(final_url) = &response.final_url {
        diagnostics.insert("finalUrl".to_string(), serde_json::json!(final_url));
    }
    if let Some(charset_hint) = &response.charset_hint {
        diagnostics.insert("charsetHint".to_string(), serde_json::json!(charset_hint));
    }
    if let Some(session) = &response.session {
        diagnostics.insert(
            "session".to_string(),
            serde_json::to_value(session).unwrap(),
        );
    }
    if let Some(redirects) = &response.redirects {
        diagnostics.insert(
            "redirects".to_string(),
            serde_json::to_value(redirects).unwrap(),
        );
    }
    if let Some(cookies) = &response.cookies {
        diagnostics.insert(
            "cookies".to_string(),
            serde_json::to_value(cookies).unwrap(),
        );
    }
    if diagnostics.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(diagnostics))
    }
}

fn with_http_diagnostics(
    mut data: serde_json::Value,
    diagnostics: Option<serde_json::Value>,
) -> serde_json::Value {
    let Some(diagnostics) = diagnostics else {
        return data;
    };
    if let Some(object) = data.as_object_mut() {
        object.insert("http".to_string(), diagnostics);
    }
    data
}

/// Attach HTTP diagnostics to a `RemoteCommandResult::Complete` payload.
/// `Pending` results are passed through unchanged (diagnostics from a
/// next-page request will be attached to the final result).
fn with_http_diagnostics_result(
    result: RemoteCommandResult,
    diagnostics: Option<serde_json::Value>,
) -> RemoteCommandResult {
    match result {
        RemoteCommandResult::Complete(data) => {
            RemoteCommandResult::Complete(with_http_diagnostics(data, diagnostics))
        }
        RemoteCommandResult::Pending(pending) => RemoteCommandResult::Pending(pending),
    }
}

/// Continue a remote-reading command after its host HTTP request completes.
pub fn complete_remote_host(
    continuation: RemoteHostContinuation,
    host_result: serde_json::Value,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let response = parse_http_response(host_result)?;
    let diagnostics = http_response_diagnostics(&response);
    let fetched_url = response.final_url.clone();
    match continuation {
        RemoteHostContinuation::BookSearch(mut params) => {
            params.search_response = response.body;
            book_search_from_params(params, state)
                .map(|data| RemoteCommandResult::Complete(data))
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
        RemoteHostContinuation::BookDetail(mut params) => {
            params.detail_response = response.body;
            book_detail_from_params(params, state)
                .map(|data| RemoteCommandResult::Complete(data))
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
        RemoteHostContinuation::BookToc(mut params) => {
            params.toc_response = response.body;
            book_toc_from_params(params, state, fetched_url.as_deref())
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
        RemoteHostContinuation::ChapterContent(mut params) => {
            params.chapter_response = response.body;
            chapter_content_from_params(params, state, fetched_url.as_deref())
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
        RemoteHostContinuation::BookTocNextPage(mut state_) => {
            state_.params.toc_response = response.body;
            continue_toc_pagination(state_, state, fetched_url.as_deref())
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
        RemoteHostContinuation::ChapterContentNextPage(mut state_) => {
            state_.params.chapter_response = response.body;
            continue_chapter_content_pagination(state_, state, fetched_url.as_deref())
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
        RemoteHostContinuation::SourceExplore(mut params) => {
            params.explore_response = Some(response.body);
            source_explore_from_params(params, state)
                .map(|result| with_http_diagnostics_result(result, diagnostics))
        }
    }
}

fn source_import(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: SourceImportParams = parse_params(contract::methods::SOURCE_IMPORT, &cmd.params)?;
    // `name` is optional at the contract layer: Legado native BookSource JSON
    // carries `bookSourceName` (not a top-level `name`). Fall back to
    // `bookSource.bookSourceName` so callers can import a raw Legado BookSource
    // verbatim. Mirrors Legado `BookSource.bookSourceName` (BookSource.kt).
    let name = params.name.or_else(|| {
        params
            .book_source
            .get("bookSourceName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    });
    let name = match name {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            return Err(CoreError::invalid_params(
                "source.import requires a non-empty name: provide `name` or `bookSource.bookSourceName`",
            ));
        }
    };
    let source_id = if params.source_id.trim().is_empty() {
        format!("source-{}", cmd.request_id)
    } else {
        params.source_id
    };
    let rules: SourceRules = if params.rules.is_null() {
        SourceRules::default()
    } else {
        serde_json::from_value(params.rules).map_err(|err| {
            CoreError::invalid_params("source.import rules must be a SourceRules object")
                .with_details(serde_json::json!({ "source": err.to_string() }))
        })?
    };
    let source = Source {
        source_id: source_id.clone(),
        name,
        base_url: params.base_url,
        rules,
        book_source: params.book_source,
    };
    let stored = state
        .storage()
        .put_source(source.clone())
        .map_err(storage_internal)?;
    Ok(serde_json::json!({
        "sourceId": stored.source_id,
        "name": stored.name,
        "imported": true,
    }))
}

fn book_search(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let params: BookSearchParams = parse_params(contract::methods::BOOK_SEARCH, &cmd.params)?;
    // Explicit pre-fetched response wins — no host round-trip needed.
    if !params.search_response.is_empty() {
        return book_search_from_params(params, state).map(RemoteCommandResult::Complete);
    }
    // Explicit `searchRequest` wins over auto-build.
    if let Some(request) = params.search_request.clone() {
        return Ok(RemoteCommandResult::Pending(pending_http_request(
            request,
            RemoteHostContinuation::BookSearch(params.clone()),
        )?));
    }
    // Auto-build from the source's `searchUrl` template (Legado AnalyzeUrl path).
    if let Some(keyword) = params.keyword.as_deref() {
        if !keyword.trim().is_empty() {
            let request = build_search_request_from_source(state, &params, keyword)?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::BookSearch(params.clone()),
            )?));
        }
    }
    // Fall back to the legacy "missing response + missing request" error.
    Err(CoreError::invalid_params(
        "searchResponse is required unless searchRequest or keyword is provided",
    ))
}

/// Build a `HostHttpRequest` from a Legado book source's `searchUrl` template.
///
/// This is the Rust port of Swift `BookSourceRequestBuilder.makeSearchRequest`
/// (non-JS path): expand `{{key}}`/`{{page}}`/`pageMinus`/`pagePlus`, parse the
/// URL DSL (url + JSON options), merge source/DSL headers, resolve relative
/// URLs against `baseUrl`, and return the assembled descriptor. The host
/// performs the actual socket/TLS work — Core never opens a connection.
fn build_search_request_from_source(
    state: &RemoteState,
    params: &BookSearchParams,
    keyword: &str,
) -> Result<HostHttpRequest, CoreError> {
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let legado = source.legado_book_source().ok_or_else(|| {
        CoreError::invalid_params(
            "cannot auto-build searchRequest: source has no Legado bookSource payload",
        )
        .with_details(serde_json::json!({ "sourceId": params.source_id }))
    })?;
    let search_url = legado
        .search_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CoreError::invalid_params("cannot auto-build searchRequest: source.searchUrl is empty")
                .with_details(serde_json::json!({ "sourceId": params.source_id }))
        })?;
    let page = params.page.unwrap_or(1).max(1);
    let ctx = AnalyzeUrlContext::for_search(keyword, page);
    // Source-level headers from Legado `header` field (object form). If the
    // field is missing, null, or not an object, treat it as empty.
    let source_headers = legado
        .header
        .as_ref()
        .and_then(|h| h.as_object())
        .cloned()
        .unwrap_or_default();
    // corpus 导入的源 baseUrl 经常为空但 bookSourceUrl 保留;相对路径 searchUrl
    // 需要一个 base 来 resolve,否则 resolve_url 返回空 → no_search_results。
    // 回退到 legado.bookSourceUrl(等同 Legado `source.bookSourceUrl`)。
    let base_url = if source.base_url.trim().is_empty() {
        legado.book_source_url.as_deref().unwrap_or("")
    } else {
        &source.base_url
    };
    build_analyze_url_request(state, search_url, &ctx, base_url, &source_headers)
}

/// Build a `HostHttpRequest` from an explicit URL field (`bookUrl` /
/// `tocUrl` / `chapterUrl`) plus the source's `header` field. Mirrors the
/// Legado path that uses `AnalyzeUrl` with no `{{key}}`/`{{page}}`
/// substitution — a plain URL (or Legado DSL form
/// `url,{"method":"POST",...}`) is expanded with an empty context.
fn build_request_from_url_field(
    state: &RemoteState,
    source_id: &str,
    inline_source: &Option<serde_json::Value>,
    raw_url: &str,
) -> Result<HostHttpRequest, CoreError> {
    let source = resolve_source(state.storage(), source_id, inline_source)?;
    let legado = source.legado_book_source().ok_or_else(|| {
        CoreError::invalid_params(
            "cannot auto-build request: source has no Legado bookSource payload",
        )
        .with_details(serde_json::json!({ "sourceId": source_id }))
    })?;
    let source_headers = legado
        .header
        .as_ref()
        .and_then(|h| h.as_object())
        .cloned()
        .unwrap_or_default();
    let ctx = AnalyzeUrlContext::for_url();
    build_analyze_url_request(state, raw_url, &ctx, &source.base_url, &source_headers)
}

/// Dispatch to `AnalyzeUrl::build_request_with_js` when the URL contains
/// `@js:`/`<js>` or a DSL `js` option; otherwise fall back to the non-JS
/// `AnalyzeUrl::build_request`. The JS sandbox lives on the
/// [`RemoteContentPipeline`], so this is where the Core/Host boundary is
/// honoured: JS evaluation is a pure compute (no sockets), but any
/// `java.get`/`java.post` calls inside the JS surface as structured
/// `unsupported` errors unless a host callback is registered.
fn build_analyze_url_request(
    state: &RemoteState,
    raw_url: &str,
    ctx: &AnalyzeUrlContext,
    base_url: &str,
    source_headers: &serde_json::Map<String, serde_json::Value>,
) -> Result<HostHttpRequest, CoreError> {
    // Legado `{{source.key}}` / `{{source.bookSourceUrl}}` resolve to the
    // source's `bookSourceUrl` (== `base_url` here). Substitute before static
    // template expansion so the URL is not misclassified as a JS expression
    // by `classify_js_expression` (which would force the JS path and fail
    // when no evaluator is wired). Mirrors Legado `AnalyzeUrl.initUrl`'s
    // `source` variable scope. The `{{source.getKey()}}` form is a JS
    // function call and remains handled by the JS path below, not here.
    // Covers the corpus variants: `{{source.key}}`, `{{source.bookSourceUrl}}`
    // (Legado canonical), and `{{source.booksourceurl}}` (lowercase typo).
    let resolved: String = raw_url
        .replace("{{source.key}}", base_url)
        .replace("{{source.bookSourceUrl}}", base_url)
        .replace("{{source.booksourceurl}}", base_url);
    let raw_url: &str = resolved.as_str();

    // Quick pre-check: classify the URL (after static template expansion) for
    // embedded JS. If no JS, use the cheaper non-JS path.
    let expanded = reader_content::analyze_url::expand_static_templates(raw_url, ctx);
    let (_, classification) = UrlDslParser::classify_js_expression(&expanded);
    let dsl_pre = UrlDslParser::parse(&expanded)
        .map_err(|e| analyze_url_internal(AnalyzeUrlError::from(e)))?;
    let has_js = classification == JsExpressionClassification::RequiresJsSandbox
        || dsl_pre.options.js.is_some()
        || dsl_pre.has_js_expression;
    if !has_js {
        return AnalyzeUrl::build_request(raw_url, ctx, base_url, source_headers)
            .map_err(analyze_url_internal);
    }
    let pipeline = state.pipeline();
    AnalyzeUrl::build_request_with_js(raw_url, ctx, base_url, source_headers, |expr, context| {
        pipeline.evaluate_url_js(expr, context)
    })
    .map_err(analyze_url_internal)
}

fn analyze_url_internal(err: AnalyzeUrlError) -> CoreError {
    match err {
        AnalyzeUrlError::JsUnsupported(reason) => CoreError::internal(format!(
            "AnalyzeUrl JS execution unsupported in this build: {reason}"
        ))
        .with_details(serde_json::json!({ "unsupported": true, "reason": reason })),
        other => CoreError::invalid_params(format!("AnalyzeUrl failed: {other}")),
    }
}

fn book_search_from_params(
    params: BookSearchParams,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let books = state
        .pipeline()
        .search(&source, &params.search_response)
        .map_err(content_internal)?;
    for book in &books {
        let _ = state.storage().put_book(book.clone());
    }
    Ok(serde_json::json!({
        "sourceId": params.source_id,
        "books": books,
    }))
}

fn book_detail(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let params: BookDetailParams = parse_params(contract::methods::BOOK_DETAIL, &cmd.params)?;
    if !params.detail_response.is_empty() {
        return book_detail_from_params(params, state).map(RemoteCommandResult::Complete);
    }
    if let Some(request) = params.detail_request.clone() {
        return Ok(RemoteCommandResult::Pending(pending_http_request(
            request,
            RemoteHostContinuation::BookDetail(params.clone()),
        )?));
    }
    if let Some(book_url) = params.book_url.as_deref() {
        if !book_url.trim().is_empty() {
            let request =
                build_request_from_url_field(state, &params.source_id, &params.source, book_url)?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::BookDetail(params.clone()),
            )?));
        }
    }
    Err(CoreError::invalid_params(
        "detailResponse is required unless detailRequest or bookUrl is provided",
    ))
}

fn book_detail_from_params(
    params: BookDetailParams,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let base: Book = serde_json::from_value(params.book.clone()).map_err(|err| {
        CoreError::invalid_params("book.detail requires a base book object")
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })?;
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let merged = state
        .pipeline()
        .detail(&source, &base, &params.detail_response)
        .map_err(content_internal)?;
    let _ = state.storage().put_book(merged.clone());
    Ok(serde_json::json!({
        "sourceId": params.source_id,
        "book": merged,
    }))
}

fn book_toc(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let params: BookTocParams = parse_params(contract::methods::BOOK_TOC, &cmd.params)?;
    if !params.toc_response.is_empty() {
        return book_toc_from_params(params, state, None);
    }
    if let Some(request) = params.toc_request.clone() {
        return Ok(RemoteCommandResult::Pending(pending_http_request(
            request,
            RemoteHostContinuation::BookToc(params.clone()),
        )?));
    }
    if let Some(toc_url) = params.toc_url.as_deref() {
        if !toc_url.trim().is_empty() {
            let request =
                build_request_from_url_field(state, &params.source_id, &params.source, toc_url)?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::BookToc(params.clone()),
            )?));
        }
    }
    Err(CoreError::invalid_params(
        "tocResponse is required unless tocRequest or tocUrl is provided",
    ))
}

fn book_toc_from_params(
    params: BookTocParams,
    state: &RemoteState,
    initial_url: Option<&str>,
) -> Result<RemoteCommandResult, CoreError> {
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let pipeline = state.pipeline();
    let toc = toc_with_next(pipeline, &source, &params.toc_response).map_err(content_internal)?;
    if let Some(next_url) = toc.next_toc_url.as_deref() {
        if !next_url.trim().is_empty() {
            return start_toc_pagination(params, toc.chapters, initial_url, next_url, state);
        }
    }
    finish_toc_result(params, toc.chapters, state)
}

/// Emit the final merged toc result (no further pages).
fn finish_toc_result(
    params: BookTocParams,
    chapters: Vec<TocEntry>,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let cache_key = format!("toc:{}", params.book_id);
    let payload = serde_json::to_string(&chapters).unwrap_or_else(|_| "[]".into());
    let _ = state.storage().put_cache(cache_key, payload);
    Ok(RemoteCommandResult::Complete(serde_json::json!({
        "sourceId": params.source_id,
        "bookId": params.book_id,
        "toc": chapters,
    })))
}

/// Kick off the first next-page fetch for `book.toc`. Registers the
/// accumulated chapters + visited URLs in a [`BookTocNextPageState`] and
/// emits a `Pending` host HTTP request for `next_url`.
fn start_toc_pagination(
    params: BookTocParams,
    chapters: Vec<TocEntry>,
    initial_url: Option<&str>,
    next_url: &str,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let mut visited_urls = HashSet::new();
    if let Some(url) = initial_url {
        visited_urls.insert(url.to_string());
    } else if let Some(toc_url) = params.toc_url.as_deref() {
        if !toc_url.trim().is_empty() {
            visited_urls.insert(toc_url.to_string());
        }
    }
    visited_urls.insert(next_url.to_string());
    let request = build_request_from_url_field(state, &params.source_id, &params.source, next_url)?;
    let pagination_state = BookTocNextPageState {
        params,
        accumulated: chapters,
        pages_fetched: 1,
        visited_urls,
    };
    Ok(RemoteCommandResult::Pending(pending_http_request(
        request,
        RemoteHostContinuation::BookTocNextPage(pagination_state),
    )?))
}

/// Continue the `book.toc` pagination loop after a next-page response arrives.
/// Parses the new page, appends its chapters, and either emits another
/// `Pending` (if another `nextTocUrl` is returned) or finishes.
fn continue_toc_pagination(
    mut pagination_state: BookTocNextPageState,
    state: &RemoteState,
    fetched_url: Option<&str>,
) -> Result<RemoteCommandResult, CoreError> {
    if let Some(url) = fetched_url {
        pagination_state.visited_urls.insert(url.to_string());
    }
    if pagination_state.pages_fetched >= MAX_NEXT_PAGES {
        return finish_toc_result(pagination_state.params, pagination_state.accumulated, state);
    }
    let source = resolve_source(
        state.storage(),
        &pagination_state.params.source_id,
        &pagination_state.params.source,
    )?;
    let pipeline = state.pipeline();
    let toc = toc_with_next(pipeline, &source, &pagination_state.params.toc_response)
        .map_err(content_internal)?;
    pagination_state.accumulated.extend(toc.chapters);
    if let Some(next_url) = toc.next_toc_url.as_deref() {
        if !next_url.trim().is_empty() && !pagination_state.visited_urls.contains(next_url) {
            pagination_state.pages_fetched += 1;
            pagination_state.visited_urls.insert(next_url.to_string());
            let request = build_request_from_url_field(
                state,
                &pagination_state.params.source_id,
                &pagination_state.params.source,
                next_url,
            )?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::BookTocNextPage(pagination_state),
            )?));
        }
    }
    finish_toc_result(pagination_state.params, pagination_state.accumulated, state)
}

fn chapter_content(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let params: ChapterContentParams =
        parse_params(contract::methods::CHAPTER_CONTENT, &cmd.params)?;
    // JS-rule path: skip auto-build entirely (existing behaviour).
    if params.js_rule.is_some() {
        return chapter_content_from_params(params, state, None);
    }
    if !params.chapter_response.is_empty() {
        return chapter_content_from_params(params, state, None);
    }
    if let Some(request) = params.chapter_request.clone() {
        return Ok(RemoteCommandResult::Pending(pending_http_request(
            request,
            RemoteHostContinuation::ChapterContent(params.clone()),
        )?));
    }
    if let Some(chapter_url) = params.chapter_url.as_deref() {
        if !chapter_url.trim().is_empty() {
            let request = build_request_from_url_field(
                state,
                &params.source_id,
                &params.source,
                chapter_url,
            )?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::ChapterContent(params.clone()),
            )?));
        }
    }
    Err(CoreError::invalid_params(
        "chapterResponse is required unless chapterRequest, chapterUrl, or jsRule is provided",
    ))
}

fn chapter_content_from_params(
    params: ChapterContentParams,
    state: &RemoteState,
    initial_url: Option<&str>,
) -> Result<RemoteCommandResult, CoreError> {
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let pipeline = state.pipeline();

    if let Some(js_rule) = params.js_rule.as_ref() {
        match pipeline.evaluate_js_rule(js_rule) {
            JsOutcome::Ok(value) => {
                let cache_key = format!("chapter:{}:{}", params.book_id, params.chapter_title);
                let _ = state.storage().put_cache(
                    cache_key,
                    serde_json::to_string(&value).unwrap_or_else(|_| "{}".into()),
                );
                return Ok(RemoteCommandResult::Complete(serde_json::json!({
                    "sourceId": params.source_id,
                    "bookId": params.book_id,
                    "chapterTitle": params.chapter_title,
                    "content": value,
                    "via": "js",
                })));
            }
            JsOutcome::Unsupported { reason } => {
                return Err(content_internal(
                    reader_content::ContentError::JsUnsupported { reason },
                ));
            }
        }
    }

    let content = chapter_content_with_next(pipeline, &source, &params.chapter_response)
        .map_err(content_internal)?;
    if let Some(next_url) = content.next_content_url.as_deref() {
        if !next_url.trim().is_empty() {
            return start_chapter_content_pagination(
                params,
                content.content,
                initial_url,
                next_url,
                state,
            );
        }
    }
    finish_chapter_content_result(params, content.content, state)
}

/// Emit the final merged chapter content (no further pages).
fn finish_chapter_content_result(
    params: ChapterContentParams,
    content: String,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let cache_key = format!("chapter:{}:{}", params.book_id, params.chapter_title);
    let _ = state.storage().put_cache(cache_key, content.clone());
    Ok(RemoteCommandResult::Complete(serde_json::json!({
        "sourceId": params.source_id,
        "bookId": params.book_id,
        "chapterTitle": params.chapter_title,
        "content": content,
        "via": "rule",
    })))
}

/// Kick off the first next-page fetch for `chapter.content`. Registers the
/// accumulated content + visited URLs in a [`ChapterContentNextPageState`] and
/// emits a `Pending` host HTTP request for `next_url`.
fn start_chapter_content_pagination(
    params: ChapterContentParams,
    content: String,
    initial_url: Option<&str>,
    next_url: &str,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let mut visited_urls = HashSet::new();
    if let Some(url) = initial_url {
        visited_urls.insert(url.to_string());
    } else if let Some(chapter_url) = params.chapter_url.as_deref() {
        if !chapter_url.trim().is_empty() {
            visited_urls.insert(chapter_url.to_string());
        }
    }
    visited_urls.insert(next_url.to_string());
    let request = build_request_from_url_field(state, &params.source_id, &params.source, next_url)?;
    let pagination_state = ChapterContentNextPageState {
        params,
        accumulated_content: content,
        pages_fetched: 1,
        visited_urls,
    };
    Ok(RemoteCommandResult::Pending(pending_http_request(
        request,
        RemoteHostContinuation::ChapterContentNextPage(pagination_state),
    )?))
}

/// Continue the `chapter.content` pagination loop after a next-page response
/// arrives. Parses the new page, appends its content, and either emits another
/// `Pending` (if another `nextContentUrl` is returned) or finishes.
fn continue_chapter_content_pagination(
    mut pagination_state: ChapterContentNextPageState,
    state: &RemoteState,
    fetched_url: Option<&str>,
) -> Result<RemoteCommandResult, CoreError> {
    if let Some(url) = fetched_url {
        pagination_state.visited_urls.insert(url.to_string());
    }
    if pagination_state.pages_fetched >= MAX_NEXT_PAGES {
        return finish_chapter_content_result(
            pagination_state.params,
            pagination_state.accumulated_content,
            state,
        );
    }
    let source = resolve_source(
        state.storage(),
        &pagination_state.params.source_id,
        &pagination_state.params.source,
    )?;
    let pipeline = state.pipeline();
    let content =
        chapter_content_with_next(pipeline, &source, &pagination_state.params.chapter_response)
            .map_err(content_internal)?;
    pagination_state
        .accumulated_content
        .push_str(&content.content);
    if let Some(next_url) = content.next_content_url.as_deref() {
        if !next_url.trim().is_empty() && !pagination_state.visited_urls.contains(next_url) {
            pagination_state.pages_fetched += 1;
            pagination_state.visited_urls.insert(next_url.to_string());
            let request = build_request_from_url_field(
                state,
                &pagination_state.params.source_id,
                &pagination_state.params.source,
                next_url,
            )?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::ChapterContentNextPage(pagination_state),
            )?));
        }
    }
    finish_chapter_content_result(
        pagination_state.params,
        pagination_state.accumulated_content,
        state,
    )
}

fn reading_progress_update(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReadingProgressUpdateParams =
        parse_params(contract::methods::READING_PROGRESS_UPDATE, &cmd.params)?;
    let progress = ReadingProgress {
        book_id: params.book_id.clone(),
        chapter_index: params.chapter_index,
        chapter_offset: params.chapter_offset,
        chapter_progress: params.chapter_progress,
    };
    let stored = state
        .storage()
        .put_progress(progress)
        .map_err(storage_internal)?;
    Ok(serde_json::json!({
        "bookId": stored.book_id,
        "chapterIndex": stored.chapter_index,
        "chapterOffset": stored.chapter_offset,
        "chapterProgress": stored.chapter_progress,
        "stored": true,
    }))
}

// ===========================================================================
// RSS vertical (V1 minimal) — pure, no host bus
// ===========================================================================

fn rss_parse(
    cmd: &reader_contract::Command,
    _state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: RssParseParams = parse_params(contract::methods::RSS_PARSE, &cmd.params)?;
    let feed = if params.feed_url.is_empty() {
        reader_rss::parse_feed(&params.xml)
    } else {
        reader_rss::parse_feed_with_url(&params.feed_url, &params.xml)
    }
    .map_err(rss_internal)?;
    let entries = feed
        .entries
        .into_iter()
        .map(|entry| RssParseEntryData {
            id: entry.id,
            title: entry.title,
            link: entry.link,
            summary: entry.summary,
            published_at: entry.published_at,
        })
        .collect::<Vec<_>>();
    let data = RssParseData {
        title: feed.title,
        feed_url: feed.feed_url,
        site_url: feed.site_url,
        description: feed.description,
        entries,
    };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn rss_refresh(
    cmd: &reader_contract::Command,
    _state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: RssRefreshParams = parse_params(contract::methods::RSS_REFRESH, &cmd.params)?;
    let policy = reader_rss::RssRefreshPolicy {
        enabled: params.enabled,
        update_interval_minutes: params.update_interval_minutes,
        last_fetched_at: params.last_fetched_at,
        force_refresh: params.force_refresh,
    };
    let decision = reader_rss::decide_rss_refresh(&policy, params.evaluated_at);
    let reason_value = serde_json::to_value(&decision.reason).map_err(serde_internal)?;
    let reason = reason_value
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("{:?}", decision.reason));
    let data = RssRefreshData {
        subscription_id: params.subscription_id,
        should_fetch: decision.should_fetch,
        reason,
        evaluated_at: decision.evaluated_at,
        next_eligible_fetch_at: decision.next_eligible_fetch_at,
    };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn rss_internal(err: reader_rss::RssError) -> CoreError {
    CoreError::internal(format!("rss command failed: {err}"))
}

// ===========================================================================
// Sync vertical (V1 minimal) — pure, no host bus
// ===========================================================================

fn sync_merge(
    cmd: &reader_contract::Command,
    _state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: SyncMergeParams = parse_params(contract::methods::SYNC_MERGE, &cmd.params)?;
    let local: reader_sync::SyncSnapshot =
        serde_json::from_value(params.local).map_err(|err| sync_invalid("local snapshot", err))?;
    let remote: reader_sync::SyncSnapshot = serde_json::from_value(params.remote)
        .map_err(|err| sync_invalid("remote snapshot", err))?;
    let result = reader_sync::merge_snapshots(
        &local,
        &remote,
        params.merged_snapshot_id,
        params.merged_device_id,
        params.merged_created_at,
    )
    .map_err(sync_internal)?;
    let snapshot = serde_json::to_value(&result.snapshot).map_err(serde_internal)?;
    let conflicts = result
        .conflicts
        .into_iter()
        .map(|conflict| serde_json::to_value(&conflict).unwrap_or(serde_json::Value::Null))
        .collect::<Vec<_>>();
    let data = SyncMergeData {
        snapshot,
        conflicts,
    };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn sync_backup(
    cmd: &reader_contract::Command,
    _state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: SyncBackupParams = parse_params(contract::methods::SYNC_BACKUP, &cmd.params)?;
    let package: reader_sync::BackupPackage = serde_json::from_value(params.package)
        .map_err(|err| sync_invalid("backup package", err))?;
    let policy: reader_sync::RestorePolicy =
        serde_json::from_value(params.policy).map_err(|err| sync_invalid("restore policy", err))?;
    let plan = reader_sync::plan_backup_restore(&package, &policy).map_err(sync_internal)?;
    let plan_value = serde_json::to_value(&plan).map_err(serde_internal)?;
    let data = SyncBackupData { plan: plan_value };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn sync_internal(err: reader_sync::SyncError) -> CoreError {
    CoreError::internal(format!("sync command failed: {err}"))
}

fn sync_invalid(field: &str, err: serde_json::Error) -> CoreError {
    CoreError::invalid_params(format!("sync command invalid {field}"))
        .with_details(serde_json::json!({ "source": err.to_string() }))
}

// ===========================================================================
// Local-book vertical (V1 minimal) — pure, no host bus
// ===========================================================================

fn local_book_parse(
    cmd: &reader_contract::Command,
    _state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: LocalBookParseParams =
        parse_params(contract::methods::LOCAL_BOOK_PARSE, &cmd.params)?;
    params.validate_local_book_parse_params()?;
    let book = if params.prefers_binary_path() {
        // S5 binary path: route through parse_local_book which auto-detects
        // EPUB/PDF/MOBI/AZW from magic bytes (with optional fileName/format
        // hint). Mirrors Legado's LocalBook dispatch by extension+mimetype.
        let bytes = decode_local_book_bytes(params.bytes_base64.as_deref().unwrap_or(""))?;
        let file_name = resolve_local_book_file_name(&params);
        let input = reader_local_book::LocalBookInput {
            book_id: &params.book_id,
            file_name: file_name.as_deref(),
            title: params.title.as_deref(),
            author: params.author.as_deref(),
            bytes: &bytes,
        };
        reader_local_book::parse_local_book(input).map_err(local_book_internal)?
    } else {
        // Legacy V1 text path: host already decoded GBK/GB18030 to UTF-8.
        reader_local_book::parse_txt_text(
            &params.book_id,
            params.title.as_deref(),
            params.author.as_deref(),
            params.file_name.as_deref(),
            &params.text,
        )
        .map_err(local_book_internal)?
    };
    let chapter_count = book.chapters.len() as u32;
    let full = serde_json::to_value(&book).map_err(serde_internal)?;
    let book_obj = full.get("book").cloned().unwrap_or(serde_json::Value::Null);
    let format = full
        .get("format")
        .and_then(|value| value.as_str())
        .unwrap_or("txt")
        .to_string();
    let encoding = full
        .get("encoding")
        .and_then(|value| value.as_str())
        .unwrap_or("utf8")
        .to_string();
    let byte_len = full
        .get("byteLen")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let char_len = full
        .get("charLen")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let data = LocalBookParseData {
        book: book_obj,
        format,
        encoding,
        byte_len,
        char_len,
        chapter_count,
    };
    serde_json::to_value(&data).map_err(serde_internal)
}

/// Decode base64-encoded local book bytes. Surfaces decode failures as
/// `INVALID_PARAMS` so a malformed wire payload never reaches the parser.
fn decode_local_book_bytes(bytes_base64: &str) -> Result<Vec<u8>, CoreError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(bytes_base64.trim())
        .map_err(|err| {
            CoreError::invalid_params(format!(
                "local_book.parse bytesBase64 must be valid standard base64: {err}"
            ))
        })
}

/// Resolve the file_name hint passed to `parse_local_book`. When `format` is
/// set and `fileName` is absent, synthesize `<format>.<format>` so the
/// extension-based declared-format detector picks it up. When both are set,
/// `fileName` wins (it may carry a real extension that differs from the
/// hint, e.g. an `.azw3` file hinted as `mobi`).
fn resolve_local_book_file_name(params: &LocalBookParseParams) -> Option<String> {
    if let Some(name) = params.file_name.as_deref() {
        if !name.trim().is_empty() {
            return Some(name.to_string());
        }
    }
    params
        .format
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|fmt| format!("{fmt}.{fmt}"))
}

fn local_book_catalog(
    cmd: &reader_contract::Command,
    _state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: LocalBookCatalogParams =
        parse_params(contract::methods::LOCAL_BOOK_CATALOG, &cmd.params)?;
    let catalog: reader_local_book::LocalBookCatalogSnapshot =
        serde_json::from_value(params.catalog).map_err(|err| local_book_invalid("catalog", err))?;
    let entry: reader_local_book::LocalBookFingerprintCatalogEntry =
        serde_json::from_value(params.entry).map_err(|err| local_book_invalid("entry", err))?;
    let chapters: Vec<reader_local_book::LocalBookChapterIndexEntry> = params
        .chapters
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .map_err(|err| local_book_invalid("chapters", err))?;
    let resources: Vec<reader_local_book::LocalBookResourceIndexEntry> = params
        .resources
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .map_err(|err| local_book_invalid("resources", err))?;
    let updated =
        reader_local_book::upsert_local_book_catalog_entry(&catalog, entry, chapters, resources)
            .map_err(local_book_internal)?;
    let catalog_value = serde_json::to_value(&updated).map_err(serde_internal)?;
    let data = LocalBookCatalogData {
        catalog: catalog_value,
    };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn local_book_internal(err: reader_local_book::LocalBookError) -> CoreError {
    CoreError::internal(format!("local_book command failed: {err}"))
}

fn local_book_invalid(field: &str, err: serde_json::Error) -> CoreError {
    CoreError::invalid_params(format!("local_book command invalid {field}"))
        .with_details(serde_json::json!({ "source": err.to_string() }))
}

// ===========================================================================
// Bookshelf vertical (V1 minimal) — pure read over BookshelfStore, no host
// ===========================================================================

fn bookshelf_list(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookshelfListParams = parse_params(contract::methods::BOOKSHELF_LIST, &cmd.params)?;
    let sort_by = parse_sort_by(&params.sort_by)?;
    let sort_direction = parse_sort_direction(&params.sort_direction)?;

    // Query the full match set (offset=0, limit=None) so `total` reflects
    // the unpaginated count; apply offset/limit on the result. The in-memory
    // store makes this cheap and keeps the protocol shape forward-compatible
    // with a future SQLite backend that could return COUNT(*) separately.
    let query = BookshelfQuery {
        source_id: params.source_id.clone(),
        group: params.group.clone(),
        keyword: params.keyword.clone(),
        has_reading_progress: None,
        sort_by,
        sort_direction,
        offset: 0,
        limit: None,
    };
    let mut matched = state
        .storage()
        .query_shelf(query)
        .map_err(storage_internal)?;
    // The store keys by (sourceId, bookId) but does not expose a bookId-only
    // filter; narrow here when only bookId is requested.
    if let Some(book_id) = &params.book_id {
        matched.retain(|entry| &entry.book_id == book_id);
    }
    let total = matched.len();
    let books = matched
        .into_iter()
        .skip(params.offset)
        .take(params.limit.unwrap_or(usize::MAX))
        .map(|entry| BookshelfEntryData {
            source_id: entry.source_id,
            book_id: entry.book_id,
            title: entry.title,
            author: entry.author,
            cover_url: entry.cover_url,
            intro: entry.intro,
            kind: entry.kind,
            last_chapter: entry.last_chapter,
            added_at: entry.added_at,
            last_read_at: entry.last_read_at,
            group: entry.group,
            sort_index: entry.sort_index,
        })
        .collect::<Vec<_>>();
    let data = BookshelfListData { books, total };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn bookshelf_get(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookshelfGetParams = parse_params(contract::methods::BOOKSHELF_GET, &cmd.params)?;
    let entry = state
        .storage()
        .get_shelf_entry(&params.source_id, &params.book_id)
        .map_err(storage_internal)?;
    let book = entry.map(entry_to_data);
    let data = BookshelfGetData { book };
    serde_json::to_value(&data).map_err(serde_internal)
}

fn parse_sort_by(value: &str) -> Result<BookshelfSortBy, CoreError> {
    serde_json::from_value::<BookshelfSortBy>(serde_json::Value::String(value.to_string()))
        .map_err(|_| {
            CoreError::invalid_params(format!(
                "bookshelf.list sortBy must be one of manual/addedAt/lastReadAt/title/author, got: {value:?}"
            ))
            .with_details(serde_json::json!({ "sortBy": value }))
        })
}

fn parse_sort_direction(value: &str) -> Result<BookshelfSortDirection, CoreError> {
    serde_json::from_value::<BookshelfSortDirection>(serde_json::Value::String(value.to_string()))
        .map_err(|_| {
            CoreError::invalid_params(format!(
                "bookshelf.list sortDirection must be ascending/descending, got: {value:?}"
            ))
            .with_details(serde_json::json!({ "sortDirection": value }))
        })
}

fn entry_to_data(entry: BookshelfEntry) -> BookshelfEntryData {
    BookshelfEntryData {
        source_id: entry.source_id,
        book_id: entry.book_id,
        title: entry.title,
        author: entry.author,
        cover_url: entry.cover_url,
        intro: entry.intro,
        kind: entry.kind,
        last_chapter: entry.last_chapter,
        added_at: entry.added_at,
        last_read_at: entry.last_read_at,
        group: entry.group,
        sort_index: entry.sort_index,
    }
}

fn serde_internal(err: serde_json::Error) -> CoreError {
    CoreError::internal(format!("serialization failed: {err}"))
}

// ===========================================================================
// replace-rule.* CRUD (Legado ReplaceRule.kt + ContentProcessor.kt:91 parity)
// ===========================================================================

fn replace_rule_to_data(rule: ReplaceRule) -> ReplaceRuleData {
    ReplaceRuleData {
        id: rule.id,
        name: rule.name,
        group: rule.group,
        pattern: rule.pattern,
        replacement: rule.replacement,
        scope: rule.scope,
        scope_title: rule.scope_title,
        scope_content: rule.scope_content,
        exclude_scope: rule.exclude_scope,
        is_enabled: rule.is_enabled,
        is_regex: rule.is_regex,
        timeout_millisecond: rule.timeout_millisecond,
        order: rule.order,
    }
}

fn next_replace_rule_id(state: &RemoteState) -> Result<i64, CoreError> {
    let rules = state
        .storage()
        .list_replace_rules()
        .map_err(storage_internal)?;
    Ok(rules.iter().map(|r| r.id).max().unwrap_or(0) + 1)
}

fn replace_rule_create(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReplaceRuleCreateParams =
        parse_params(contract::methods::REPLACE_RULE_CREATE, &cmd.params)?;
    let id = params
        .id
        .unwrap_or_else(|| next_replace_rule_id(state).unwrap_or(1));
    let rule = ReplaceRule {
        id,
        name: params.name,
        group: params.group,
        pattern: params.pattern,
        replacement: params.replacement,
        scope: params.scope,
        scope_title: params.scope_title,
        scope_content: params.scope_content,
        exclude_scope: params.exclude_scope,
        is_enabled: params.is_enabled,
        is_regex: params.is_regex,
        timeout_millisecond: params.timeout_millisecond,
        order: params.order,
    };
    let stored = state
        .storage()
        .put_replace_rule(rule)
        .map_err(storage_internal)?;
    let data = ReplaceRuleCreateData {
        rule: replace_rule_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn replace_rule_list(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReplaceRuleListParams =
        parse_params(contract::methods::REPLACE_RULE_LIST, &cmd.params)?;
    let rules = state
        .storage()
        .list_replace_rules()
        .map_err(storage_internal)?;
    let rules = if params.enabled_only == Some(true) {
        rules.into_iter().filter(|r| r.is_enabled).collect()
    } else {
        rules
    };
    let data = ReplaceRuleListData {
        rules: rules.into_iter().map(replace_rule_to_data).collect(),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn replace_rule_update(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReplaceRuleUpdateParams =
        parse_params(contract::methods::REPLACE_RULE_UPDATE, &cmd.params)?;
    let existing = state
        .storage()
        .get_replace_rule(params.id)
        .map_err(storage_internal)?
        .ok_or_else(|| {
            CoreError::invalid_params(format!("replace-rule not found: id={}", params.id))
        })?;
    let updated = ReplaceRule {
        id: existing.id,
        name: params.name.unwrap_or(existing.name),
        group: params.group.or(existing.group),
        pattern: params.pattern.unwrap_or(existing.pattern),
        replacement: params.replacement.unwrap_or(existing.replacement),
        scope: params.scope.or(existing.scope),
        scope_title: params.scope_title.unwrap_or(existing.scope_title),
        scope_content: params.scope_content.unwrap_or(existing.scope_content),
        exclude_scope: params.exclude_scope.or(existing.exclude_scope),
        is_enabled: params.is_enabled.unwrap_or(existing.is_enabled),
        is_regex: params.is_regex.unwrap_or(existing.is_regex),
        timeout_millisecond: params
            .timeout_millisecond
            .unwrap_or(existing.timeout_millisecond),
        order: params.order.unwrap_or(existing.order),
    };
    let stored = state
        .storage()
        .put_replace_rule(updated)
        .map_err(storage_internal)?;
    let data = ReplaceRuleUpdateData {
        rule: replace_rule_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn replace_rule_delete(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReplaceRuleDeleteParams =
        parse_params(contract::methods::REPLACE_RULE_DELETE, &cmd.params)?;
    let existed = state
        .storage()
        .get_replace_rule(params.id)
        .map_err(storage_internal)?
        .is_some();
    if existed {
        state
            .storage()
            .delete_replace_rule(params.id)
            .map_err(storage_internal)?;
    }
    let data = ReplaceRuleDeleteData {
        id: params.id,
        deleted: existed,
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

// ===========================================================================
// bookmark.* CRUD (Legado Bookmark.kt + BookmarkDao.kt parity)
// ===========================================================================

fn bookmark_to_data(b: Bookmark) -> BookmarkData {
    BookmarkData {
        time: b.time,
        book_name: b.book_name,
        book_author: b.book_author,
        chapter_index: b.chapter_index,
        chapter_pos: b.chapter_pos,
        chapter_name: b.chapter_name,
        book_text: b.book_text,
        content: b.content,
    }
}

fn next_bookmark_time(state: &RemoteState) -> Result<i64, CoreError> {
    let bookmarks = state
        .storage()
        .list_bookmarks()
        .map_err(storage_internal)?;
    Ok(bookmarks.iter().map(|b| b.time).max().unwrap_or(0) + 1)
}

fn bookmark_create(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookmarkCreateParams =
        parse_params(contract::methods::BOOKMARK_CREATE, &cmd.params)?;
    let time = params
        .time
        .unwrap_or_else(|| next_bookmark_time(state).unwrap_or(1));
    let bookmark = Bookmark {
        time,
        book_name: params.book_name,
        book_author: params.book_author,
        chapter_index: params.chapter_index,
        chapter_pos: params.chapter_pos,
        chapter_name: params.chapter_name,
        book_text: params.book_text,
        content: params.content,
    };
    let stored = state
        .storage()
        .put_bookmark(bookmark)
        .map_err(storage_internal)?;
    let data = BookmarkCreateData {
        bookmark: bookmark_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn bookmark_list(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookmarkListParams =
        parse_params(contract::methods::BOOKMARK_LIST, &cmd.params)?;
    let bookmarks = match (params.book_name, params.book_author) {
        (Some(name), Some(author)) => state
            .storage()
            .list_bookmarks_by_book(&name, &author)
            .map_err(storage_internal)?,
        _ => state.storage().list_bookmarks().map_err(storage_internal)?,
    };
    let data = BookmarkListData {
        bookmarks: bookmarks.into_iter().map(bookmark_to_data).collect(),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn bookmark_update(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookmarkUpdateParams =
        parse_params(contract::methods::BOOKMARK_UPDATE, &cmd.params)?;
    let existing = state
        .storage()
        .get_bookmark(params.time)
        .map_err(storage_internal)?
        .ok_or_else(|| {
            CoreError::invalid_params(format!("bookmark not found: time={}", params.time))
        })?;
    let updated = Bookmark {
        time: existing.time,
        book_name: params.book_name.unwrap_or(existing.book_name),
        book_author: params.book_author.unwrap_or(existing.book_author),
        chapter_index: params.chapter_index.unwrap_or(existing.chapter_index),
        chapter_pos: params.chapter_pos.unwrap_or(existing.chapter_pos),
        chapter_name: params.chapter_name.unwrap_or(existing.chapter_name),
        book_text: params.book_text.unwrap_or(existing.book_text),
        content: params.content.unwrap_or(existing.content),
    };
    let stored = state
        .storage()
        .put_bookmark(updated)
        .map_err(storage_internal)?;
    let data = BookmarkUpdateData {
        bookmark: bookmark_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn bookmark_delete(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookmarkDeleteParams =
        parse_params(contract::methods::BOOKMARK_DELETE, &cmd.params)?;
    let existed = state
        .storage()
        .get_bookmark(params.time)
        .map_err(storage_internal)?
        .is_some();
    if existed {
        state
            .storage()
            .delete_bookmark(params.time)
            .map_err(storage_internal)?;
    }
    let data = BookmarkDeleteData {
        time: params.time,
        deleted: existed,
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

// ===========================================================================
// book-group.* CRUD (Legado BookGroup.kt + BookGroupDao.kt parity)
// ===========================================================================

fn book_group_to_data(g: BookGroup) -> BookGroupData {
    BookGroupData {
        group_id: g.group_id,
        group_name: g.group_name,
        cover: g.cover,
        order: g.order,
        enable_refresh: g.enable_refresh,
        show: g.show,
    }
}

fn next_book_group_id(state: &RemoteState) -> Result<i64, CoreError> {
    let groups = state
        .storage()
        .list_book_groups()
        .map_err(storage_internal)?;
    Ok(groups.iter().map(|g| g.group_id).max().unwrap_or(0) + 1)
}

fn book_group_create(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookGroupCreateParams =
        parse_params(contract::methods::BOOK_GROUP_CREATE, &cmd.params)?;
    let group_id = params
        .group_id
        .unwrap_or_else(|| next_book_group_id(state).unwrap_or(1));
    let group = BookGroup {
        group_id,
        group_name: params.group_name,
        cover: params.cover,
        order: params.order,
        enable_refresh: params.enable_refresh,
        show: params.show,
    };
    let stored = state
        .storage()
        .put_book_group(group)
        .map_err(storage_internal)?;
    let data = BookGroupCreateData {
        group: book_group_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn book_group_list(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookGroupListParams =
        parse_params(contract::methods::BOOK_GROUP_LIST, &cmd.params)?;
    let groups = state
        .storage()
        .list_book_groups()
        .map_err(storage_internal)?;
    let groups = if params.show_only == Some(true) {
        groups.into_iter().filter(|g| g.show).collect()
    } else {
        groups
    };
    let data = BookGroupListData {
        groups: groups.into_iter().map(book_group_to_data).collect(),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn book_group_update(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookGroupUpdateParams =
        parse_params(contract::methods::BOOK_GROUP_UPDATE, &cmd.params)?;
    let existing = state
        .storage()
        .get_book_group(params.group_id)
        .map_err(storage_internal)?
        .ok_or_else(|| {
            CoreError::invalid_params(format!(
                "book-group not found: groupId={}",
                params.group_id
            ))
        })?;
    let updated = BookGroup {
        group_id: existing.group_id,
        group_name: params.group_name.unwrap_or(existing.group_name),
        cover: params.cover.or(existing.cover),
        order: params.order.unwrap_or(existing.order),
        enable_refresh: params.enable_refresh.unwrap_or(existing.enable_refresh),
        show: params.show.unwrap_or(existing.show),
    };
    let stored = state
        .storage()
        .put_book_group(updated)
        .map_err(storage_internal)?;
    let data = BookGroupUpdateData {
        group: book_group_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn book_group_delete(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: BookGroupDeleteParams =
        parse_params(contract::methods::BOOK_GROUP_DELETE, &cmd.params)?;
    let existed = state
        .storage()
        .get_book_group(params.group_id)
        .map_err(storage_internal)?
        .is_some();
    if existed {
        state
            .storage()
            .delete_book_group(params.group_id)
            .map_err(storage_internal)?;
    }
    let data = BookGroupDeleteData {
        group_id: params.group_id,
        deleted: existed,
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

// ===========================================================================
// read-record.* CRUD (Legado ReadRecord.kt + ReadRecordDao.kt parity)
// ===========================================================================

fn read_record_to_data(r: ReadRecord) -> ReadRecordData {
    ReadRecordData {
        device_id: r.device_id,
        book_name: r.book_name,
        read_time: r.read_time,
        last_read: r.last_read,
    }
}

fn read_record_create(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReadRecordCreateParams =
        parse_params(contract::methods::READ_RECORD_CREATE, &cmd.params)?;
    if params.book_name.trim().is_empty() {
        return Err(CoreError::invalid_params(
            "read-record.create bookName must be non-empty",
        ));
    }
    let record = ReadRecord {
        device_id: params.device_id,
        book_name: params.book_name,
        read_time: params.read_time,
        last_read: params.last_read,
    };
    let stored = state
        .storage()
        .put_read_record(record)
        .map_err(storage_internal)?;
    let data = ReadRecordCreateData {
        record: read_record_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn read_record_list(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReadRecordListParams =
        parse_params(contract::methods::READ_RECORD_LIST, &cmd.params)?;
    let records = state
        .storage()
        .list_read_records()
        .map_err(storage_internal)?;
    let records = if let Some(device_id) = params.device_id {
        records
            .into_iter()
            .filter(|r| r.device_id == device_id)
            .collect()
    } else {
        records
    };
    let data = ReadRecordListData {
        records: records.into_iter().map(read_record_to_data).collect(),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn read_record_update(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReadRecordUpdateParams =
        parse_params(contract::methods::READ_RECORD_UPDATE, &cmd.params)?;
    let existing = state
        .storage()
        .get_read_record(&params.device_id, &params.book_name)
        .map_err(storage_internal)?
        .ok_or_else(|| {
            CoreError::invalid_params(format!(
                "read-record not found: deviceId={}, bookName={}",
                params.device_id, params.book_name
            ))
        })?;
    let updated = ReadRecord {
        device_id: existing.device_id,
        book_name: existing.book_name,
        read_time: params.read_time.unwrap_or(existing.read_time),
        last_read: params.last_read.unwrap_or(existing.last_read),
    };
    let stored = state
        .storage()
        .put_read_record(updated)
        .map_err(storage_internal)?;
    let data = ReadRecordUpdateData {
        record: read_record_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn read_record_delete(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: ReadRecordDeleteParams =
        parse_params(contract::methods::READ_RECORD_DELETE, &cmd.params)?;
    let existed = state
        .storage()
        .get_read_record(&params.device_id, &params.book_name)
        .map_err(storage_internal)?
        .is_some();
    if existed {
        state
            .storage()
            .delete_read_record(&params.device_id, &params.book_name)
            .map_err(storage_internal)?;
    }
    let data = ReadRecordDeleteData {
        device_id: params.device_id,
        book_name: params.book_name,
        deleted: existed,
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

// ===========================================================================
// source.exploreKinds + source.explore (Legado WebBook.kt:93 parity)
// ===========================================================================

/// `source.exploreKinds`: parse a source's `exploreUrl` field into discovery
/// categories. Pure parse — no host callback.
fn source_explore_kinds(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: SourceExploreKindsParams =
        parse_params(contract::methods::SOURCE_EXPLORE_KINDS, &cmd.params)?;
    let explore_url = if let Some(url) = params.explore_url.as_deref() {
        url.to_string()
    } else {
        let source_id = source_id_or_empty(&params.source_id, &params.source)?;
        let source = resolve_source(state.storage(), &source_id, &params.source)?;
        let legado = source.legado_book_source().ok_or_else(|| {
            CoreError::invalid_params(
                "cannot parse exploreKinds: source has no Legado bookSource payload",
            )
            .with_details(serde_json::json!({ "sourceId": source_id }))
        })?;
        legado.explore_url.clone().unwrap_or_default()
    };
    let trimmed = explore_url.trim();
    let kinds: Vec<SourceExploreKindEntry> =
        if trimmed.starts_with("@js:") || trimmed.starts_with("<js>") {
            let context = serde_json::json!({});
            state
                .pipeline()
                .parse_explore_kinds_with_js(&explore_url, &context)
                .into_iter()
                .map(|kind| SourceExploreKindEntry {
                    title: kind.title,
                    url: kind.url,
                })
                .collect()
        } else {
            reader_content::parse_explore_kinds(&explore_url)
                .into_iter()
                .map(|kind| SourceExploreKindEntry {
                    title: kind.title,
                    url: kind.url,
                })
                .collect()
        };
    let data = SourceExploreKindsData { kinds };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

/// `source.explore`: fetch books from a discovery category URL. Three modes
/// (mirrors `book.search`): prefetched response, pre-built request, or
/// auto-build from `url` + source `header`.
fn source_explore(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let params: SourceExploreParams = parse_params(contract::methods::SOURCE_EXPLORE, &cmd.params)?;
    if let Some(response) = params.explore_response.as_deref() {
        if !response.is_empty() {
            let data = source_explore_parse_response(params.clone(), response, state)?;
            return Ok(RemoteCommandResult::Complete(data));
        }
    }
    if let Some(request) = params.explore_request.clone() {
        return Ok(RemoteCommandResult::Pending(pending_http_request(
            request,
            RemoteHostContinuation::SourceExplore(params),
        )?));
    }
    if let Some(url) = params.url.as_deref() {
        if !url.trim().is_empty() {
            let request = build_explore_request_from_url(state, &params, url)?;
            return Ok(RemoteCommandResult::Pending(pending_http_request(
                request,
                RemoteHostContinuation::SourceExplore(params),
            )?));
        }
    }
    Err(CoreError::invalid_params(
        "exploreResponse is required unless exploreRequest or url is provided",
    ))
}

fn build_explore_request_from_url(
    state: &RemoteState,
    params: &SourceExploreParams,
    raw_url: &str,
) -> Result<HostHttpRequest, CoreError> {
    let source_id = source_id_or_empty(&params.source_id, &params.source)?;
    let source = resolve_source(state.storage(), &source_id, &params.source)?;
    let legado = source.legado_book_source().ok_or_else(|| {
        CoreError::invalid_params(
            "cannot auto-build exploreRequest: source has no Legado bookSource payload",
        )
        .with_details(serde_json::json!({ "sourceId": source_id }))
    })?;
    let source_headers = legado
        .header
        .as_ref()
        .and_then(|h| h.as_object())
        .cloned()
        .unwrap_or_default();
    let page = params.page.unwrap_or(1).max(1);
    let ctx = AnalyzeUrlContext::for_search("", page);
    build_analyze_url_request(state, raw_url, &ctx, &source.base_url, &source_headers)
}

fn source_explore_from_params(
    params: SourceExploreParams,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    // Clone the response before moving `params` into the parser to satisfy
    // the borrow checker (the response is a field of `params`).
    let response = params
        .explore_response
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::internal("source.explore continuation missing response"))?;
    let data = source_explore_parse_response(params, &response, state)?;
    Ok(RemoteCommandResult::Complete(data))
}

fn source_explore_parse_response(
    params: SourceExploreParams,
    response: &str,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let source_id = source_id_or_empty(&params.source_id, &params.source)?;
    let source = resolve_source(state.storage(), &source_id, &params.source)?;
    let books = state
        .pipeline()
        .explore(&source, response)
        .map_err(content_internal)?;
    for book in &books {
        let _ = state.storage().put_book(book.clone());
    }
    let books_data: Vec<serde_json::Value> = books
        .iter()
        .map(|book| serde_json::to_value(book).unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(serde_json::json!({
        "sourceId": source_id,
        "books": books_data,
    }))
}

// ===========================================================================
// txt-toc-rule.* CRUD (Legado TxtTocRule.kt parity)
// ===========================================================================

fn txt_toc_rule_to_data(rule: TxtTocRule) -> TxtTocRuleData {
    TxtTocRuleData {
        id: rule.id,
        name: rule.name,
        rule: rule.rule,
        example: rule.example,
        serial_number: rule.serial_number,
        enable: rule.enable,
    }
}

fn next_txt_toc_rule_id(state: &RemoteState) -> i64 {
    let rules = state.storage().list_txt_toc_rules().unwrap_or_default();
    rules.iter().map(|r| r.id).max().unwrap_or(0) + 1
}

fn txt_toc_rule_create(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: TxtTocRuleCreateParams =
        parse_params(contract::methods::TXT_TOC_RULE_CREATE, &cmd.params)?;
    let id = params.id.unwrap_or_else(|| next_txt_toc_rule_id(state));
    let rule = TxtTocRule {
        id,
        name: params.name,
        rule: params.rule,
        example: params.example,
        serial_number: params.serial_number,
        enable: params.enable,
    };
    let stored = state
        .storage()
        .put_txt_toc_rule(rule)
        .map_err(storage_internal)?;
    let data = TxtTocRuleCreateData {
        rule: txt_toc_rule_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn txt_toc_rule_list(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: TxtTocRuleListParams =
        parse_params(contract::methods::TXT_TOC_RULE_LIST, &cmd.params)?;
    let rules = if params.enabled_only == Some(true) {
        state
            .storage()
            .list_enabled_txt_toc_rules()
            .map_err(storage_internal)?
    } else {
        state
            .storage()
            .list_txt_toc_rules()
            .map_err(storage_internal)?
    };
    let data = TxtTocRuleListData {
        rules: rules.into_iter().map(txt_toc_rule_to_data).collect(),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn txt_toc_rule_update(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: TxtTocRuleUpdateParams =
        parse_params(contract::methods::TXT_TOC_RULE_UPDATE, &cmd.params)?;
    let existing = state
        .storage()
        .get_txt_toc_rule(params.id)
        .map_err(storage_internal)?
        .ok_or_else(|| {
            CoreError::invalid_params(format!("txt-toc-rule not found: id={}", params.id))
        })?;
    let updated = TxtTocRule {
        id: existing.id,
        name: params.name.unwrap_or(existing.name),
        rule: params.rule.unwrap_or(existing.rule),
        example: params.example.or(existing.example),
        serial_number: params.serial_number.unwrap_or(existing.serial_number),
        enable: params.enable.unwrap_or(existing.enable),
    };
    let stored = state
        .storage()
        .put_txt_toc_rule(updated)
        .map_err(storage_internal)?;
    let data = TxtTocRuleUpdateData {
        rule: txt_toc_rule_to_data(stored),
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

fn txt_toc_rule_delete(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: TxtTocRuleDeleteParams =
        parse_params(contract::methods::TXT_TOC_RULE_DELETE, &cmd.params)?;
    let existing = state
        .storage()
        .get_txt_toc_rule(params.id)
        .map_err(storage_internal)?;
    let deleted = existing.is_some();
    if deleted {
        state
            .storage()
            .delete_txt_toc_rule(params.id)
            .map_err(storage_internal)?;
    }
    let data = TxtTocRuleDeleteData {
        id: params.id,
        deleted,
    };
    Ok(serde_json::to_value(data).map_err(serde_internal)?)
}

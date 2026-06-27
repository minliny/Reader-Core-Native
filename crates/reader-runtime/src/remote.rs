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
        parse_params, BookDetailParams, BookSearchParams, BookTocParams, ChapterContentParams,
        HostHttpRequest, HostHttpResponse, LocalBookCatalogData, LocalBookCatalogParams,
        LocalBookParseData, LocalBookParseParams, ReadingProgressUpdateParams, RssParseData,
        RssParseEntryData, RssParseParams, RssRefreshData, RssRefreshParams, SourceImportParams,
        SyncBackupData, SyncBackupParams, SyncMergeData, SyncMergeParams,
    },
    CoreError, Event, HostCapability,
};
use reader_domain::{Book, ReadingProgress, Source, SourceRules};
use reader_storage::InMemoryStorage;

use crate::sink::EventSink;

/// Shared remote-reading state held by the runtime: the content pipeline and
/// the in-memory storage. The active-request registry is owned by the runtime
/// and passed in at dispatch time so remote handlers reuse the same tracking as
/// the built-in commands.
#[derive(Clone)]
pub struct RemoteState {
    pipeline: Arc<RemoteContentPipeline>,
    storage: Arc<InMemoryStorage>,
}

impl RemoteState {
    /// Create fresh state with default pipeline + storage.
    pub fn new() -> Self {
        Self {
            pipeline: Arc::new(RemoteContentPipeline::new()),
            storage: Arc::new(InMemoryStorage::new()),
        }
    }

    pub fn pipeline(&self) -> &RemoteContentPipeline {
        &self.pipeline
    }

    pub fn storage(&self) -> &InMemoryStorage {
        &self.storage
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
enum RemoteCommandResult {
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

/// Continue a remote-reading command after its host HTTP request completes.
pub fn complete_remote_host(
    continuation: RemoteHostContinuation,
    host_result: serde_json::Value,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let response = parse_http_response(host_result)?;
    let diagnostics = http_response_diagnostics(&response);
    match continuation {
        RemoteHostContinuation::BookSearch(mut params) => {
            params.search_response = response.body;
            book_search_from_params(params, state)
                .map(|data| with_http_diagnostics(data, diagnostics))
        }
        RemoteHostContinuation::BookDetail(mut params) => {
            params.detail_response = response.body;
            book_detail_from_params(params, state)
                .map(|data| with_http_diagnostics(data, diagnostics))
        }
        RemoteHostContinuation::BookToc(mut params) => {
            params.toc_response = response.body;
            book_toc_from_params(params, state).map(|data| with_http_diagnostics(data, diagnostics))
        }
        RemoteHostContinuation::ChapterContent(mut params) => {
            params.chapter_response = response.body;
            chapter_content_from_params(params, state)
                .map(|data| with_http_diagnostics(data, diagnostics))
        }
    }
}

fn source_import(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<serde_json::Value, CoreError> {
    let params: SourceImportParams = parse_params(contract::methods::SOURCE_IMPORT, &cmd.params)?;
    if params.name.trim().is_empty() {
        return Err(CoreError::invalid_params(
            "source.import requires a non-empty name",
        ));
    }
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
        name: params.name,
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
    build_analyze_url_request(state, search_url, &ctx, &source.base_url, &source_headers)
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
        return book_toc_from_params(params, state).map(RemoteCommandResult::Complete);
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
) -> Result<serde_json::Value, CoreError> {
    let source = resolve_source(state.storage(), &params.source_id, &params.source)?;
    let toc = state
        .pipeline()
        .toc(&source, &params.toc_response)
        .map_err(content_internal)?;
    let cache_key = format!("toc:{}", params.book_id);
    let payload = serde_json::to_string(&toc).unwrap_or_else(|_| "[]".into());
    let _ = state.storage().put_cache(cache_key, payload);
    Ok(serde_json::json!({
        "sourceId": params.source_id,
        "bookId": params.book_id,
        "toc": toc,
    }))
}

fn chapter_content(
    cmd: &reader_contract::Command,
    state: &RemoteState,
) -> Result<RemoteCommandResult, CoreError> {
    let params: ChapterContentParams =
        parse_params(contract::methods::CHAPTER_CONTENT, &cmd.params)?;
    // JS-rule path: skip auto-build entirely (existing behaviour).
    if params.js_rule.is_some() {
        return chapter_content_from_params(params, state).map(RemoteCommandResult::Complete);
    }
    if !params.chapter_response.is_empty() {
        return chapter_content_from_params(params, state).map(RemoteCommandResult::Complete);
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
) -> Result<serde_json::Value, CoreError> {
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
                return Ok(serde_json::json!({
                    "sourceId": params.source_id,
                    "bookId": params.book_id,
                    "chapterTitle": params.chapter_title,
                    "content": value,
                    "via": "js",
                }));
            }
            JsOutcome::Unsupported { reason } => {
                return Err(content_internal(
                    reader_content::ContentError::JsUnsupported { reason },
                ));
            }
        }
    }

    let content = pipeline
        .chapter_content(&source, &params.chapter_response)
        .map_err(content_internal)?;

    let cache_key = format!("chapter:{}:{}", params.book_id, params.chapter_title);
    let _ = state.storage().put_cache(cache_key, content.clone());

    Ok(serde_json::json!({
        "sourceId": params.source_id,
        "bookId": params.book_id,
        "chapterTitle": params.chapter_title,
        "content": content,
        "via": "rule",
    }))
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

fn serde_internal(err: serde_json::Error) -> CoreError {
    CoreError::internal(format!("serialization failed: {err}"))
}

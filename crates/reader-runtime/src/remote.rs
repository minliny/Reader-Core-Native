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

use reader_content::{JsOutcome, RemoteContentPipeline};
use reader_contract::{
    self as contract,
    remote::{
        parse_params, BookDetailParams, BookSearchParams, BookTocParams, ChapterContentParams,
        HostHttpRequest, HostHttpResponse, ReadingProgressUpdateParams, SourceImportParams,
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
pub enum RemoteDispatch {
    NotHandled,
    Finished,
    Pending(PendingHostRequest),
}

enum RemoteCommandResult {
    Complete(serde_json::Value),
    Pending(PendingHostRequest),
}

/// A host capability request that must complete before the original remote
/// command can finish.
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

fn pending_or_missing_response(
    response: &str,
    request: Option<HostHttpRequest>,
    response_field: &str,
    request_field: &str,
    continuation: RemoteHostContinuation,
) -> Result<Option<RemoteCommandResult>, CoreError> {
    if !response.is_empty() {
        return Ok(None);
    }
    let Some(request) = request else {
        return Err(CoreError::invalid_params(format!(
            "{response_field} is required unless {request_field} is provided"
        )));
    };
    Ok(Some(RemoteCommandResult::Pending(pending_http_request(
        request,
        continuation,
    )?)))
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
    if let Some(pending) = pending_or_missing_response(
        &params.search_response,
        params.search_request.clone(),
        "searchResponse",
        "searchRequest",
        RemoteHostContinuation::BookSearch(params.clone()),
    )? {
        return Ok(pending);
    }
    book_search_from_params(params, state).map(RemoteCommandResult::Complete)
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
    if let Some(pending) = pending_or_missing_response(
        &params.detail_response,
        params.detail_request.clone(),
        "detailResponse",
        "detailRequest",
        RemoteHostContinuation::BookDetail(params.clone()),
    )? {
        return Ok(pending);
    }
    book_detail_from_params(params, state).map(RemoteCommandResult::Complete)
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
    if let Some(pending) = pending_or_missing_response(
        &params.toc_response,
        params.toc_request.clone(),
        "tocResponse",
        "tocRequest",
        RemoteHostContinuation::BookToc(params.clone()),
    )? {
        return Ok(pending);
    }
    book_toc_from_params(params, state).map(RemoteCommandResult::Complete)
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
    if params.js_rule.is_none() {
        if let Some(pending) = pending_or_missing_response(
            &params.chapter_response,
            params.chapter_request.clone(),
            "chapterResponse",
            "chapterRequest",
            RemoteHostContinuation::ChapterContent(params.clone()),
        )? {
            return Ok(pending);
        }
    }
    chapter_content_from_params(params, state).map(RemoteCommandResult::Complete)
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

//! Remote-reading vertical command handlers (V1 minimal).
//!
//! These commands implement the import → search → detail → toc → chapter →
//! progress pipeline over inline/fixture content. They do **not** perform live
//! network I/O: the host is expected to supply pre-fetched response bodies. A
//! JS rule that calls a host capability (`java.get`/`java.post`) without a
//! registered callback yields a structured `unsupported` error.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use reader_content::{JsOutcome, RemoteContentPipeline};
use reader_contract::{
    self as contract,
    remote::{
        parse_params, BookDetailParams, BookSearchParams, BookTocParams, ChapterContentParams,
        ReadingProgressUpdateParams, SourceImportParams,
    },
    CoreError, Event,
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
) -> bool {
    let request_id = cmd.request_id;
    let result: Result<serde_json::Value, CoreError> = match method {
        contract::methods::SOURCE_IMPORT => source_import(cmd, state),
        contract::methods::BOOK_SEARCH => book_search(cmd, state),
        contract::methods::BOOK_DETAIL => book_detail(cmd, state),
        contract::methods::BOOK_TOC => book_toc(cmd, state),
        contract::methods::CHAPTER_CONTENT => chapter_content(cmd, state),
        contract::methods::READING_PROGRESS_UPDATE => reading_progress_update(cmd, state),
        _ => return false,
    };
    let event = match result {
        Ok(data) => Event::result(request_id, data),
        Err(err) => error_event(request_id, err),
    };
    finish(sink, active_requests, request_id, event);
    true
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
) -> Result<serde_json::Value, CoreError> {
    let params: BookSearchParams = parse_params(contract::methods::BOOK_SEARCH, &cmd.params)?;
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
) -> Result<serde_json::Value, CoreError> {
    let params: BookDetailParams = parse_params(contract::methods::BOOK_DETAIL, &cmd.params)?;
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
) -> Result<serde_json::Value, CoreError> {
    let params: BookTocParams = parse_params(contract::methods::BOOK_TOC, &cmd.params)?;
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
) -> Result<serde_json::Value, CoreError> {
    let params: ChapterContentParams =
        parse_params(contract::methods::CHAPTER_CONTENT, &cmd.params)?;
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

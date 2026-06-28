//! `--test-source` / `--test-corpus` — 真实书源 L1-L5 链式测试.
//!
//! 本模块负责:
//! - 把单个 raw Legado 书源 JSON 跑通 import → search → detail → toc → content
//! - 真实 HTTP 由 CLI 充当 Host 执行(Core 发出 http.execute,CLI 用 ureq 拉取)
//! - 支持录像(--record):保存每步 HTTP 响应到磁盘
//! - 支持离线回放(--offline / --test-corpus-offline):用录像数据代替真实 HTTP

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reader_contract::{methods, Command, CoreError, Event, HostCapability};
use reader_runtime::{EventSink, Runtime};
use serde_json::{json, Value};

/// 每个 source 的录像条目(每个 HTTP 步骤一条).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordedStep {
    pub level: String, // "L2-search" / "L3-detail" / "L4-toc" / "L5-content"
    pub url: String,
    pub method: String,
    pub request_headers: Value,
    pub request_body: Option<String>,
    pub response_status: u16,
    pub response_headers: Value,
    pub response_body: String,
    pub final_url: Option<String>,
}

/// 一个 source 的完整录像.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceRecording {
    pub source_id: String,
    pub source_name: String,
    pub recorded_at: String,
    pub keyword: String,
    pub steps: Vec<RecordedStep>,
}

/// 单步测试结果.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pass,
    Fail,
    Skip,
}

/// 单个 source 的 L1-L5 测试结果.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceTestResult {
    pub source_id: String,
    pub source_file: String,
    pub source_name: String,
    pub source_url: String,
    pub priority: Option<String>,
    pub rule_forms: Vec<String>,
    pub has_js: bool,
    pub has_multirule: bool,
    pub has_regex: bool,
    pub levels: BTreeMap<String, LevelResult>,
    pub failure_reason: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LevelResult {
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl LevelResult {
    fn pass() -> Self {
        Self {
            status: StepStatus::Pass,
            reason: None,
            detail: None,
        }
    }
    fn fail(reason: impl Into<String>) -> Self {
        Self {
            status: StepStatus::Fail,
            reason: Some(reason.into()),
            detail: None,
        }
    }
    fn fail_with(reason: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: StepStatus::Fail,
            reason: Some(reason.into()),
            detail: Some(detail.into()),
        }
    }
    fn skip(reason: impl Into<String>) -> Self {
        Self {
            status: StepStatus::Skip,
            reason: Some(reason.into()),
            detail: None,
        }
    }
}

/// 测试单个 source 的配置.
pub struct TestSourceConfig {
    pub source_path: PathBuf,
    pub keyword: String,
    pub timeout: Duration,
    /// 录像输出目录;若设置,则把每步 HTTP 响应保存到 `<dir>/<source_id>.json`.
    pub record_dir: Option<PathBuf>,
    /// 离线回放:若设置,从该目录读取 `<source_id>.json` 录像,不发真实 HTTP.
    pub offline_dir: Option<PathBuf>,
    /// 静默模式(批量测试时用),不打印单个 source 的进度.
    /// 当前 test_source 自身不打印,进度由 test_corpus 控制;保留字段供未来单源详情开关.
    #[allow(dead_code)]
    pub quiet: bool,
}

impl Default for TestSourceConfig {
    fn default() -> Self {
        Self {
            source_path: PathBuf::new(),
            keyword: String::new(),
            timeout: Duration::from_secs(15),
            record_dir: None,
            offline_dir: None,
            quiet: false,
        }
    }
}

struct ChannelSink {
    tx: std::sync::mpsc::Sender<Event>,
}

impl EventSink for ChannelSink {
    fn emit(&self, event: &Event) {
        let _ = self.tx.send(event.clone());
    }
}

/// HTTP 响应(Core 期望的 host.complete result 形态).
struct HostHttpResponse {
    status: u16,
    headers: Value,
    body: String,
    final_url: Option<String>,
}

/// 单个 source 的完整 L1-L5 测试入口.
///
/// 返回 `SourceTestResult`.即使中途某步失败,也会把已完成步骤的结果填入 `levels`,
/// 后续步骤标记为 `Skip`.
pub fn test_source(config: &TestSourceConfig) -> SourceTestResult {
    let start = Instant::now();
    let raw = match fs::read_to_string(&config.source_path) {
        Ok(s) => s,
        Err(err) => {
            return failed_result(
                &config.source_path,
                "",
                "",
                "L1-import",
                "read_source_file",
                &err.to_string(),
                start,
            );
        }
    };
    let source_json: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            return failed_result(
                &config.source_path,
                "",
                "",
                "L1-import",
                "parse_source_json",
                &err.to_string(),
                start,
            );
        }
    };

    let source_id = source_json
        .get("sourceId")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let source_name = source_json
        .get("bookSourceName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let source_url = source_json
        .get("bookSourceUrl")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mut result = SourceTestResult {
        source_id: source_id.clone(),
        source_file: config
            .source_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        source_name: source_name.clone(),
        source_url: source_url.clone(),
        priority: None,
        rule_forms: vec![],
        has_js: false,
        has_multirule: false,
        has_regex: false,
        levels: BTreeMap::new(),
        failure_reason: None,
        duration_ms: 0,
    };

    // 离线模式:先尝试加载录像
    let recording = if let Some(offline_dir) = &config.offline_dir {
        let rec_path = offline_dir.join(format!("{source_id}.json"));
        match fs::read_to_string(&rec_path) {
            Ok(s) => match serde_json::from_str::<SourceRecording>(&s) {
                Ok(r) => Some(r),
                Err(err) => {
                    result.levels.insert(
                        "L1-import".into(),
                        LevelResult::fail_with("recording_parse_error", err.to_string()),
                    );
                    result.failure_reason = Some("recording_parse_error".into());
                    result.duration_ms = start.elapsed().as_millis() as u64;
                    return result;
                }
            },
            Err(_) => {
                result
                    .levels
                    .insert("L1-import".into(), LevelResult::skip("no_recording"));
                result.failure_reason = Some("no_recording".into());
                result.duration_ms = start.elapsed().as_millis() as u64;
                return result;
            }
        }
    } else {
        None
    };

    // 构造 Runtime
    let (tx, rx) = std::sync::mpsc::channel();
    let sink = Arc::new(ChannelSink { tx });
    let runtime = Runtime::new(sink);

    // 构造 Source wrapper:rules 留空,全部规则走 bookSource
    let source_wrapper = json!({
        "sourceId": source_id,
        "name": source_name,
        "baseUrl": source_url,
        "rules": {},
        "bookSource": source_json,
    });

    let mut recording_steps: Vec<RecordedStep> = Vec::new();
    let mut next_request_id: u64 = 1;

    // L1: source.import
    let import_id = next_request_id;
    next_request_id += 1;
    if let Err(err) = runtime.send(Command::new(
        import_id,
        methods::SOURCE_IMPORT,
        json!({
            "sourceId": source_id,
            "name": source_name,
            "baseUrl": source_url,
            "rules": {},
            "bookSource": source_json,
        }),
    )) {
        result.levels.insert(
            "L1-import".into(),
            LevelResult::fail_with("send_error", core_error_string(&err)),
        );
        result.failure_reason = Some("send_error".into());
        result.duration_ms = start.elapsed().as_millis() as u64;
        return result;
    }
    match recv_with_timeout(&rx, config.timeout) {
        Ok(Event::Result { data, .. }) => {
            if data.get("imported").and_then(Value::as_bool) == Some(true) {
                result
                    .levels
                    .insert("L1-import".into(), LevelResult::pass());
            } else {
                result.levels.insert(
                    "L1-import".into(),
                    LevelResult::fail("import_not_confirmed"),
                );
                result.failure_reason = Some("import_not_confirmed".into());
                result.duration_ms = start.elapsed().as_millis() as u64;
                return result;
            }
        }
        Ok(Event::Error { error, .. }) => {
            result.levels.insert(
                "L1-import".into(),
                LevelResult::fail_with("import_error", core_error_string(&error)),
            );
            result.failure_reason = Some("import_error".into());
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }
        Ok(other) => {
            result.levels.insert(
                "L1-import".into(),
                LevelResult::fail_with("unexpected_event", format!("{other:?}")),
            );
            result.failure_reason = Some("unexpected_event".into());
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }
        Err(err) => {
            result.levels.insert(
                "L1-import".into(),
                LevelResult::fail_with("timeout_or_disconnected", err),
            );
            result.failure_reason = Some("timeout_or_disconnected".into());
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }
    }

    // L2: book.search
    let search_id = next_request_id;
    next_request_id += 1;
    let search_params = if let Some(rec) = &recording {
        // 离线模式:用录像的 L2 响应直接喂给 Core
        let l2 = rec.steps.iter().find(|s| s.level == "L2-search");
        json!({
            "sourceId": source_id,
            "source": source_wrapper,
            "searchResponse": l2.map(|s| s.response_body.as_str()).unwrap_or(""),
        })
    } else {
        json!({
            "sourceId": source_id,
            "source": source_wrapper,
            "keyword": config.keyword,
            "page": 1,
        })
    };
    let search_result = run_command_with_http(
        &runtime,
        &rx,
        search_id,
        methods::BOOK_SEARCH,
        search_params,
        config.timeout,
        &config.offline_dir,
        &recording,
        "L2-search",
        &mut recording_steps,
        &mut next_request_id,
    );

    let books = match search_result {
        CommandOutcome::Result(data) => {
            let books = data.get("books").cloned().unwrap_or(Value::Array(vec![]));
            if books.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                result
                    .levels
                    .insert("L2-search".into(), LevelResult::fail("no_search_results"));
                result.failure_reason = Some("no_search_results".into());
                skip_remaining(&mut result, &["L3-detail", "L4-toc", "L5-content"]);
                finalize(&mut result, &recording, &recording_steps, config, start);
                return result;
            }
            result
                .levels
                .insert("L2-search".into(), LevelResult::pass());
            books
        }
        CommandOutcome::Error { reason, detail } => {
            result.failure_reason = Some(reason.clone());
            result
                .levels
                .insert("L2-search".into(), LevelResult::fail_with(reason, detail));
            skip_remaining(&mut result, &["L3-detail", "L4-toc", "L5-content"]);
            finalize(&mut result, &recording, &recording_steps, config, start);
            return result;
        }
    };

    // 取第一本书
    let first_book = books.get(0).cloned().unwrap_or(Value::Null);
    let book_id = first_book
        .get("bookId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let book_url = first_book
        .get("bookUrl")
        .and_then(Value::as_str)
        .or_else(|| first_book.get("detailUrl").and_then(Value::as_str))
        .unwrap_or(&book_id)
        .to_string();

    // L3: book.detail
    let detail_id = next_request_id;
    next_request_id += 1;
    let detail_params = if let Some(rec) = &recording {
        let l3 = rec.steps.iter().find(|s| s.level == "L3-detail");
        json!({
            "sourceId": source_id,
            "book": first_book,
            "source": source_wrapper,
            "detailResponse": l3.map(|s| s.response_body.as_str()).unwrap_or(""),
        })
    } else {
        json!({
            "sourceId": source_id,
            "book": first_book,
            "source": source_wrapper,
            "bookUrl": book_url,
        })
    };
    let detail_result = run_command_with_http(
        &runtime,
        &rx,
        detail_id,
        methods::BOOK_DETAIL,
        detail_params,
        config.timeout,
        &config.offline_dir,
        &recording,
        "L3-detail",
        &mut recording_steps,
        &mut next_request_id,
    );
    let detail_data = match detail_result {
        CommandOutcome::Result(data) => {
            result
                .levels
                .insert("L3-detail".into(), LevelResult::pass());
            data
        }
        CommandOutcome::Error { reason, detail } => {
            result.failure_reason = Some(reason.clone());
            result
                .levels
                .insert("L3-detail".into(), LevelResult::fail_with(reason, detail));
            skip_remaining(&mut result, &["L4-toc", "L5-content"]);
            finalize(&mut result, &recording, &recording_steps, config, start);
            return result;
        }
    };

    // 从 detail 结果取 tocUrl
    let toc_url = detail_data
        .get("tocUrl")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // 如果 detail 没给 tocUrl,退回 book_url
            book_url.clone()
        });

    // L4: book.toc
    let toc_id = next_request_id;
    next_request_id += 1;
    let toc_params = if let Some(rec) = &recording {
        let l4 = rec.steps.iter().find(|s| s.level == "L4-toc");
        json!({
            "sourceId": source_id,
            "bookId": book_id,
            "source": source_wrapper,
            "tocResponse": l4.map(|s| s.response_body.as_str()).unwrap_or(""),
        })
    } else {
        json!({
            "sourceId": source_id,
            "bookId": book_id,
            "source": source_wrapper,
            "tocUrl": toc_url,
        })
    };
    let toc_result = run_command_with_http(
        &runtime,
        &rx,
        toc_id,
        methods::BOOK_TOC,
        toc_params,
        config.timeout,
        &config.offline_dir,
        &recording,
        "L4-toc",
        &mut recording_steps,
        &mut next_request_id,
    );
    let toc_data = match toc_result {
        CommandOutcome::Result(data) => {
            let toc = data.get("toc").cloned().unwrap_or(Value::Array(vec![]));
            if toc.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                result
                    .levels
                    .insert("L4-toc".into(), LevelResult::fail("no_toc_entries"));
                result.failure_reason = Some("no_toc_entries".into());
                skip_remaining(&mut result, &["L5-content"]);
                finalize(&mut result, &recording, &recording_steps, config, start);
                return result;
            }
            result.levels.insert("L4-toc".into(), LevelResult::pass());
            toc
        }
        CommandOutcome::Error { reason, detail } => {
            result.failure_reason = Some(reason.clone());
            result
                .levels
                .insert("L4-toc".into(), LevelResult::fail_with(reason, detail));
            skip_remaining(&mut result, &["L5-content"]);
            finalize(&mut result, &recording, &recording_steps, config, start);
            return result;
        }
    };

    // 取第一章
    let first_chapter = toc_data.get(0).cloned().unwrap_or(Value::Null);
    let chapter_url = first_chapter
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let chapter_title = first_chapter
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if chapter_url.is_empty() {
        result
            .levels
            .insert("L5-content".into(), LevelResult::fail("no_chapter_url"));
        result.failure_reason = Some("no_chapter_url".into());
        finalize(&mut result, &recording, &recording_steps, config, start);
        return result;
    }

    // L5: chapter.content
    let content_id = next_request_id;
    next_request_id += 1;
    let content_params = if let Some(rec) = &recording {
        let l5 = rec.steps.iter().find(|s| s.level == "L5-content");
        json!({
            "sourceId": source_id,
            "bookId": book_id,
            "chapterTitle": chapter_title,
            "source": source_wrapper,
            "chapterResponse": l5.map(|s| s.response_body.as_str()).unwrap_or(""),
        })
    } else {
        json!({
            "sourceId": source_id,
            "bookId": book_id,
            "chapterTitle": chapter_title,
            "source": source_wrapper,
            "chapterUrl": chapter_url,
        })
    };
    let content_result = run_command_with_http(
        &runtime,
        &rx,
        content_id,
        methods::CHAPTER_CONTENT,
        content_params,
        config.timeout,
        &config.offline_dir,
        &recording,
        "L5-content",
        &mut recording_steps,
        &mut next_request_id,
    );
    match content_result {
        CommandOutcome::Result(data) => {
            let content = data
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // L5 通过条件: content >50 字符(对照 AGENTS.md 5 级通过定义).
            // 旧实现仅判 is_empty(),无法识别"返回但内容过短"的假通过.
            if content.chars().count() <= 50 {
                result
                    .levels
                    .insert("L5-content".into(), LevelResult::fail("content_too_short"));
                result.failure_reason = Some("content_too_short".into());
            } else {
                result
                    .levels
                    .insert("L5-content".into(), LevelResult::pass());
            }
        }
        CommandOutcome::Error { reason, detail } => {
            result.failure_reason = Some(reason.clone());
            result
                .levels
                .insert("L5-content".into(), LevelResult::fail_with(reason, detail));
        }
    }

    finalize(&mut result, &recording, &recording_steps, config, start);
    result
}

enum CommandOutcome {
    Result(Value),
    Error { reason: String, detail: String },
}

/// 发送一个 command,自动处理可能出现的 http.execute host request.
///
/// - live 模式:Core 发 http.execute → 本函数用 ureq 真实拉取 → host.complete
/// - offline 模式:不发 HTTP(Core 因为传了 *Response 字段,直接解析)
#[allow(clippy::too_many_arguments)]
fn run_command_with_http(
    runtime: &Runtime,
    rx: &Receiver<Event>,
    request_id: u64,
    method: &str,
    params: Value,
    timeout: Duration,
    _offline_dir: &Option<PathBuf>,
    recording: &Option<SourceRecording>,
    level: &str,
    recording_steps: &mut Vec<RecordedStep>,
    next_request_id: &mut u64,
) -> CommandOutcome {
    if let Err(err) = runtime.send(Command::new(request_id, method, params)) {
        return CommandOutcome::Error {
            reason: "send_error".into(),
            detail: core_error_string(&err),
        };
    }

    loop {
        let event = match recv_with_timeout(rx, timeout) {
            Ok(e) => e,
            Err(err) => {
                return CommandOutcome::Error {
                    reason: "timeout_or_disconnected".into(),
                    detail: err,
                };
            }
        };
        match event {
            Event::Result { data, .. } => return CommandOutcome::Result(data),
            Event::Error { error, .. } => {
                // 判断是否是 JS unsupported 之类的已知原因
                let reason = classify_error(&error);
                return CommandOutcome::Error {
                    reason,
                    detail: core_error_string(&error),
                };
            }
            Event::HostRequest {
                operation_id,
                capability,
                params,
                ..
            } => {
                if capability != HostCapability::HttpExecute {
                    return CommandOutcome::Error {
                        reason: "unexpected_host_capability".into(),
                        detail: format!("expected http.execute, got {capability:?}"),
                    };
                }
                // 离线模式:Core 不应该发 http.execute(因为传了 *Response)
                // 如果还是发了,说明该步骤录像缺失,用录像数据兜底
                let response = if let Some(rec) = recording {
                    let step = rec.steps.iter().find(|s| s.level == level);
                    if let Some(step) = step {
                        HostHttpResponse {
                            status: step.response_status,
                            headers: step.response_headers.clone(),
                            body: step.response_body.clone(),
                            final_url: step.final_url.clone(),
                        }
                    } else {
                        // 录像缺失,返回空响应让 Core 报错
                        HostHttpResponse {
                            status: 404,
                            headers: json!({}),
                            body: String::new(),
                            final_url: None,
                        }
                    }
                } else {
                    // live 模式:真实 HTTP
                    match execute_http(&params, timeout) {
                        Ok(r) => r,
                        Err(err) => {
                            // 发 host.error
                            let err_id = *next_request_id;
                            *next_request_id += 1;
                            let _ = runtime.send(Command::new(
                                err_id,
                                methods::HOST_ERROR,
                                json!({
                                    "operationId": operation_id,
                                    "error": {
                                        "code": "INTERNAL",
                                        "message": err,
                                        "retryable": false,
                                    },
                                }),
                            ));
                            // 继续等 Core 把 error 事件发回来
                            continue;
                        }
                    }
                };

                // 记录这一步(录像)
                if recording.is_none() {
                    recording_steps.push(RecordedStep {
                        level: level.to_string(),
                        url: params
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        method: params
                            .get("method")
                            .and_then(Value::as_str)
                            .unwrap_or("GET")
                            .to_string(),
                        request_headers: params.get("headers").cloned().unwrap_or(json!({})),
                        request_body: params
                            .get("body")
                            .and_then(Value::as_str)
                            .map(|s| s.to_string()),
                        response_status: response.status,
                        response_headers: response.headers.clone(),
                        response_body: response.body.clone(),
                        final_url: response.final_url.clone(),
                    });
                }

                let complete_id = *next_request_id;
                *next_request_id += 1;
                if let Err(err) = runtime.send(Command::new(
                    complete_id,
                    methods::HOST_COMPLETE,
                    json!({
                        "operationId": operation_id,
                        "result": {
                            "status": response.status,
                            "headers": response.headers,
                            "body": response.body,
                            "finalUrl": response.final_url,
                        },
                    }),
                )) {
                    return CommandOutcome::Error {
                        reason: "host_complete_send_error".into(),
                        detail: core_error_string(&err),
                    };
                }
                // 继续等 Core 的最终 Result/Error
            }
        }
    }
}

/// 用 ureq 执行真实 HTTP 请求.
fn execute_http(params: &Value, timeout: Duration) -> Result<HostHttpResponse, String> {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .ok_or("missing url")?;
    let method = params
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_uppercase();
    let headers = params.get("headers").cloned().unwrap_or(json!({}));
    let body = params.get("body").and_then(Value::as_str);

    // 用一个信任所有证书的 rustls config — 这是 dev 工具,用于测试真实 Legado 书源,
    // 很多书源站点的 TLS 证书有问题(自签名/过期/链不全),不能用浏览器级别严格校验.
    let tls_config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    let agent = ureq::AgentBuilder::new()
        .tls_config(Arc::new(tls_config))
        .timeout_read(timeout)
        .timeout_write(timeout)
        .redirects(10)
        .build();

    let req = match method.as_str() {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        "PUT" => agent.put(url),
        "DELETE" => agent.delete(url),
        "HEAD" => agent.head(url),
        other => agent.request(other, url),
    };

    // 设置 headers
    let mut req = req;
    if let Some(h) = headers.as_object() {
        for (k, v) in h {
            if let Some(s) = v.as_str() {
                req = req.set(k, s);
            }
        }
    }

    // 执行
    let response = if let Some(b) = body {
        match req.send_string(b) {
            Ok(r) => r,
            Err(ureq::Error::Status(_status, r)) => r, // 非 2xx 也算响应,把 body 拿回来
            Err(err) => return Err(err.to_string()),
        }
    } else {
        match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_status, r)) => r,
            Err(err) => return Err(err.to_string()),
        }
    };

    let status = response.status();
    let mut resp_headers = serde_json::Map::new();
    for name in response.headers_names() {
        if let Some(val) = response.header(&name) {
            resp_headers.insert(name, Value::String(val.to_string()));
        }
    }
    let final_url = response.get_url().to_string();
    let body_text = response.into_string().unwrap_or_default();

    Ok(HostHttpResponse {
        status,
        headers: Value::Object(resp_headers),
        body: body_text,
        final_url: Some(final_url),
    })
}

fn recv_with_timeout(rx: &Receiver<Event>, timeout: Duration) -> Result<Event, String> {
    rx.recv_timeout(timeout).map_err(|err| match err {
        std::sync::mpsc::RecvTimeoutError::Timeout => format!("recv timeout after {timeout:?}"),
        std::sync::mpsc::RecvTimeoutError::Disconnected => "runtime disconnected".into(),
    })
}

fn classify_error(error: &CoreError) -> String {
    // CoreError 没有 Display,用 message + details JSON 字符串做模式匹配
    let s = core_error_string(error);
    if s.contains("unsupported") || s.contains("JS") || s.contains("js") {
        "js_unsupported".into()
    } else if s.contains("timeout") || s.contains("Timeout") {
        "http_timeout".into()
    } else if s.contains("http") || s.contains("HTTP") || s.contains("network") {
        "http_error".into()
    } else if s.contains("multirule") || s.contains("MultiRule") {
        "multirule_blocker".into()
    } else if s.contains("extract") || s.contains("parse") {
        "parse_error".into()
    } else {
        "core_error".into()
    }
}

/// 把 CoreError 转成可读字符串(没有 Display,手动拼).
fn core_error_string(error: &CoreError) -> String {
    let details_str = if error.details.is_null() {
        String::new()
    } else {
        format!(" details={}", error.details)
    };
    format!("[{:?}] {}{}", error.code, error.message, details_str)
}

fn skip_remaining(result: &mut SourceTestResult, levels: &[&str]) {
    for level in levels {
        result
            .levels
            .entry((*level).to_string())
            .or_insert_with(|| LevelResult::skip("previous_level_failed"));
    }
}

fn failed_result(
    source_path: &Path,
    source_id: &str,
    source_name: &str,
    level: &str,
    reason: &str,
    detail: &str,
    start: Instant,
) -> SourceTestResult {
    let mut levels = BTreeMap::new();
    levels.insert(level.to_string(), LevelResult::fail_with(reason, detail));
    SourceTestResult {
        source_id: source_id.to_string(),
        source_file: source_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        source_name: source_name.to_string(),
        source_url: String::new(),
        priority: None,
        rule_forms: vec![],
        has_js: false,
        has_multirule: false,
        has_regex: false,
        levels,
        failure_reason: Some(reason.to_string()),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

fn finalize(
    result: &mut SourceTestResult,
    _recording: &Option<SourceRecording>,
    recording_steps: &[RecordedStep],
    config: &TestSourceConfig,
    start: Instant,
) {
    result.duration_ms = start.elapsed().as_millis() as u64;

    // 保存录像(live 模式 + --record)
    if let Some(record_dir) = &config.record_dir {
        if !recording_steps.is_empty() {
            let rec = SourceRecording {
                source_id: result.source_id.clone(),
                source_name: result.source_name.clone(),
                recorded_at: chrono_now_iso(),
                keyword: config.keyword.clone(),
                steps: recording_steps.to_vec(),
            };
            let _ = fs::create_dir_all(record_dir);
            let path = record_dir.join(format!("{}.json", result.source_id));
            let _ = fs::write(
                &path,
                serde_json::to_string_pretty(&rec).unwrap_or_default(),
            );
        }
    }
}

fn chrono_now_iso() -> String {
    // 不引入 chrono,用 SystemTime + UTC 估算
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("1970-01-01T00:00:{now}Z")
}

/// 信任所有证书的 verifier — 仅用于 dev 工具测试真实 Legado 书源.
///
/// 很多书源站点的 TLS 证书有问题(自签名/过期/链不全),浏览器级别严格校验会
/// 直接失败,导致无法验证 Core 的解析能力。这个 verifier 跳过所有证书校验,
/// 让 HTTP 拉取能继续,从而把测试焦点放在 Core 的规则解析上。
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![
            RSA_PKCS1_SHA256,
            RSA_PKCS1_SHA384,
            RSA_PKCS1_SHA512,
            ECDSA_NISTP256_SHA256,
            ECDSA_NISTP384_SHA384,
            ED25519,
            RSA_PSS_SHA256,
            RSA_PSS_SHA384,
            RSA_PSS_SHA512,
        ]
    }
}

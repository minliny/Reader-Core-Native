//! Reader-Core QuickJS integration.
//!
//! This crate owns the native JS sandbox boundary. The first batch keeps the
//! public surface narrow: evaluate a script, return a JSON-compatible value,
//! expose structured failures, enforce configured execution limits, and route
//! host API calls through an internal callback registry.

use rquickjs::{
    function::Rest, CatchResultExt, Context, Ctx, Error as QuickJsError, Exception, Runtime,
    Value as QuickJsValue,
};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use std::{
    collections::BTreeMap,
    error::Error as StdError,
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_JSON_DEPTH: usize = 128;
const MAX_PROMISE_JOBS: usize = 1024;
const DEFAULT_LOCAL_TIME_OFFSET_MS: i64 = 8 * 60 * 60 * 1000;
const DEFAULT_TIME_FORMAT_PATTERN: &str = "yyyy/MM/dd HH:mm";
const DEFAULT_WEB_VIEW_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1";
const REFRESH_TOC_URL_LOG_MESSAGE: &str = "java.refreshTocUrl() requested";
const TOAST_LOG_MESSAGE: &str = "java.toast() requested";
const LONG_TOAST_LOG_MESSAGE: &str = "java.longToast() requested";
static UUID_COUNTER: AtomicU64 = AtomicU64::new(0);

pub type JsResult<T> = Result<T, JsError>;

pub trait JsSandbox {
    fn evaluate(&self, script: &str) -> JsResult<JsEvaluation>;
    fn evaluate_with_options(
        &self,
        script: &str,
        options: JsExecutionOptions,
    ) -> JsResult<JsEvaluation>;
    fn capabilities(&self) -> JsRuntimeCapabilities;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsRuntimeConfig {
    pub timeout: Option<Duration>,
    pub memory_limit_bytes: Option<usize>,
    pub max_stack_size_bytes: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct JsExecutionOptions {
    pub timeout: Option<Duration>,
    pub cancellation_token: Option<CancellationToken>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JsEvaluation {
    pub value: JsonValue,
    pub console: Vec<ConsoleRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsRuntimeCapabilities {
    pub engine: &'static str,
    pub timeout: CapabilityStatus,
    pub cancellation: CapabilityStatus,
    pub memory_limit: CapabilityStatus,
    pub stack_limit: CapabilityStatus,
    pub console_capture: CapabilityStatus,
    pub promise_jobs: CapabilityStatus,
    pub host_callbacks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityStatus {
    Enforced,
    SupportedNotConfigured,
    Unsupported { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    pub kind: JsErrorKind,
    pub message: String,
    pub stack: Option<String>,
    pub value: Option<JsonValue>,
}

impl JsError {
    fn new(kind: JsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            stack: None,
            value: None,
        }
    }

    fn with_stack(mut self, stack: Option<String>) -> Self {
        self.stack = stack;
        self
    }

    fn with_value(mut self, value: Option<JsonValue>) -> Self {
        self.value = value;
        self
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl StdError for JsError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsErrorKind {
    Cancelled,
    Exception,
    HostCallback,
    Internal,
    MemoryLimit,
    NonJsonValue,
    Syntax,
    Timeout,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleRecord {
    pub level: ConsoleLevel,
    pub args: Vec<JsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleLevel {
    Log,
    Warn,
    Error,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Strong-typed descriptor for a host-routed `java.*` call.
///
/// Replaces the former weak `HostCall { name, args: Vec<JsonValue> }` shape.
/// The host receives a `HostDescriptor` variant and pattern-matches on it
/// directly — no string-name switch, no re-parsing of `Vec<JsonValue>` args.
/// reader-js routes the JS call into the appropriate variant (parsing/validating
/// args once); the host executes the real network/file/state operation and
/// returns a `JsonValue` back to JS.
///
/// Field semantics mirror legado's `JsExtensions` / `AnalyzeRule` signatures
/// (see audit in project_memory). Return shapes are host-defined `JsonValue`
/// (reader-js cannot bridge legado's live Java objects like `Connection.Response`
/// or `StrResponse` — the host serializes the relevant fields into JSON).
#[derive(Clone, Debug, PartialEq)]
pub enum HostDescriptor {
    /// `java.get(urlStr, headers?)` — low-level HTTP GET (legado JsExtensions).
    HttpGet {
        url: String,
        headers: Option<JsonValue>,
    },
    /// `java.post(urlStr, body, headers?)` — low-level HTTP POST. `body` defaults
    /// to empty string when omitted (legado leniency).
    HttpPost {
        url: String,
        body: String,
        headers: Option<JsonValue>,
    },
    /// `java.connect(urlStr, header?)` — high-level fetch returning a response
    /// object. `header` is a JSON-encoded string (legado `Map<String,String>`).
    HttpConnect { url: String, header: Option<String> },
    /// `java.ajax(url)` — high-level fetch returning the body string only (legado
    /// returns `String?`, NOT a Response object). If `url` is a list, legado
    /// takes the first element.
    Ajax { url: String },
    /// `java.ajaxAll(urlList)` — concurrent fetch, returns one response per url.
    AjaxAll { urls: Vec<String> },
    /// `java.getSource()` — returns the currently-bound source (no args).
    GetSource,
    /// `java.getString(ruleStr)` — rule-engine content extraction (NOT network).
    GetString { rule: String },
    /// `java.getStringList(ruleStr)` — multi-valued rule extraction.
    GetStringList { rule: String },
    /// `java.downloadFile(url)` — stream URL to cache file, return relative path.
    DownloadFile { url: String },
    /// `java.cacheFile(urlStr, saveTime?)` — text cache, returns file contents.
    /// `save_time` is in seconds (legado `Int`).
    CacheFile { url: String, save_time: Option<i64> },
    /// `java.importScript(path)` — fetch script source text (http → cacheFile,
    /// local → readTxtFile). Does NOT execute the script.
    ImportScript { path: String },
    /// `java.setContent(content, baseUrl?)` — re-init rule engine working content.
    /// `content` is `Option` (legado throws on null; reader-js passes None through).
    SetContent {
        content: Option<String>,
        base_url: Option<String>,
    },
    /// `java.put(key, value)` — variable storage (NOT HTTP PUT). Writes to
    /// source/book/chapter variable map; returns `value`.
    Put { key: String, value: String },
    /// `java.reGetBook()` — re-discover the book (no args, side-effect only).
    ReGetBook,

    // ===== S3 network-class closure: 28 new variants vs Legado JsExtensions.kt =====
    /// `java.head(urlStr, headers)` — HTTP HEAD (no body, like `get` but HEAD).
    /// Legado JsExtensions.kt:399. `headers` is the JS object passed as 2nd arg
    /// (legado `Map<String,String>`); reader-js passes the raw JSON through.
    HttpHead {
        url: String,
        headers: Option<JsonValue>,
    },

    /// `java.webView(html?, url?, js?)` — load WebView, run JS, return body.
    /// Legado JsExtensions.kt:170. All three args are nullable strings.
    WebView {
        html: Option<String>,
        url: Option<String>,
        js: Option<String>,
    },
    /// `java.webViewGetSource(html?, url?, js?, sourceRegex)` — WebView fetch
    /// with source-URL regex. Legado JsExtensions.kt:188.
    WebViewGetSource {
        html: Option<String>,
        url: Option<String>,
        js: Option<String>,
        source_regex: String,
    },
    /// `java.webViewGetOverrideUrl(html?, url?, js?, overrideUrlRegex)` —
    /// WebView fetch with override-URL regex. Legado JsExtensions.kt:207.
    WebViewGetOverrideUrl {
        html: Option<String>,
        url: Option<String>,
        js: Option<String>,
        override_url_regex: String,
    },
    /// `java.startBrowser(url, title)` — open in-app browser (no return value).
    /// Legado JsExtensions.kt:233. Host returns `null` on success.
    StartBrowser { url: String, title: String },
    /// `java.startBrowserAwait(url, title[, refetchAfterSuccess])` — open
    /// browser and wait for verification result. Legado JsExtensions.kt:241/249.
    /// `refetch_after_success` defaults to `true` when omitted.
    StartBrowserAwait {
        url: String,
        title: String,
        refetch_after_success: Option<bool>,
    },
    /// `java.getVerificationCode(imageUrl)` — captcha image verification.
    /// Legado JsExtensions.kt:256.
    GetVerificationCode { image_url: String },
    /// `java.getCookie(tag[, key])` — cookie jar read. Legado JsExtensions.kt:305/309.
    /// `key` is `None` when only `tag` is provided (returns full cookie header).
    GetCookie { tag: String, key: Option<String> },

    /// `java.getFile(path)` — resolve a cache-relative path to a File. Legado
    /// JsExtensions.kt:566. Host returns a JSON object with `path`/`exists`/etc.
    GetFile { path: String },
    /// `java.readFile(path)` — read raw bytes of a cache-relative file. Legado
    /// JsExtensions.kt:581. Host returns base64-encoded bytes or `null`.
    ReadFile { path: String },
    /// `java.readTxtFile(path[, charsetName])` — read text file with optional
    /// charset. Legado JsExtensions.kt:589/598. `charset` is `None` for auto-detect.
    ReadTxtFile {
        path: String,
        charset: Option<String>,
    },
    /// `java.deleteFile(path)` — delete a cache-relative file. Legado JsExtensions.kt:609.
    /// Host returns `true`/`false`.
    DeleteFile { path: String },
    /// `java.unzipFile(zipPath)` — extract zip. Legado JsExtensions.kt:619. Host
    /// returns the relative extraction path.
    UnzipFile { zip_path: String },
    /// `java.un7zFile(zipPath)` — extract 7z. Legado JsExtensions.kt:628.
    Un7zFile { zip_path: String },
    /// `java.unrarFile(zipPath)` — extract rar. Legado JsExtensions.kt:637.
    UnrarFile { zip_path: String },
    /// `java.unArchiveFile(zipPath)` — extract any archive. Legado JsExtensions.kt:646.
    UnArchiveFile { zip_path: String },
    /// `java.getTxtInFolder(path)` — concat all text files in a folder. Legado
    /// JsExtensions.kt:659.
    GetTxtInFolder { path: String },
    /// `java.getZipStringContent(url, path[, charsetName])` — read a file inside
    /// a zip. Legado JsExtensions.kt:683/689. `url` may be http URL or hex string.
    GetZipStringContent {
        url: String,
        path: String,
        charset: Option<String>,
    },
    /// `java.getRarStringContent(url, path[, charsetName])` — Legado JsExtensions.kt:700/706.
    GetRarStringContent {
        url: String,
        path: String,
        charset: Option<String>,
    },
    /// `java.get7zStringContent(url, path[, charsetName])` — Legado JsExtensions.kt:717/723.
    Get7zStringContent {
        url: String,
        path: String,
        charset: Option<String>,
    },
    /// `java.getZipByteArrayContent(url, path)` — read raw bytes from zip entry.
    /// Legado JsExtensions.kt:734. Host returns base64-encoded bytes or `null`.
    GetZipByteArrayContent { url: String, path: String },
    /// `java.getRarByteArrayContent(url, path)` — Legado JsExtensions.kt:762.
    GetRarByteArrayContent { url: String, path: String },
    /// `java.get7zByteArrayContent(url, path)` — Legado JsExtensions.kt:780.
    Get7zByteArrayContent { url: String, path: String },

    /// `java.queryBase64TTF(data)` — deprecated alias of `queryTTF`. Legado
    /// JsExtensions.kt:802. Host returns a JSON handle representing the parsed
    /// font (reader-js cannot bridge legado's live `QueryTTF` object).
    QueryBase64TTF { data: String },
    /// `java.queryTTF(data[, useCache])` — parse TTF font from url/file/base64/
    /// bytes. Legado JsExtensions.kt:813/857. `use_cache` defaults to `true`.
    QueryTTF {
        data: JsonValue,
        use_cache: Option<bool>,
    },
    /// `java.replaceFont(text, errorQueryTTF, correctQueryTTF[, filter])` —
    /// replace obfuscated font glyphs. Legado JsExtensions.kt:867/904. The TTF
    /// args are the JSON handles returned by `queryTTF`; `filter` defaults to
    /// `false`.
    ReplaceFont {
        text: String,
        error_query_ttf: JsonValue,
        correct_query_ttf: JsonValue,
        filter: Option<bool>,
    },

    /// `java.androidId()` — device ID (no args). Legado JsExtensions.kt:981.
    AndroidId,
    /// `java.openUrl(url[, mimeType])` — open external URL. Legado
    /// JsExtensions.kt:985/990. `mime_type` is `None` when omitted.
    OpenUrl {
        url: String,
        mime_type: Option<String>,
    },
}

impl HostDescriptor {
    /// Returns the `java.*` callback name this descriptor routes to. Useful for
    /// hosts that still want to log/inspect the routed method name.
    pub fn callback_name(&self) -> &'static str {
        match self {
            Self::HttpGet { .. } => "java.get",
            Self::HttpPost { .. } => "java.post",
            Self::HttpConnect { .. } => "java.connect",
            Self::Ajax { .. } => "java.ajax",
            Self::AjaxAll { .. } => "java.ajaxAll",
            Self::GetSource => "java.getSource",
            Self::GetString { .. } => "java.getString",
            Self::GetStringList { .. } => "java.getStringList",
            Self::DownloadFile { .. } => "java.downloadFile",
            Self::CacheFile { .. } => "java.cacheFile",
            Self::ImportScript { .. } => "java.importScript",
            Self::SetContent { .. } => "java.setContent",
            Self::Put { .. } => "java.put",
            Self::ReGetBook => "java.reGetBook",
            // S3 network-class closure (28 new variants)
            Self::HttpHead { .. } => "java.head",
            Self::WebView { .. } => "java.webView",
            Self::WebViewGetSource { .. } => "java.webViewGetSource",
            Self::WebViewGetOverrideUrl { .. } => "java.webViewGetOverrideUrl",
            Self::StartBrowser { .. } => "java.startBrowser",
            Self::StartBrowserAwait { .. } => "java.startBrowserAwait",
            Self::GetVerificationCode { .. } => "java.getVerificationCode",
            Self::GetCookie { .. } => "java.getCookie",
            Self::GetFile { .. } => "java.getFile",
            Self::ReadFile { .. } => "java.readFile",
            Self::ReadTxtFile { .. } => "java.readTxtFile",
            Self::DeleteFile { .. } => "java.deleteFile",
            Self::UnzipFile { .. } => "java.unzipFile",
            Self::Un7zFile { .. } => "java.un7zFile",
            Self::UnrarFile { .. } => "java.unrarFile",
            Self::UnArchiveFile { .. } => "java.unArchiveFile",
            Self::GetTxtInFolder { .. } => "java.getTxtInFolder",
            Self::GetZipStringContent { .. } => "java.getZipStringContent",
            Self::GetRarStringContent { .. } => "java.getRarStringContent",
            Self::Get7zStringContent { .. } => "java.get7zStringContent",
            Self::GetZipByteArrayContent { .. } => "java.getZipByteArrayContent",
            Self::GetRarByteArrayContent { .. } => "java.getRarByteArrayContent",
            Self::Get7zByteArrayContent { .. } => "java.get7zByteArrayContent",
            Self::QueryBase64TTF { .. } => "java.queryBase64TTF",
            Self::QueryTTF { .. } => "java.queryTTF",
            Self::ReplaceFont { .. } => "java.replaceFont",
            Self::AndroidId => "java.androidId",
            Self::OpenUrl { .. } => "java.openUrl",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostError {
    pub message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl StdError for HostError {}

type HostCallback =
    Arc<dyn Fn(HostDescriptor) -> Result<JsonValue, HostError> + Send + Sync + 'static>;
type ConsoleBuffer = Arc<Mutex<Vec<ConsoleRecord>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastDuration {
    Short,
    Long,
}

impl ToastDuration {
    fn label(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Long => "long",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Short => TOAST_LOG_MESSAGE,
            Self::Long => LONG_TOAST_LOG_MESSAGE,
        }
    }
}

#[derive(Clone, Default)]
pub struct HostCallbackRegistry {
    callbacks: BTreeMap<String, HostCallback>,
}

impl HostCallbackRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&mut self, name: impl Into<String>, callback: F)
    where
        F: Fn(HostDescriptor) -> Result<JsonValue, HostError> + Send + Sync + 'static,
    {
        self.callbacks.insert(name.into(), Arc::new(callback));
    }

    pub fn contains(&self, name: &str) -> bool {
        self.callbacks.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.callbacks.keys().cloned().collect()
    }

    fn call(&self, name: &str, descriptor: HostDescriptor) -> Result<JsonValue, HostError> {
        let callback = self
            .callbacks
            .get(name)
            .ok_or_else(|| HostError::new(format!("unregistered host callback: {name}")))?;
        callback(descriptor)
    }
}

impl fmt::Debug for HostCallbackRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostCallbackRegistry")
            .field("callbacks", &self.names())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct QuickJsSandbox {
    config: JsRuntimeConfig,
    host_callbacks: HostCallbackRegistry,
}

impl QuickJsSandbox {
    pub fn new(config: JsRuntimeConfig) -> Self {
        Self {
            config,
            host_callbacks: HostCallbackRegistry::default(),
        }
    }

    pub fn with_host_callbacks(
        config: JsRuntimeConfig,
        host_callbacks: HostCallbackRegistry,
    ) -> Self {
        Self {
            config,
            host_callbacks,
        }
    }

    pub fn config(&self) -> &JsRuntimeConfig {
        &self.config
    }

    pub fn host_callbacks(&self) -> &HostCallbackRegistry {
        &self.host_callbacks
    }

    fn install_host_api<'js>(
        &self,
        ctx: &Ctx<'js>,
        console: ConsoleBuffer,
    ) -> Result<(), QuickJsError> {
        let java = rquickjs::Object::new(ctx.clone())?;
        java.set(
            "get",
            make_host_callback(ctx.clone(), HostMethod::Get, self.host_callbacks.clone())?,
        )?;
        java.set(
            "post",
            make_host_callback(ctx.clone(), HostMethod::Post, self.host_callbacks.clone())?,
        )?;
        java.set(
            "connect",
            make_host_callback(
                ctx.clone(),
                HostMethod::Connect,
                self.host_callbacks.clone(),
            )?,
        )?;
        java.set(
            "ajaxAll",
            make_host_callback(
                ctx.clone(),
                HostMethod::AjaxAll,
                self.host_callbacks.clone(),
            )?,
        )?;
        java.set(
            "ajax",
            make_host_callback(ctx.clone(), HostMethod::Ajax, self.host_callbacks.clone())?,
        )?;
        java.set(
            "getSource",
            make_host_callback(
                ctx.clone(),
                HostMethod::GetSource,
                self.host_callbacks.clone(),
            )?,
        )?;
        java.set(
            "getString",
            make_host_callback(
                ctx.clone(),
                HostMethod::GetString,
                self.host_callbacks.clone(),
            )?,
        )?;
        java.set(
            "getStringList",
            make_host_callback(
                ctx.clone(),
                HostMethod::GetStringList,
                self.host_callbacks.clone(),
            )?,
        )?;
        java.set("base64Encode", make_base64_encode_callback(ctx.clone())?)?;
        java.set("base64Decode", make_base64_decode_callback(ctx.clone())?)?;
        java.set("base64Decoder", make_base64_decode_callback(ctx.clone())?)?;
        java.set(
            "base64DecodeToByteArray",
            make_base64_decode_to_byte_array_callback(ctx.clone())?,
        )?;
        java.set("hexEncode", make_hex_encode_callback(ctx.clone())?)?;
        java.set("hexDecode", make_hex_decode_callback(ctx.clone())?)?;
        java.set("hexEncodeToString", make_hex_encode_callback(ctx.clone())?)?;
        java.set("hexDecodeToString", make_hex_decode_callback(ctx.clone())?)?;
        java.set(
            "hexDecodeToByteArray",
            make_hex_decode_to_byte_array_callback(ctx.clone())?,
        )?;
        java.set("md5Encode", make_md5_encode_callback(ctx.clone())?)?;
        java.set("md5Encode16", make_md5_encode16_callback(ctx.clone())?)?;
        java.set("hashDigest", make_hash_digest_callback(ctx.clone())?)?;
        java.set("digestHex", make_hash_digest_callback(ctx.clone())?)?;
        java.set(
            "digestBase64Str",
            make_hash_digest_base64_callback(ctx.clone())?,
        )?;
        java.set("hmacDigest", make_hmac_digest_callback(ctx.clone())?)?;
        java.set("HMacHex", make_hmac_digest_callback(ctx.clone())?)?;
        java.set("hmacHex", make_hmac_digest_callback(ctx.clone())?)?;
        java.set("HMacBase64", make_hmac_base64_callback(ctx.clone())?)?;
        java.set("hmacBase64", make_hmac_base64_callback(ctx.clone())?)?;
        java.set("createSign", make_create_sign_callback(ctx.clone())?)?;
        java.set(
            "createSymmetricCrypto",
            make_create_symmetric_crypto_callback(ctx.clone())?,
        )?;
        java.set(
            "aesBase64DecodeToString",
            make_aes_base64_decode_to_string_callback(ctx.clone())?,
        )?;
        java.set(
            "aesEncodeToBase64String",
            make_aes_encode_to_base64_string_callback(ctx.clone())?,
        )?;
        java.set(
            "desEncodeToBase64String",
            make_des_encode_to_base64_string_callback(ctx.clone())?,
        )?;
        java.set(
            "tripleDESEncodeBase64Str",
            make_triple_des_encode_base64_str_callback(ctx.clone())?,
        )?;
        java.set("strToBytes", make_str_to_bytes_callback(ctx.clone())?)?;
        java.set("bytesToStr", make_bytes_to_str_callback(ctx.clone())?)?;
        java.set("encodeURI", make_encode_uri_callback(ctx.clone())?)?;
        java.set(
            "encodeURIComponent",
            make_encode_uri_component_callback(ctx.clone())?,
        )?;
        java.set("timeFormat", make_time_format_callback(ctx.clone())?)?;
        java.set("timeFormatUTC", make_time_format_utc_callback(ctx.clone())?)?;
        java.set("toNumChapter", make_to_num_chapter_callback(ctx.clone())?)?;
        java.set("t2s", make_t2s_callback(ctx.clone())?)?;
        java.set("s2t", make_s2t_callback(ctx.clone())?)?;
        java.set("htmlFormat", make_html_format_callback(ctx.clone())?)?;
        java.set("toURL", make_to_url_callback(ctx.clone())?)?;
        java.set("log", make_log_callback(ctx.clone(), console.clone())?)?;
        java.set(
            "refreshTocUrl",
            make_refresh_toc_url_callback(ctx.clone(), console.clone())?,
        )?;
        java.set(
            "toast",
            make_toast_callback(ctx.clone(), console.clone(), ToastDuration::Short)?,
        )?;
        java.set(
            "longToast",
            make_toast_callback(ctx.clone(), console.clone(), ToastDuration::Long)?,
        )?;
        java.set("logType", make_log_type_callback(ctx.clone())?)?;
        java.set("randomUUID", make_random_uuid_callback(ctx.clone())?)?;
        java.set("getWebViewUA", make_get_web_view_ua_callback(ctx.clone())?)?;
        java.set(
            "call",
            make_host_dispatch_callback(ctx.clone(), self.host_callbacks.clone())?,
        )?;
        install_residual_host_routing(ctx.clone(), &java, self.host_callbacks.clone())?;
        ctx.globals().set("java", java)?;
        ctx.globals()
            .set("Buffer", make_buffer_object(ctx.clone())?)?;
        ctx.globals().set(
            "ajax",
            make_host_callback(ctx.clone(), HostMethod::Ajax, self.host_callbacks.clone())?,
        )?;
        ctx.globals().set(
            "ajaxAll",
            make_host_callback(
                ctx.clone(),
                HostMethod::AjaxAll,
                self.host_callbacks.clone(),
            )?,
        )?;
        ctx.globals()
            .set("base64Encode", make_base64_encode_callback(ctx.clone())?)?;
        ctx.globals()
            .set("base64Decode", make_base64_decode_callback(ctx.clone())?)?;
        ctx.globals()
            .set("base64Decoder", make_base64_decode_callback(ctx.clone())?)?;
        ctx.globals().set(
            "base64DecodeToByteArray",
            make_base64_decode_to_byte_array_callback(ctx.clone())?,
        )?;
        ctx.globals()
            .set("hexEncode", make_hex_encode_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hexDecode", make_hex_decode_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hexEncodeToString", make_hex_encode_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hexDecodeToString", make_hex_decode_callback(ctx.clone())?)?;
        ctx.globals().set(
            "hexDecodeToByteArray",
            make_hex_decode_to_byte_array_callback(ctx.clone())?,
        )?;
        ctx.globals()
            .set("md5Encode", make_md5_encode_callback(ctx.clone())?)?;
        ctx.globals()
            .set("md5Encode16", make_md5_encode16_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hashDigest", make_hash_digest_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hmacDigest", make_hmac_digest_callback(ctx.clone())?)?;
        ctx.globals()
            .set("HMacHex", make_hmac_digest_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hmacHex", make_hmac_digest_callback(ctx.clone())?)?;
        ctx.globals()
            .set("HMacBase64", make_hmac_base64_callback(ctx.clone())?)?;
        ctx.globals()
            .set("hmacBase64", make_hmac_base64_callback(ctx.clone())?)?;
        ctx.globals()
            .set("createSign", make_create_sign_callback(ctx.clone())?)?;
        ctx.globals().set(
            "createSymmetricCrypto",
            make_create_symmetric_crypto_callback(ctx.clone())?,
        )?;
        ctx.globals().set(
            "aesBase64DecodeToString",
            make_aes_base64_decode_to_string_callback(ctx.clone())?,
        )?;
        ctx.globals().set(
            "aesEncodeToBase64String",
            make_aes_encode_to_base64_string_callback(ctx.clone())?,
        )?;
        ctx.globals()
            .set("strToBytes", make_str_to_bytes_callback(ctx.clone())?)?;
        ctx.globals()
            .set("bytesToStr", make_bytes_to_str_callback(ctx.clone())?)?;
        ctx.globals()
            .set("encodeURI", make_encode_uri_callback(ctx.clone())?)?;
        ctx.globals().set(
            "encodeURIComponent",
            make_encode_uri_component_callback(ctx.clone())?,
        )?;
        ctx.globals().set(
            "getSource",
            make_host_callback(
                ctx.clone(),
                HostMethod::GetSource,
                self.host_callbacks.clone(),
            )?,
        )?;
        ctx.globals()
            .set("timeFormat", make_time_format_callback(ctx.clone())?)?;
        ctx.globals()
            .set("timeFormatUTC", make_time_format_utc_callback(ctx.clone())?)?;
        ctx.globals()
            .set("toNumChapter", make_to_num_chapter_callback(ctx.clone())?)?;
        ctx.globals().set("t2s", make_t2s_callback(ctx.clone())?)?;
        ctx.globals().set("s2t", make_s2t_callback(ctx.clone())?)?;
        ctx.globals()
            .set("htmlFormat", make_html_format_callback(ctx.clone())?)?;
        ctx.globals()
            .set("toURL", make_to_url_callback(ctx.clone())?)?;
        ctx.globals()
            .set("log", make_log_callback(ctx.clone(), console.clone())?)?;
        ctx.globals().set(
            "refreshTocUrl",
            make_refresh_toc_url_callback(ctx.clone(), console.clone())?,
        )?;
        ctx.globals().set(
            "toast",
            make_toast_callback(ctx.clone(), console.clone(), ToastDuration::Short)?,
        )?;
        ctx.globals().set(
            "longToast",
            make_toast_callback(ctx.clone(), console.clone(), ToastDuration::Long)?,
        )?;
        ctx.globals()
            .set("logType", make_log_type_callback(ctx.clone())?)?;
        ctx.globals()
            .set("randomUUID", make_random_uuid_callback(ctx.clone())?)?;
        ctx.globals()
            .set("getWebViewUA", make_get_web_view_ua_callback(ctx.clone())?)?;
        Ok(())
    }

    fn install_console_api<'js>(
        &self,
        ctx: &Ctx<'js>,
        console: ConsoleBuffer,
    ) -> Result<(), QuickJsError> {
        let console_object = rquickjs::Object::new(ctx.clone())?;
        console_object.set(
            "log",
            make_console_callback(ctx.clone(), ConsoleLevel::Log, console.clone())?,
        )?;
        console_object.set(
            "warn",
            make_console_callback(ctx.clone(), ConsoleLevel::Warn, console.clone())?,
        )?;
        console_object.set(
            "error",
            make_console_callback(ctx.clone(), ConsoleLevel::Error, console)?,
        )?;
        ctx.globals().set("console", console_object)?;
        Ok(())
    }
}

impl Default for QuickJsSandbox {
    fn default() -> Self {
        Self::new(JsRuntimeConfig::default())
    }
}

impl JsSandbox for QuickJsSandbox {
    fn evaluate(&self, script: &str) -> JsResult<JsEvaluation> {
        self.evaluate_with_options(script, JsExecutionOptions::default())
    }

    fn evaluate_with_options(
        &self,
        script: &str,
        options: JsExecutionOptions,
    ) -> JsResult<JsEvaluation> {
        if options
            .cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(JsError::new(JsErrorKind::Cancelled, "execution cancelled"));
        }

        let timeout = options.timeout.or(self.config.timeout);
        if timeout.is_some_and(|duration| duration.is_zero()) {
            return Err(JsError::new(
                JsErrorKind::Timeout,
                "execution timeout elapsed before evaluation",
            ));
        }

        let runtime = Runtime::new().map_err(map_quickjs_engine_error)?;
        runtime
            .set_info("reader-core-native reader-js")
            .map_err(map_quickjs_engine_error)?;
        if let Some(limit) = self.config.memory_limit_bytes {
            runtime.set_memory_limit(limit);
        }
        if let Some(limit) = self.config.max_stack_size_bytes {
            runtime.set_max_stack_size(limit);
        }

        let interrupt = InterruptState::new(timeout, options.cancellation_token);
        runtime.set_interrupt_handler(Some(Box::new({
            let interrupt = interrupt.clone();
            move || interrupt.should_interrupt()
        })));

        let console = Arc::new(Mutex::new(Vec::new()));
        let context = Context::full(&runtime).map_err(map_quickjs_engine_error)?;
        let mut evaluation = context.with(|ctx| {
            self.install_host_api(&ctx, console.clone())
                .map_err(map_quickjs_engine_error)?;
            self.install_console_api(&ctx, console.clone())
                .map_err(map_quickjs_engine_error)?;
            let result = ctx.eval::<QuickJsValue<'_>, _>(script).catch(&ctx);
            let value = result.map_err(|error| map_caught_error(error, &interrupt))?;
            let value = resolve_maybe_promise(value, &ctx, &interrupt)?;
            drain_promise_jobs(&ctx, &interrupt)?;
            quickjs_value_to_json(&value, 0).map(|value| JsEvaluation {
                value,
                console: Vec::new(),
            })
        })?;
        evaluation.console = console_records(&console);
        Ok(evaluation)
    }

    fn capabilities(&self) -> JsRuntimeCapabilities {
        JsRuntimeCapabilities {
            engine: "quickjs/rquickjs",
            timeout: configured_status(self.config.timeout.is_some()),
            cancellation: CapabilityStatus::SupportedNotConfigured,
            memory_limit: configured_status(self.config.memory_limit_bytes.is_some()),
            stack_limit: configured_status(self.config.max_stack_size_bytes.is_some()),
            console_capture: CapabilityStatus::Enforced,
            promise_jobs: CapabilityStatus::Enforced,
            host_callbacks: self.host_callbacks.names(),
        }
    }
}

#[derive(Clone)]
struct InterruptState {
    deadline: Option<Instant>,
    cancellation_token: Option<CancellationToken>,
    reason: Arc<AtomicU8>,
}

impl InterruptState {
    fn new(timeout: Option<Duration>, cancellation_token: Option<CancellationToken>) -> Self {
        Self {
            deadline: timeout.map(|duration| Instant::now() + duration),
            cancellation_token,
            reason: Arc::new(AtomicU8::new(InterruptReason::None as u8)),
        }
    }

    fn should_interrupt(&self) -> bool {
        if self
            .cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            self.set_reason(InterruptReason::Cancelled);
            return true;
        }

        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.set_reason(InterruptReason::Timeout);
            return true;
        }

        false
    }

    fn current_reason(&self) -> InterruptReason {
        match self.reason.load(Ordering::SeqCst) {
            1 => InterruptReason::Timeout,
            2 => InterruptReason::Cancelled,
            _ => InterruptReason::None,
        }
    }

    fn set_reason(&self, reason: InterruptReason) {
        let _ = self.reason.compare_exchange(
            InterruptReason::None as u8,
            reason as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterruptReason {
    None = 0,
    Timeout = 1,
    Cancelled = 2,
}

fn configured_status(configured: bool) -> CapabilityStatus {
    if configured {
        CapabilityStatus::Enforced
    } else {
        CapabilityStatus::SupportedNotConfigured
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostMethod {
    Get,
    Post,
    Connect,
    AjaxAll,
    Ajax,
    GetSource,
    GetString,
    GetStringList,
}

impl HostMethod {
    fn callback_name(self) -> &'static str {
        match self {
            Self::Get => "java.get",
            Self::Post => "java.post",
            Self::Connect => "java.connect",
            Self::AjaxAll => "java.ajaxAll",
            Self::Ajax => "java.ajax",
            Self::GetSource => "java.getSource",
            Self::GetString => "java.getString",
            Self::GetStringList => "java.getStringList",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "get" | "java.get" => Some(Self::Get),
            "post" | "java.post" => Some(Self::Post),
            "connect" | "java.connect" => Some(Self::Connect),
            "ajaxAll" | "java.ajaxAll" => Some(Self::AjaxAll),
            "ajax" | "java.ajax" => Some(Self::Ajax),
            "getSource" | "java.getSource" => Some(Self::GetSource),
            "getString" | "java.getString" => Some(Self::GetString),
            "getStringList" | "java.getStringList" => Some(Self::GetStringList),
            _ => None,
        }
    }

    fn validate_args<'js>(self, ctx: &Ctx<'js>, args: &[JsonValue]) -> Result<(), QuickJsError> {
        if self == Self::GetSource {
            if args.is_empty() {
                return Ok(());
            }

            return Err(Exception::throw_type(
                ctx,
                "java.getSource does not accept arguments",
            ));
        }

        if self == Self::AjaxAll {
            let Some(requests) = args.first() else {
                return Err(Exception::throw_type(
                    ctx,
                    "java.ajaxAll requires a request array argument",
                ));
            };

            if !requests.is_array() {
                return Err(Exception::throw_type(
                    ctx,
                    "java.ajaxAll request argument must be an array",
                ));
            }

            return Ok(());
        }

        if matches!(self, Self::GetString | Self::GetStringList) {
            let Some(rule) = args.first() else {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{} requires a rule string argument", self.callback_name()).as_str(),
                ));
            };

            if !rule.is_string() {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{} rule argument must be a string", self.callback_name()).as_str(),
                ));
            }

            return Ok(());
        }

        let Some(url) = args.first() else {
            return Err(Exception::throw_type(
                ctx,
                format!("{} requires a URL string argument", self.callback_name()).as_str(),
            ));
        };

        if !url.is_string() {
            return Err(Exception::throw_type(
                ctx,
                format!("{} URL argument must be a string", self.callback_name()).as_str(),
            ));
        }

        Ok(())
    }
}

fn make_host_callback<'js>(
    ctx: Ctx<'js>,
    method: HostMethod,
    registry: HostCallbackRegistry,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |ctx: Ctx<'js>,
              args: Rest<QuickJsValue<'js>>|
              -> Result<QuickJsValue<'js>, QuickJsError> {
            let json_args = host_args_to_json(&ctx, &args.0)?;
            method.validate_args(&ctx, &json_args)?;
            let descriptor = build_host_descriptor(method, &json_args)?;

            let result = registry
                .call(method.callback_name(), descriptor)
                .map_err(|error| {
                    Exception::throw_internal(
                        &ctx,
                        format!("host callback {} failed: {error}", method.callback_name())
                            .as_str(),
                    )
                })?;

            json_to_quickjs(&ctx, &result).map_err(|_| {
                Exception::throw_internal(
                    &ctx,
                    format!(
                        "host callback {} returned invalid JSON",
                        method.callback_name()
                    )
                    .as_str(),
                )
            })
        },
    )
}

fn make_buffer_object<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Object<'js>, QuickJsError> {
    let buffer = rquickjs::Object::new(ctx.clone())?;
    buffer.set(
        "concat",
        rquickjs::Function::new(
            ctx,
            |ctx: Ctx<'js>,
             args: Rest<QuickJsValue<'js>>|
             -> Result<QuickJsValue<'js>, QuickJsError> {
                let Some(chunks_value) = args.0.first() else {
                    return json_to_quickjs(&ctx, &JsonValue::Array(Vec::new()));
                };
                let chunks = match quickjs_value_to_json(chunks_value, 0) {
                    Ok(JsonValue::Array(chunks)) => chunks,
                    Ok(_) => {
                        return Err(Exception::throw_type(
                            &ctx,
                            "Buffer.concat list argument must be an array",
                        ));
                    }
                    Err(error) => {
                        return Err(Exception::throw_type(
                            &ctx,
                            format!("Buffer.concat list is not JSON-compatible: {error}").as_str(),
                        ));
                    }
                };

                let mut bytes = chunks
                    .into_iter()
                    .flat_map(json_value_to_byte_array)
                    .collect::<Vec<_>>();

                if let Some(total_length) = args
                    .0
                    .get(1)
                    .and_then(|value| quickjs_value_to_json(value, 0).ok())
                    .and_then(|value| value.as_u64())
                    .and_then(|value| usize::try_from(value).ok())
                {
                    bytes.resize(total_length, 0);
                }

                json_to_quickjs(&ctx, &bytes_to_json_array(bytes))
            },
        )?,
    )?;
    Ok(buffer)
}

fn make_base64_encode_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let Some(input_value) = args.0.first() else {
                return Ok(String::new());
            };
            let input = match quickjs_value_to_json(input_value, 0) {
                Ok(JsonValue::String(value)) => value,
                Ok(value) => value.to_string(),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!("java.base64Encode input is not JSON-compatible: {error}").as_str(),
                    ));
                }
            };
            let flags = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_i64())
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);

            Ok(base64_encode_with_flags(input.as_bytes(), flags))
        },
    )
}

fn make_base64_decode_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<Option<String>, QuickJsError> {
            let Some(input_value) = args.0.first() else {
                return Ok(None);
            };
            let input = match quickjs_value_to_json(input_value, 0) {
                Ok(JsonValue::String(value)) => value,
                Ok(value) => value.to_string(),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!("base64Decode input is not JSON-compatible: {error}").as_str(),
                    ));
                }
            };
            let decode_option = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .unwrap_or(JsonValue::Null);
            let flags = decode_option
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);
            let Some(bytes) = base64_decode_with_flags(&input, flags) else {
                return Ok(None);
            };

            if let Some(charset) = decode_option.as_str() {
                return Ok(decode_bytes_with_charset(bytes, charset));
            }

            Ok(String::from_utf8(bytes).ok())
        },
    )
}

fn make_base64_decode_to_byte_array_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<Option<Vec<u8>>, QuickJsError> {
            let Some(input_value) = args.0.first() else {
                return Ok(None);
            };
            let input = match quickjs_value_to_json(input_value, 0) {
                Ok(JsonValue::String(value)) => value,
                Ok(value) => value.to_string(),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!(
                            "java.base64DecodeToByteArray input is not JSON-compatible: {error}"
                        )
                        .as_str(),
                    ));
                }
            };
            let flags = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_i64())
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);

            Ok(base64_decode_with_flags(&input, flags))
        },
    )
}

fn make_hex_encode_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> String {
        hex_encode(input.as_bytes())
    })
}

fn make_hex_decode_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> Option<String> {
        let bytes = hex_decode(&input);
        String::from_utf8(bytes).ok()
    })
}

fn make_hex_decode_to_byte_array_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> Vec<u8> { hex_decode(&input) })
}

fn make_md5_encode_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> String { md5_hex(input.as_bytes()) })
}

fn make_md5_encode16_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> String {
        md5_hex(input.as_bytes())[8..24].to_string()
    })
}

fn make_hash_digest_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |_ctx: Ctx<'js>, input: String, algorithm: String| -> Result<String, QuickJsError> {
            Ok(hash_digest_hex(input.as_bytes(), &algorithm).unwrap_or_default())
        },
    )
}

fn make_hash_digest_base64_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |_ctx: Ctx<'js>, input: String, algorithm: String| -> Result<String, QuickJsError> {
            Ok(hash_digest_base64(input.as_bytes(), &algorithm).unwrap_or_default())
        },
    )
}

fn make_hmac_digest_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>,
         input: String,
         algorithm: String,
         key: String|
         -> Result<String, QuickJsError> {
            hmac_digest_hex(input.as_bytes(), &algorithm, key.as_bytes()).ok_or_else(|| {
                Exception::throw_type(
                    &ctx,
                    format!("hmacDigest unsupported algorithm: {algorithm}").as_str(),
                )
            })
        },
    )
}

fn make_hmac_base64_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>,
         input: String,
         algorithm: String,
         key: String|
         -> Result<String, QuickJsError> {
            hmac_digest_base64(input.as_bytes(), &algorithm, key.as_bytes()).ok_or_else(|| {
                Exception::throw_type(
                    &ctx,
                    format!("HMacBase64 unsupported algorithm: {algorithm}").as_str(),
                )
            })
        },
    )
}

fn make_create_sign_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let Some(params_value) = args.0.first() else {
                return Ok(String::new());
            };
            let params = quickjs_value_to_json(params_value, 0).map_err(|error| {
                Exception::throw_type(
                    &ctx,
                    format!("createSign params are not JSON-compatible: {error}").as_str(),
                )
            })?;
            let key = args
                .0
                .get(1)
                .map(|value| quickjs_value_to_string(value))
                .transpose()
                .map_err(|error| {
                    Exception::throw_type(
                        &ctx,
                        format!("createSign key is not JSON-compatible: {error}").as_str(),
                    )
                })?
                .unwrap_or_default();
            let algorithm = args
                .0
                .get(2)
                .map(|value| quickjs_value_to_string(value))
                .transpose()
                .map_err(|error| {
                    Exception::throw_type(
                        &ctx,
                        format!("createSign algorithm is not JSON-compatible: {error}").as_str(),
                    )
                })?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "HMAC-MD5".to_string());
            let payload = create_sign_payload(&params);
            if payload.is_empty() || key.is_empty() {
                return Ok(String::new());
            }

            hmac_digest_hex(payload.as_bytes(), &algorithm, key.as_bytes()).ok_or_else(|| {
                Exception::throw_type(
                    &ctx,
                    format!("createSign unsupported algorithm: {algorithm}").as_str(),
                )
            })
        },
    )
}

fn make_create_symmetric_crypto_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>,
         args: Rest<QuickJsValue<'js>>|
         -> Result<rquickjs::Object<'js>, QuickJsError> {
            let transformation = optional_quickjs_string(args.0.first(), "AES/CBC/PKCS7Padding");
            let key =
                optional_symmetric_key_bytes(&ctx, args.0.get(1), "createSymmetricCrypto key")?;
            let iv = optional_symmetric_key_bytes(&ctx, args.0.get(2), "createSymmetricCrypto iv")?;

            let crypto = rquickjs::Object::new(ctx.clone())?;
            let encrypt_transformation = transformation.clone();
            let encrypt_key = key.clone();
            let encrypt_iv = iv.clone();
            crypto.set(
                "encrypt",
                rquickjs::Function::new(ctx.clone(), move |input: String| -> String {
                    symmetric_encrypt_base64_with_key_bytes(
                        &encrypt_transformation,
                        &encrypt_key,
                        &encrypt_iv,
                        input.as_bytes(),
                    )
                    .unwrap_or_default()
                })?,
            )?;

            let encrypt_base64_transformation = transformation.clone();
            let encrypt_base64_key = key.clone();
            let encrypt_base64_iv = iv.clone();
            crypto.set(
                "encryptBase64",
                rquickjs::Function::new(ctx.clone(), move |input: String| -> String {
                    symmetric_encrypt_base64_with_key_bytes(
                        &encrypt_base64_transformation,
                        &encrypt_base64_key,
                        &encrypt_base64_iv,
                        input.as_bytes(),
                    )
                    .unwrap_or_default()
                })?,
            )?;

            let encrypt_hex_transformation = transformation.clone();
            let encrypt_hex_key = key.clone();
            let encrypt_hex_iv = iv.clone();
            crypto.set(
                "encryptHex",
                rquickjs::Function::new(ctx.clone(), move |input: String| -> String {
                    symmetric_encrypt_hex_with_key_bytes(
                        &encrypt_hex_transformation,
                        &encrypt_hex_key,
                        &encrypt_hex_iv,
                        input.as_bytes(),
                    )
                    .unwrap_or_default()
                })?,
            )?;

            let decrypt_hex_transformation = transformation.clone();
            let decrypt_hex_key = key.clone();
            let decrypt_hex_iv = iv.clone();
            crypto.set(
                "decryptHex",
                rquickjs::Function::new(ctx.clone(), move |input: String| -> String {
                    let encrypted = hex_decode(&input);
                    let decrypted = symmetric_decrypt_cipher_bytes_with_key_bytes(
                        &decrypt_hex_transformation,
                        &decrypt_hex_key,
                        &decrypt_hex_iv,
                        &encrypted,
                    )
                    .unwrap_or_default();
                    String::from_utf8(decrypted).unwrap_or_default()
                })?,
            )?;

            let decrypt_transformation = transformation.clone();
            let decrypt_key = key.clone();
            let decrypt_iv = iv.clone();
            crypto.set(
                "decryptStr",
                rquickjs::Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'js>,
                          args: Rest<QuickJsValue<'js>>|
                          -> Result<String, QuickJsError> {
                        let Some(input_value) = args.0.first() else {
                            return Ok(String::new());
                        };
                        let encrypted =
                            symmetric_cipher_input_bytes(&ctx, input_value, "decryptStr")?;
                        let decrypted = symmetric_decrypt_cipher_bytes_with_key_bytes(
                            &decrypt_transformation,
                            &decrypt_key,
                            &decrypt_iv,
                            &encrypted,
                        )
                        .unwrap_or_default();

                        Ok(String::from_utf8(decrypted).unwrap_or_default())
                    },
                )?,
            )?;

            let decrypt_alias_transformation = transformation;
            let decrypt_alias_key = key;
            let decrypt_alias_iv = iv;
            crypto.set(
                "decrypt",
                rquickjs::Function::new(
                    ctx,
                    move |ctx: Ctx<'js>,
                          args: Rest<QuickJsValue<'js>>|
                          -> Result<QuickJsValue<'js>, QuickJsError> {
                        let Some(input_value) = args.0.first() else {
                            return json_to_quickjs(&ctx, &JsonValue::Array(Vec::new()));
                        };
                        let input_was_string = input_value.is_string();
                        let encrypted = symmetric_cipher_input_bytes(&ctx, input_value, "decrypt")?;
                        let decrypted = symmetric_decrypt_cipher_bytes_with_key_bytes(
                            &decrypt_alias_transformation,
                            &decrypt_alias_key,
                            &decrypt_alias_iv,
                            &encrypted,
                        )
                        .unwrap_or_default();

                        if input_was_string {
                            return json_to_quickjs(
                                &ctx,
                                &JsonValue::String(
                                    String::from_utf8(decrypted).unwrap_or_default(),
                                ),
                            );
                        }

                        json_to_quickjs(&ctx, &bytes_to_json_array(decrypted))
                    },
                )?,
            )?;
            Ok(crypto)
        },
    )
}

fn make_aes_base64_decode_to_string_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let data = optional_quickjs_string(args.0.first(), "");
            let key = optional_quickjs_string(args.0.get(1), "");
            let transformation = optional_quickjs_string(args.0.get(2), "AES/CBC/PKCS7Padding");
            let iv = optional_quickjs_string(args.0.get(3), "");

            Ok(
                symmetric_decrypt_base64_to_string(&transformation, &key, &iv, &data)
                    .unwrap_or_default(),
            )
        },
    )
}

fn make_aes_encode_to_base64_string_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let data = optional_quickjs_string(args.0.first(), "");
            let key = optional_quickjs_string(args.0.get(1), "");
            let transformation = optional_quickjs_string(args.0.get(2), "AES/CBC/PKCS7Padding");
            let iv = optional_quickjs_string(args.0.get(3), "");

            Ok(
                symmetric_encrypt_base64(&transformation, &key, &iv, data.as_bytes())
                    .unwrap_or_default(),
            )
        },
    )
}

fn make_des_encode_to_base64_string_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let data = optional_quickjs_string(args.0.first(), "");
            let key = optional_quickjs_string(args.0.get(1), "");
            let transformation = optional_quickjs_string(args.0.get(2), "DES/CBC/PKCS5Padding");
            let iv = optional_quickjs_string(args.0.get(3), "");

            Ok(
                symmetric_encrypt_base64(&transformation, &key, &iv, data.as_bytes())
                    .unwrap_or_default(),
            )
        },
    )
}

fn make_triple_des_encode_base64_str_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let data = optional_quickjs_string(args.0.first(), "");
            let key = optional_quickjs_string(args.0.get(1), "");
            let mode = optional_quickjs_string(args.0.get(2), "CBC");
            let padding = optional_quickjs_string(args.0.get(3), "PKCS5Padding");
            let iv = optional_quickjs_string(args.0.get(4), "");
            let transformation = format!("DESede/{mode}/{padding}");

            Ok(
                symmetric_encrypt_base64(&transformation, &key, &iv, data.as_bytes())
                    .unwrap_or_default(),
            )
        },
    )
}

fn symmetric_cipher_input_bytes(
    ctx: &Ctx<'_>,
    value: &QuickJsValue<'_>,
    helper_name: &str,
) -> Result<Vec<u8>, QuickJsError> {
    match quickjs_value_to_json(value, 0) {
        Ok(JsonValue::String(value)) => Ok(base64_decode_with_flags(&value, 0).unwrap_or_default()),
        Ok(value) => Ok(json_value_to_byte_array(value)),
        Err(error) => Err(Exception::throw_type(
            ctx,
            format!("{helper_name} input is not JSON-compatible: {error}").as_str(),
        )),
    }
}

fn optional_symmetric_key_bytes(
    ctx: &Ctx<'_>,
    value: Option<&QuickJsValue<'_>>,
    helper_name: &str,
) -> Result<Vec<u8>, QuickJsError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    match quickjs_value_to_json(value, 0) {
        Ok(JsonValue::Null) => Ok(Vec::new()),
        Ok(JsonValue::String(value)) => Ok(value.into_bytes()),
        Ok(value @ (JsonValue::Array(_) | JsonValue::Number(_))) => {
            Ok(json_value_to_byte_array(value))
        }
        Ok(_) => Ok(Vec::new()),
        Err(error) => Err(Exception::throw_type(
            ctx,
            format!("{helper_name} is not JSON-compatible: {error}").as_str(),
        )),
    }
}

fn json_value_to_byte_array(value: JsonValue) -> Vec<u8> {
    match value {
        JsonValue::Array(items) => items
            .into_iter()
            .filter_map(|item| item.as_i64().map(|value| (value & 0xff) as u8))
            .collect(),
        JsonValue::Number(value) => value
            .as_i64()
            .map(|value| vec![(value & 0xff) as u8])
            .unwrap_or_default(),
        JsonValue::Null => Vec::new(),
        JsonValue::String(value) => value.into_bytes(),
        _ => Vec::new(),
    }
}

fn bytes_to_json_array(bytes: Vec<u8>) -> JsonValue {
    JsonValue::Array(
        bytes
            .into_iter()
            .map(|byte| JsonValue::Number(JsonNumber::from(byte)))
            .collect(),
    )
}

fn make_str_to_bytes_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let Some(input_value) = args.0.first() else {
                return Ok(String::new());
            };
            let input = match quickjs_value_to_json(input_value, 0) {
                Ok(JsonValue::String(value)) => value,
                Ok(value) => value.to_string(),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!("strToBytes input is not JSON-compatible: {error}").as_str(),
                    ));
                }
            };
            let charset = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "UTF-8".to_string());
            let bytes = encode_string_with_charset(&input, &charset);

            Ok(hex_encode(&bytes))
        },
    )
}

fn make_bytes_to_str_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let Some(input_value) = args.0.first() else {
                return Ok(String::new());
            };
            let bytes = match quickjs_value_to_json(input_value, 0) {
                Ok(value) => bytes_to_utf8_input(value),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!("bytesToStr input is not JSON-compatible: {error}").as_str(),
                    ));
                }
            };
            let charset = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "UTF-8".to_string());

            Ok(decode_bytes_with_charset(bytes, &charset).unwrap_or_default())
        },
    )
}

fn make_encode_uri_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let (input, charset) = parse_uri_encode_args(&ctx, args, "encodeURI")?;
            Ok(percent_encode_uri(&input, charset.as_deref()))
        },
    )
}

fn make_encode_uri_component_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let (input, charset) = parse_uri_encode_args(&ctx, args, "encodeURIComponent")?;
            Ok(percent_encode_uri_component(&input, charset.as_deref()))
        },
    )
}

fn parse_uri_encode_args<'js>(
    ctx: &Ctx<'js>,
    args: Rest<QuickJsValue<'js>>,
    helper_name: &str,
) -> Result<(String, Option<String>), QuickJsError> {
    let Some(input_value) = args.0.first() else {
        return Ok((String::new(), None));
    };
    let input = match quickjs_value_to_json(input_value, 0) {
        Ok(JsonValue::String(value)) => value,
        Ok(value) => value.to_string(),
        Err(error) => {
            return Err(Exception::throw_type(
                ctx,
                format!("{helper_name} input is not JSON-compatible: {error}").as_str(),
            ));
        }
    };
    let charset = args
        .0
        .get(1)
        .and_then(|value| quickjs_value_to_json(value, 0).ok())
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .filter(|value| !value.is_empty());

    Ok((input, charset))
}

fn make_time_format_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let time_ms = args
                .0
                .first()
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            let pattern = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| match value {
                    JsonValue::String(value) => Some(value),
                    JsonValue::Null => None,
                    value => Some(value.to_string()),
                })
                .unwrap_or_else(|| DEFAULT_TIME_FORMAT_PATTERN.to_string());

            Ok(time_format_local(time_ms, &pattern))
        },
    )
}

fn make_time_format_utc_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let time_ms = args
                .0
                .first()
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            let pattern = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .ok_or_else(|| {
                    Exception::throw_type(&ctx, "timeFormatUTC requires a format string argument")
                })?;
            let offset_ms = args
                .0
                .get(2)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_f64())
                .map(|value| value.trunc() as i64)
                .unwrap_or(0);

            Ok(time_format_utc(time_ms, &pattern, offset_ms))
        },
    )
}

fn make_to_num_chapter_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<Option<String>, QuickJsError> {
            let Some(input_value) = args.0.first() else {
                return Ok(None);
            };
            let input = match quickjs_value_to_json(input_value, 0) {
                Ok(JsonValue::Null) => return Ok(None),
                Ok(JsonValue::String(value)) => value,
                Ok(value) => value.to_string(),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!("toNumChapter input is not JSON-compatible: {error}").as_str(),
                    ));
                }
            };

            Ok(Some(to_num_chapter(&input)))
        },
    )
}

fn make_t2s_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> String { t2s(&input) })
}

fn make_s2t_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> String { s2t(&input) })
}

fn make_html_format_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |input: String| -> String { html_format(&input) })
}

fn make_to_url_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<QuickJsValue<'js>, QuickJsError> {
            let Some(url_value) = args.0.first() else {
                return Err(Exception::throw_type(
                    &ctx,
                    "toURL requires a URL string argument",
                ));
            };
            let url = match quickjs_value_to_json(url_value, 0) {
                Ok(JsonValue::String(value)) => value,
                Ok(value) => value.to_string(),
                Err(error) => {
                    return Err(Exception::throw_type(
                        &ctx,
                        format!("toURL input is not JSON-compatible: {error}").as_str(),
                    ));
                }
            };
            let url = url.trim().to_string();
            if url.is_empty() {
                return json_to_quickjs(&ctx, &JsonValue::String(String::new()));
            }
            let base_url = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_str().map(ToOwned::to_owned));
            if !is_absolute_url(&url) {
                let Some(base_url) = base_url.as_deref().filter(|value| !value.is_empty()) else {
                    return json_to_quickjs(&ctx, &JsonValue::String(url));
                };
                if parse_absolute_url(base_url).is_err() {
                    return json_to_quickjs(&ctx, &JsonValue::String(url));
                }
            }
            let parts = resolve_js_url(&url, base_url.as_deref()).map_err(|message| {
                Exception::throw_type(&ctx, format!("toURL failed: {message}").as_str())
            })?;
            Ok(js_url_to_object(&ctx, parts)?.into_value())
        },
    )
}

fn make_log_callback<'js>(
    ctx: Ctx<'js>,
    console: ConsoleBuffer,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |message: String| -> Result<String, QuickJsError> {
            let mut records = console.lock().map_err(|_| QuickJsError::Unknown)?;
            records.push(ConsoleRecord {
                level: ConsoleLevel::Log,
                args: vec![JsonValue::String(message.clone())],
            });
            Ok(message)
        },
    )
}

fn make_refresh_toc_url_callback<'js>(
    ctx: Ctx<'js>,
    console: ConsoleBuffer,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, move || -> Result<String, QuickJsError> {
        let mut records = console.lock().map_err(|_| QuickJsError::Unknown)?;
        records.push(ConsoleRecord {
            level: ConsoleLevel::Log,
            args: vec![JsonValue::String(REFRESH_TOC_URL_LOG_MESSAGE.to_string())],
        });
        Ok(String::new())
    })
}

fn make_toast_callback<'js>(
    ctx: Ctx<'js>,
    console: ConsoleBuffer,
    duration: ToastDuration,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |ctx: Ctx<'js>, args: Rest<QuickJsValue<'js>>| -> Result<String, QuickJsError> {
            let message = args
                .0
                .first()
                .map(|value| js_value_to_compat_string(value, &ctx))
                .transpose()?
                .unwrap_or_default();
            let mut records = console.lock().map_err(|_| QuickJsError::Unknown)?;
            records.push(ConsoleRecord {
                level: ConsoleLevel::Log,
                args: vec![
                    JsonValue::String(duration.marker().to_string()),
                    JsonValue::String(message),
                    JsonValue::String(duration.label().to_string()),
                ],
            });
            Ok(String::new())
        },
    )
}

fn make_log_type_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, |args: Rest<QuickJsValue<'js>>| -> String {
        args.0
            .first()
            .map(js_value_type_name)
            .unwrap_or_else(|| "undefined".to_string())
    })
}

fn make_random_uuid_callback<'js>(ctx: Ctx<'js>) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, || -> String { random_uuid_v4() })
}

fn make_get_web_view_ua_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(ctx, || DEFAULT_WEB_VIEW_UA.to_string())
}

fn make_host_dispatch_callback<'js>(
    ctx: Ctx<'js>,
    registry: HostCallbackRegistry,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |ctx: Ctx<'js>,
              args: Rest<QuickJsValue<'js>>|
              -> Result<QuickJsValue<'js>, QuickJsError> {
            let mut json_args = host_args_to_json(&ctx, &args.0)?;
            let Some(method_name) = json_args.first().and_then(JsonValue::as_str) else {
                return Err(Exception::throw_type(
                    &ctx,
                    "java.call requires the host method name as its first string argument",
                ));
            };
            let Some(method) = HostMethod::from_name(method_name) else {
                return Err(Exception::throw_type(
                    &ctx,
                    format!("unknown host method: {method_name}").as_str(),
                ));
            };

            json_args.remove(0);
            method.validate_args(&ctx, &json_args)?;
            let descriptor = build_host_descriptor(method, &json_args)?;
            let result = registry
                .call(method.callback_name(), descriptor)
                .map_err(|error| {
                    Exception::throw_internal(
                        &ctx,
                        format!("host callback {} failed: {error}", method.callback_name())
                            .as_str(),
                    )
                })?;

            json_to_quickjs(&ctx, &result).map_err(|_| {
                Exception::throw_internal(
                    &ctx,
                    format!(
                        "host callback {} returned invalid JSON",
                        method.callback_name()
                    )
                    .as_str(),
                )
            })
        },
    )
}

fn host_args_to_json<'js>(
    ctx: &Ctx<'js>,
    args: &[QuickJsValue<'js>],
) -> Result<Vec<JsonValue>, QuickJsError> {
    let mut json_args = Vec::with_capacity(args.len());
    for arg in args.iter() {
        match quickjs_value_to_json(arg, 0) {
            Ok(value) => json_args.push(value),
            Err(error) => {
                return Err(Exception::throw_type(
                    ctx,
                    format!("host callback argument is not JSON-compatible: {error}").as_str(),
                ));
            }
        }
    }
    Ok(json_args)
}

fn make_console_callback<'js>(
    ctx: Ctx<'js>,
    level: ConsoleLevel,
    console: ConsoleBuffer,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |args: Rest<QuickJsValue<'js>>| -> Result<(), QuickJsError> {
            let record = ConsoleRecord {
                level: level.clone(),
                args: args.0.iter().map(console_arg_to_json).collect(),
            };
            let mut records = console.lock().map_err(|_| QuickJsError::Unknown)?;
            records.push(record);
            Ok(())
        },
    )
}

fn console_arg_to_json(value: &QuickJsValue<'_>) -> JsonValue {
    quickjs_value_to_json(value, 0).unwrap_or_else(|_| {
        let mut fallback = JsonMap::new();
        fallback.insert(
            "type".to_string(),
            JsonValue::String(value.type_name().to_string()),
        );
        fallback.insert(
            "display".to_string(),
            JsonValue::String(format!("[{}]", value.type_name())),
        );
        JsonValue::Object(fallback)
    })
}

fn js_value_to_compat_string(
    value: &QuickJsValue<'_>,
    ctx: &Ctx<'_>,
) -> Result<String, QuickJsError> {
    match quickjs_value_to_json(value, 0) {
        Ok(JsonValue::Null) => Ok(String::new()),
        Ok(JsonValue::String(value)) => Ok(value),
        Ok(value) => Ok(value.to_string()),
        Err(error) => Err(Exception::throw_type(
            ctx,
            format!("toast input is not JSON-compatible: {error}").as_str(),
        )),
    }
}

fn random_uuid_v4() -> String {
    let counter = UUID_COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut state = (timestamp as u64)
        ^ ((timestamp >> 64) as u64).rotate_left(17)
        ^ counter.wrapping_mul(0x9e37_79b9_7f4a_7c15);

    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let value = splitmix64(&mut state).to_be_bytes();
        chunk.copy_from_slice(&value[..chunk.len()]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format_uuid_bytes(&bytes)
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn format_uuid_bytes(bytes: &[u8; 16]) -> String {
    let mut output = String::with_capacity(36);
    for (index, byte) in bytes.iter().copied().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            output.push('-');
        }
        output.push(HEX_TABLE[(byte >> 4) as usize] as char);
        output.push(HEX_TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode_with_flags(input: &[u8], flags: i32) -> String {
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);

        output.push(BASE64_TABLE[(first >> 2) as usize] as char);
        output.push(BASE64_TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(
                BASE64_TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char,
            );
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(BASE64_TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }

    if flags & 8 != 0 {
        output = output.replace('+', "-").replace('/', "_");
    }
    if flags & 1 != 0 {
        output.truncate(output.trim_end_matches('=').len());
    }

    output
}

fn base64_decode_with_flags(input: &str, flags: i32) -> Option<Vec<u8>> {
    let mut bytes = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.is_empty() {
        return Some(Vec::new());
    }

    if flags & 8 != 0 || bytes.iter().any(|byte| matches!(byte, b'-' | b'_')) {
        for byte in bytes.iter_mut() {
            match *byte {
                b'-' => *byte = b'+',
                b'_' => *byte = b'/',
                _ => {}
            }
        }
    }

    match bytes.len() % 4 {
        0 => {}
        2 => bytes.extend_from_slice(b"=="),
        3 => bytes.push(b'='),
        _ => return None,
    }

    let mut output = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunk_count = bytes.len() / 4;
    for (chunk_index, chunk) in bytes.chunks_exact(4).enumerate() {
        let mut values = [0u8; 4];
        let mut padding = 0usize;
        for (index, byte) in chunk.iter().copied().enumerate() {
            if byte == b'=' {
                padding += 1;
                values[index] = 0;
            } else {
                if padding > 0 {
                    return None;
                }
                values[index] = base64_value(byte)?;
            }
        }
        if padding > 0 && chunk_index + 1 != chunk_count {
            return None;
        }

        output.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if padding < 1 {
            output.push((values[2] << 6) | values[3]);
        }
    }

    Some(output)
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

const HEX_TABLE: &[u8; 16] = b"0123456789abcdef";

fn hex_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input.iter().copied() {
        output.push(HEX_TABLE[(byte >> 4) as usize] as char);
        output.push(HEX_TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(input: &str) -> Vec<u8> {
    let bytes = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let mut output = Vec::with_capacity(bytes.len().div_ceil(2));

    for chunk in bytes.chunks(2) {
        match chunk {
            [single] => {
                if let Some(value) = hex_value(*single) {
                    output.push(value);
                }
            }
            [high, low] => {
                if let (Some(high), Some(low)) = (hex_value(*high), hex_value(*low)) {
                    output.push((high << 4) | low);
                }
            }
            _ => {}
        }
    }

    output
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymmetricMode {
    Cbc,
    Ecb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymmetricAlgorithm {
    Aes,
    Des,
    TripleDes,
    Sm4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymmetricPadding {
    Pkcs7,
    NoPadding,
    ZeroPadding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SymmetricTransformation {
    algorithm: SymmetricAlgorithm,
    mode: SymmetricMode,
    padding: SymmetricPadding,
}

fn symmetric_encrypt_base64(
    transformation: &str,
    key: &str,
    iv: &str,
    input: &[u8],
) -> Option<String> {
    symmetric_encrypt_base64_with_key_bytes(transformation, key.as_bytes(), iv.as_bytes(), input)
}

fn symmetric_encrypt_base64_with_key_bytes(
    transformation: &str,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
) -> Option<String> {
    let encrypted = symmetric_encrypt_bytes_with_key_bytes(transformation, key, iv, input)?;
    Some(base64_encode_with_flags(&encrypted, 0))
}

fn symmetric_encrypt_hex_with_key_bytes(
    transformation: &str,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
) -> Option<String> {
    let encrypted = symmetric_encrypt_bytes_with_key_bytes(transformation, key, iv, input)?;
    Some(hex_encode(&encrypted))
}

fn symmetric_encrypt_bytes_with_key_bytes(
    transformation: &str,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
) -> Option<Vec<u8>> {
    let transformation = parse_symmetric_transformation(transformation)?;
    match transformation.algorithm {
        SymmetricAlgorithm::Aes => aes_encrypt_bytes(input, key, iv, transformation),
        SymmetricAlgorithm::Des => des_encrypt_bytes(input, key, iv, transformation),
        SymmetricAlgorithm::TripleDes => triple_des_encrypt_bytes(input, key, iv, transformation),
        SymmetricAlgorithm::Sm4 => sm4_encrypt_bytes(input, key, iv, transformation),
    }
}

fn symmetric_decrypt_base64_to_string(
    transformation: &str,
    key: &str,
    iv: &str,
    input: &str,
) -> Option<String> {
    let encrypted = base64_decode_with_flags(input, 0)?;
    let decrypted = symmetric_decrypt_cipher_bytes(transformation, key, iv, &encrypted)?;
    String::from_utf8(decrypted).ok()
}

fn symmetric_decrypt_cipher_bytes(
    transformation: &str,
    key: &str,
    iv: &str,
    encrypted: &[u8],
) -> Option<Vec<u8>> {
    symmetric_decrypt_cipher_bytes_with_key_bytes(
        transformation,
        key.as_bytes(),
        iv.as_bytes(),
        encrypted,
    )
}

fn symmetric_decrypt_cipher_bytes_with_key_bytes(
    transformation: &str,
    key: &[u8],
    iv: &[u8],
    encrypted: &[u8],
) -> Option<Vec<u8>> {
    let transformation = parse_symmetric_transformation(transformation)?;
    match transformation.algorithm {
        SymmetricAlgorithm::Aes => aes_decrypt_bytes(encrypted, key, iv, transformation),
        SymmetricAlgorithm::Des => des_decrypt_bytes(encrypted, key, iv, transformation),
        SymmetricAlgorithm::TripleDes => {
            triple_des_decrypt_bytes(encrypted, key, iv, transformation)
        }
        SymmetricAlgorithm::Sm4 => sm4_decrypt_bytes(encrypted, key, iv, transformation),
    }
}

fn parse_symmetric_transformation(raw: &str) -> Option<SymmetricTransformation> {
    let parts = raw.trim().split('/').collect::<Vec<_>>();
    let algorithm = match normalize_crypto_part(parts.first().copied().unwrap_or("AES")).as_str() {
        "AES" => SymmetricAlgorithm::Aes,
        "DES" => SymmetricAlgorithm::Des,
        "DESEDE" | "DESEDE3" | "3DES" | "TRIPLEDES" | "DES3" => SymmetricAlgorithm::TripleDes,
        "SM4" => SymmetricAlgorithm::Sm4,
        _ => return None,
    };

    let mode = match normalize_crypto_part(parts.get(1).copied().unwrap_or("CBC")).as_str() {
        "CBC" => SymmetricMode::Cbc,
        "ECB" => SymmetricMode::Ecb,
        _ => return None,
    };
    let padding =
        match normalize_crypto_part(parts.get(2).copied().unwrap_or("PKCS7PADDING")).as_str() {
            "PKCS7PADDING" | "PKCS5PADDING" | "PKCS7" | "PKCS5" => SymmetricPadding::Pkcs7,
            "NOPADDING" => SymmetricPadding::NoPadding,
            "ZEROPADDING" | "ZERO" => SymmetricPadding::ZeroPadding,
            _ => return None,
        };

    Some(SymmetricTransformation {
        algorithm,
        mode,
        padding,
    })
}

fn normalize_crypto_part(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|ch| *ch != '-' && *ch != '_' && !ch.is_ascii_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

fn aes_encrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if !matches!(key.len(), 16 | 24 | 32) {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 16 {
        return None;
    }

    let mut input = input.to_vec();
    apply_symmetric_padding(&mut input, transformation.padding, 16)?;
    let round_keys = expand_aes_key(key)?;
    let rounds = aes_rounds_for_key(key)?;
    let mut previous = [0u8; 16];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
        }
        aes_encrypt_block(&mut block, &round_keys, rounds);
        if transformation.mode == SymmetricMode::Cbc {
            previous = block;
        }
        output.extend_from_slice(&block);
    }

    Some(output)
}

fn aes_decrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if !matches!(key.len(), 16 | 24 | 32) || input.len() % 16 != 0 {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 16 {
        return None;
    }

    let round_keys = expand_aes_key(key)?;
    let rounds = aes_rounds_for_key(key)?;
    let mut previous = [0u8; 16];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        let encrypted_block = block;
        aes_decrypt_block(&mut block, &round_keys, rounds);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
            previous = encrypted_block;
        }
        output.extend_from_slice(&block);
    }

    remove_symmetric_padding(&mut output, transformation.padding, 16)?;
    Some(output)
}

fn apply_symmetric_padding(
    input: &mut Vec<u8>,
    padding: SymmetricPadding,
    block_size: usize,
) -> Option<()> {
    match padding {
        SymmetricPadding::NoPadding => (input.len() % block_size == 0).then_some(()),
        SymmetricPadding::ZeroPadding => {
            let remainder = input.len() % block_size;
            if remainder != 0 {
                input.extend(std::iter::repeat(0).take(block_size - remainder));
            }
            Some(())
        }
        SymmetricPadding::Pkcs7 => {
            let pad_len = block_size - (input.len() % block_size);
            let pad_len = if pad_len == 0 { block_size } else { pad_len };
            input.extend(std::iter::repeat(pad_len as u8).take(pad_len));
            Some(())
        }
    }
}

fn remove_symmetric_padding(
    input: &mut Vec<u8>,
    padding: SymmetricPadding,
    block_size: usize,
) -> Option<()> {
    match padding {
        SymmetricPadding::NoPadding => Some(()),
        SymmetricPadding::ZeroPadding => {
            while input.last().is_some_and(|byte| *byte == 0) {
                input.pop();
            }
            Some(())
        }
        SymmetricPadding::Pkcs7 => {
            let pad_len = usize::from(*input.last()?);
            if pad_len == 0 || pad_len > block_size || pad_len > input.len() {
                return None;
            }
            if !input[input.len() - pad_len..]
                .iter()
                .all(|byte| usize::from(*byte) == pad_len)
            {
                return None;
            }
            input.truncate(input.len() - pad_len);
            Some(())
        }
    }
}

fn des_encrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if key.len() != 8 {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 8 {
        return None;
    }

    let mut input = input.to_vec();
    apply_symmetric_padding(&mut input, transformation.padding, 8)?;
    let subkeys = expand_des_key(key)?;
    let mut previous = [0u8; 8];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(chunk);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
        }
        des_crypt_block(&mut block, &subkeys, false);
        if transformation.mode == SymmetricMode::Cbc {
            previous = block;
        }
        output.extend_from_slice(&block);
    }

    Some(output)
}

fn des_decrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if key.len() != 8 || input.len() % 8 != 0 {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 8 {
        return None;
    }

    let subkeys = expand_des_key(key)?;
    let mut previous = [0u8; 8];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(chunk);
        let encrypted_block = block;
        des_crypt_block(&mut block, &subkeys, true);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
            previous = encrypted_block;
        }
        output.extend_from_slice(&block);
    }

    remove_symmetric_padding(&mut output, transformation.padding, 8)?;
    Some(output)
}

fn triple_des_encrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if !matches!(key.len(), 16 | 24) {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 8 {
        return None;
    }

    let mut input = input.to_vec();
    apply_symmetric_padding(&mut input, transformation.padding, 8)?;
    let key_schedule = expand_triple_des_key(key)?;
    let mut previous = [0u8; 8];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(chunk);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
        }
        triple_des_crypt_block(&mut block, &key_schedule, false);
        if transformation.mode == SymmetricMode::Cbc {
            previous = block;
        }
        output.extend_from_slice(&block);
    }

    Some(output)
}

fn triple_des_decrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if !matches!(key.len(), 16 | 24) || input.len() % 8 != 0 {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 8 {
        return None;
    }

    let key_schedule = expand_triple_des_key(key)?;
    let mut previous = [0u8; 8];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(chunk);
        let encrypted_block = block;
        triple_des_crypt_block(&mut block, &key_schedule, true);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
            previous = encrypted_block;
        }
        output.extend_from_slice(&block);
    }

    remove_symmetric_padding(&mut output, transformation.padding, 8)?;
    Some(output)
}

fn sm4_encrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if key.len() != 16 {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 16 {
        return None;
    }

    let mut input = input.to_vec();
    apply_symmetric_padding(&mut input, transformation.padding, 16)?;
    let round_keys = expand_sm4_key(key)?;
    let mut previous = [0u8; 16];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
        }
        sm4_crypt_block(&mut block, &round_keys);
        if transformation.mode == SymmetricMode::Cbc {
            previous = block;
        }
        output.extend_from_slice(&block);
    }

    Some(output)
}

fn sm4_decrypt_bytes(
    input: &[u8],
    key: &[u8],
    iv: &[u8],
    transformation: SymmetricTransformation,
) -> Option<Vec<u8>> {
    if key.len() != 16 || input.len() % 16 != 0 {
        return None;
    }
    if transformation.mode == SymmetricMode::Cbc && iv.len() != 16 {
        return None;
    }

    let mut round_keys = expand_sm4_key(key)?;
    round_keys.reverse();
    let mut previous = [0u8; 16];
    if transformation.mode == SymmetricMode::Cbc {
        previous.copy_from_slice(iv);
    }

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        let encrypted_block = block;
        sm4_crypt_block(&mut block, &round_keys);
        if transformation.mode == SymmetricMode::Cbc {
            for (byte, previous_byte) in block.iter_mut().zip(previous.iter()) {
                *byte ^= *previous_byte;
            }
            previous = encrypted_block;
        }
        output.extend_from_slice(&block);
    }

    remove_symmetric_padding(&mut output, transformation.padding, 16)?;
    Some(output)
}

fn expand_des_key(key: &[u8]) -> Option<[u64; 16]> {
    let key: [u8; 8] = key.try_into().ok()?;
    let permuted = des_permute(u64::from_be_bytes(key), 64, &DES_PC1);
    let mut c = ((permuted >> 28) & 0x0fff_ffff) as u32;
    let mut d = (permuted & 0x0fff_ffff) as u32;
    let mut subkeys = [0u64; 16];

    for (round, shift) in DES_KEY_SHIFTS.iter().copied().enumerate() {
        c = des_rotate_28(c, shift);
        d = des_rotate_28(d, shift);
        let combined = ((u64::from(c)) << 28) | u64::from(d);
        subkeys[round] = des_permute(combined, 56, &DES_PC2);
    }

    Some(subkeys)
}

fn expand_triple_des_key(key: &[u8]) -> Option<[[u64; 16]; 3]> {
    let first = expand_des_key(key.get(0..8)?)?;
    let second = expand_des_key(key.get(8..16)?)?;
    let third = if key.len() == 24 {
        expand_des_key(key.get(16..24)?)?
    } else {
        first
    };

    Some([first, second, third])
}

fn triple_des_crypt_block(block: &mut [u8; 8], key_schedule: &[[u64; 16]; 3], decrypt: bool) {
    if decrypt {
        des_crypt_block(block, &key_schedule[2], true);
        des_crypt_block(block, &key_schedule[1], false);
        des_crypt_block(block, &key_schedule[0], true);
    } else {
        des_crypt_block(block, &key_schedule[0], false);
        des_crypt_block(block, &key_schedule[1], true);
        des_crypt_block(block, &key_schedule[2], false);
    }
}

fn expand_sm4_key(key: &[u8]) -> Option<[u32; 32]> {
    let key: [u8; 16] = key.try_into().ok()?;
    let mut state = [0u32; 36];
    for index in 0..4 {
        state[index] = load_be_u32(&key, index * 4) ^ SM4_FK[index];
    }

    let mut round_keys = [0u32; 32];
    for index in 0..32 {
        let next = state[index]
            ^ sm4_key_transform(
                state[index + 1] ^ state[index + 2] ^ state[index + 3] ^ SM4_CK[index],
            );
        state[index + 4] = next;
        round_keys[index] = next;
    }

    Some(round_keys)
}

fn sm4_crypt_block(block: &mut [u8; 16], round_keys: &[u32; 32]) {
    let mut state = [0u32; 36];
    for index in 0..4 {
        state[index] = load_be_u32(block, index * 4);
    }
    for index in 0..32 {
        state[index + 4] = state[index]
            ^ sm4_round_transform(
                state[index + 1] ^ state[index + 2] ^ state[index + 3] ^ round_keys[index],
            );
    }

    store_be_u32(&mut block[0..4], state[35]);
    store_be_u32(&mut block[4..8], state[34]);
    store_be_u32(&mut block[8..12], state[33]);
    store_be_u32(&mut block[12..16], state[32]);
}

fn sm4_round_transform(value: u32) -> u32 {
    let substituted = sm4_substitute(value);
    substituted
        ^ substituted.rotate_left(2)
        ^ substituted.rotate_left(10)
        ^ substituted.rotate_left(18)
        ^ substituted.rotate_left(24)
}

fn sm4_key_transform(value: u32) -> u32 {
    let substituted = sm4_substitute(value);
    substituted ^ substituted.rotate_left(13) ^ substituted.rotate_left(23)
}

fn sm4_substitute(value: u32) -> u32 {
    let b0 = u32::from(SM4_SBOX[((value >> 24) & 0xff) as usize]);
    let b1 = u32::from(SM4_SBOX[((value >> 16) & 0xff) as usize]);
    let b2 = u32::from(SM4_SBOX[((value >> 8) & 0xff) as usize]);
    let b3 = u32::from(SM4_SBOX[(value & 0xff) as usize]);
    (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
}

fn load_be_u32(bytes: &[u8], offset: usize) -> u32 {
    (u32::from(bytes[offset]) << 24)
        | (u32::from(bytes[offset + 1]) << 16)
        | (u32::from(bytes[offset + 2]) << 8)
        | u32::from(bytes[offset + 3])
}

fn store_be_u32(output: &mut [u8], value: u32) {
    output[0] = ((value >> 24) & 0xff) as u8;
    output[1] = ((value >> 16) & 0xff) as u8;
    output[2] = ((value >> 8) & 0xff) as u8;
    output[3] = (value & 0xff) as u8;
}

fn des_rotate_28(value: u32, shift: u8) -> u32 {
    ((value << shift) | (value >> (28 - shift))) & 0x0fff_ffff
}

fn des_crypt_block(block: &mut [u8; 8], subkeys: &[u64; 16], decrypt: bool) {
    let permuted = des_permute(u64::from_be_bytes(*block), 64, &DES_IP);
    let mut left = (permuted >> 32) as u32;
    let mut right = permuted as u32;

    for round in 0..16 {
        let subkey = if decrypt {
            subkeys[15 - round]
        } else {
            subkeys[round]
        };
        let next_left = right;
        let next_right = left ^ des_feistel(right, subkey);
        left = next_left;
        right = next_right;
    }

    let preoutput = (u64::from(right) << 32) | u64::from(left);
    *block = des_permute(preoutput, 64, &DES_FP).to_be_bytes();
}

fn des_feistel(right: u32, subkey: u64) -> u32 {
    let expanded = des_permute(u64::from(right), 32, &DES_E) ^ subkey;
    let mut sboxed = 0u32;
    for box_index in 0..8 {
        let shift = 42usize - (box_index * 6);
        let value = ((expanded >> shift) & 0x3f) as u8;
        let row = usize::from(((value & 0x20) >> 4) | (value & 0x01));
        let column = usize::from((value >> 1) & 0x0f);
        sboxed = (sboxed << 4) | u32::from(DES_SBOXES[box_index][row * 16 + column]);
    }

    des_permute(u64::from(sboxed), 32, &DES_P) as u32
}

fn des_permute(input: u64, input_bits: u8, table: &[u8]) -> u64 {
    let mut output = 0u64;
    for position in table.iter().copied() {
        output <<= 1;
        output |= (input >> usize::from(input_bits - position)) & 1;
    }
    output
}

fn aes_rounds_for_key(key: &[u8]) -> Option<usize> {
    match key.len() {
        16 => Some(10),
        24 => Some(12),
        32 => Some(14),
        _ => None,
    }
}

fn expand_aes_key(key: &[u8]) -> Option<Vec<u8>> {
    let key_words = key.len() / 4;
    let rounds = aes_rounds_for_key(key)?;
    let expanded_len = 16 * (rounds + 1);
    let mut expanded = vec![0u8; expanded_len];
    expanded[..key.len()].copy_from_slice(key);

    let mut bytes_generated = key.len();
    let mut rcon_index = 1usize;
    let mut temp = [0u8; 4];
    while bytes_generated < expanded_len {
        temp.copy_from_slice(&expanded[bytes_generated - 4..bytes_generated]);
        if bytes_generated % key.len() == 0 {
            temp.rotate_left(1);
            for byte in temp.iter_mut() {
                *byte = AES_SBOX[usize::from(*byte)];
            }
            temp[0] ^= AES_RCON[rcon_index];
            rcon_index += 1;
        } else if key_words > 6 && bytes_generated % key.len() == 16 {
            for byte in temp.iter_mut() {
                *byte = AES_SBOX[usize::from(*byte)];
            }
        }

        for temp_byte in temp {
            expanded[bytes_generated] = expanded[bytes_generated - key.len()] ^ temp_byte;
            bytes_generated += 1;
        }
    }

    Some(expanded)
}

fn aes_encrypt_block(state: &mut [u8; 16], round_keys: &[u8], rounds: usize) {
    aes_add_round_key(state, round_keys, 0);
    for round in 1..rounds {
        aes_sub_bytes(state);
        aes_shift_rows(state);
        aes_mix_columns(state);
        aes_add_round_key(state, round_keys, round);
    }
    aes_sub_bytes(state);
    aes_shift_rows(state);
    aes_add_round_key(state, round_keys, rounds);
}

fn aes_decrypt_block(state: &mut [u8; 16], round_keys: &[u8], rounds: usize) {
    aes_add_round_key(state, round_keys, rounds);
    for round in (1..rounds).rev() {
        aes_inv_shift_rows(state);
        aes_inv_sub_bytes(state);
        aes_add_round_key(state, round_keys, round);
        aes_inv_mix_columns(state);
    }
    aes_inv_shift_rows(state);
    aes_inv_sub_bytes(state);
    aes_add_round_key(state, round_keys, 0);
}

fn aes_add_round_key(state: &mut [u8; 16], round_keys: &[u8], round: usize) {
    let offset = round * 16;
    for (index, byte) in state.iter_mut().enumerate() {
        *byte ^= round_keys[offset + index];
    }
}

fn aes_sub_bytes(state: &mut [u8; 16]) {
    for byte in state.iter_mut() {
        *byte = AES_SBOX[usize::from(*byte)];
    }
}

fn aes_inv_sub_bytes(state: &mut [u8; 16]) {
    for byte in state.iter_mut() {
        *byte = AES_INV_SBOX[usize::from(*byte)];
    }
}

fn aes_shift_rows(state: &mut [u8; 16]) {
    let original = *state;
    state[1] = original[5];
    state[5] = original[9];
    state[9] = original[13];
    state[13] = original[1];
    state[2] = original[10];
    state[6] = original[14];
    state[10] = original[2];
    state[14] = original[6];
    state[3] = original[15];
    state[7] = original[3];
    state[11] = original[7];
    state[15] = original[11];
}

fn aes_inv_shift_rows(state: &mut [u8; 16]) {
    let original = *state;
    state[1] = original[13];
    state[5] = original[1];
    state[9] = original[5];
    state[13] = original[9];
    state[2] = original[10];
    state[6] = original[14];
    state[10] = original[2];
    state[14] = original[6];
    state[3] = original[7];
    state[7] = original[11];
    state[11] = original[15];
    state[15] = original[3];
}

fn aes_mix_columns(state: &mut [u8; 16]) {
    for column in 0..4 {
        let offset = column * 4;
        let a0 = state[offset];
        let a1 = state[offset + 1];
        let a2 = state[offset + 2];
        let a3 = state[offset + 3];
        state[offset] = aes_gmul(a0, 2) ^ aes_gmul(a1, 3) ^ a2 ^ a3;
        state[offset + 1] = a0 ^ aes_gmul(a1, 2) ^ aes_gmul(a2, 3) ^ a3;
        state[offset + 2] = a0 ^ a1 ^ aes_gmul(a2, 2) ^ aes_gmul(a3, 3);
        state[offset + 3] = aes_gmul(a0, 3) ^ a1 ^ a2 ^ aes_gmul(a3, 2);
    }
}

fn aes_inv_mix_columns(state: &mut [u8; 16]) {
    for column in 0..4 {
        let offset = column * 4;
        let a0 = state[offset];
        let a1 = state[offset + 1];
        let a2 = state[offset + 2];
        let a3 = state[offset + 3];
        state[offset] = aes_gmul(a0, 14) ^ aes_gmul(a1, 11) ^ aes_gmul(a2, 13) ^ aes_gmul(a3, 9);
        state[offset + 1] =
            aes_gmul(a0, 9) ^ aes_gmul(a1, 14) ^ aes_gmul(a2, 11) ^ aes_gmul(a3, 13);
        state[offset + 2] =
            aes_gmul(a0, 13) ^ aes_gmul(a1, 9) ^ aes_gmul(a2, 14) ^ aes_gmul(a3, 11);
        state[offset + 3] =
            aes_gmul(a0, 11) ^ aes_gmul(a1, 13) ^ aes_gmul(a2, 9) ^ aes_gmul(a3, 14);
    }
}

fn aes_gmul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0u8;
    for _ in 0..8 {
        if b & 1 != 0 {
            product ^= a;
        }
        let high_bit = a & 0x80;
        a <<= 1;
        if high_bit != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    product
}

const DES_IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6,
    64, 56, 48, 40, 32, 24, 16, 8, 57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3, 61,
    53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];

const DES_FP: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31, 38, 6, 46, 14, 54, 22, 62, 30,
    37, 5, 45, 13, 53, 21, 61, 29, 36, 4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27,
    34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];

const DES_E: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18,
    19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];

const DES_P: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19,
    13, 30, 6, 22, 11, 4, 25,
];

const DES_PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60,
    52, 44, 36, 63, 55, 47, 39, 31, 23, 15, 7, 62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29,
    21, 13, 5, 28, 20, 12, 4,
];

const DES_PC2: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10, 23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2, 41, 52,
    31, 37, 47, 55, 30, 40, 51, 45, 33, 48, 44, 49, 39, 56, 34, 53, 46, 42, 50, 36, 29, 32,
];

const DES_KEY_SHIFTS: [u8; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

const DES_SBOXES: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12,
        11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9,
        1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1,
        10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15,
        4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5,
        14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6,
        9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2,
        12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 1,
        13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15,
        10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14,
        2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13,
        14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5,
        15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5,
        12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4,
        10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6,
        11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10,
        8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];

const SM4_FK: [u32; 4] = [0xa3b1_bac6, 0x56aa_3350, 0x677d_9197, 0xb270_22dc];

const SM4_CK: [u32; 32] = [
    0x0007_0e15,
    0x1c23_2a31,
    0x383f_464d,
    0x545b_6269,
    0x7077_7e85,
    0x8c93_9aa1,
    0xa8af_b6bd,
    0xc4cb_d2d9,
    0xe0e7_eef5,
    0xfc03_0a11,
    0x181f_262d,
    0x343b_4249,
    0x5057_5e65,
    0x6c73_7a81,
    0x888f_969d,
    0xa4ab_b2b9,
    0xc0c7_ced5,
    0xdce3_eaf1,
    0xf8ff_060d,
    0x141b_2229,
    0x3037_3e45,
    0x4c53_5a61,
    0x686f_767d,
    0x848b_9299,
    0xa0a7_aeb5,
    0xbcc3_cad1,
    0xd8df_e6ed,
    0xf4fb_0209,
    0x1017_1e25,
    0x2c33_3a41,
    0x484f_565d,
    0x646b_7279,
];

const SM4_SBOX: [u8; 256] = [
    0xd6, 0x90, 0xe9, 0xfe, 0xcc, 0xe1, 0x3d, 0xb7, 0x16, 0xb6, 0x14, 0xc2, 0x28, 0xfb, 0x2c, 0x05,
    0x2b, 0x67, 0x9a, 0x76, 0x2a, 0xbe, 0x04, 0xc3, 0xaa, 0x44, 0x13, 0x26, 0x49, 0x86, 0x06, 0x99,
    0x9c, 0x42, 0x50, 0xf4, 0x91, 0xef, 0x98, 0x7a, 0x33, 0x54, 0x0b, 0x43, 0xed, 0xcf, 0xac, 0x62,
    0xe4, 0xb3, 0x1c, 0xa9, 0xc9, 0x08, 0xe8, 0x95, 0x80, 0xdf, 0x94, 0xfa, 0x75, 0x8f, 0x3f, 0xa6,
    0x47, 0x07, 0xa7, 0xfc, 0xf3, 0x73, 0x17, 0xba, 0x83, 0x59, 0x3c, 0x19, 0xe6, 0x85, 0x4f, 0xa8,
    0x68, 0x6b, 0x81, 0xb2, 0x71, 0x64, 0xda, 0x8b, 0xf8, 0xeb, 0x0f, 0x4b, 0x70, 0x56, 0x9d, 0x35,
    0x1e, 0x24, 0x0e, 0x5e, 0x63, 0x58, 0xd1, 0xa2, 0x25, 0x22, 0x7c, 0x3b, 0x01, 0x21, 0x78, 0x87,
    0xd4, 0x00, 0x46, 0x57, 0x9f, 0xd3, 0x27, 0x52, 0x4c, 0x36, 0x02, 0xe7, 0xa0, 0xc4, 0xc8, 0x9e,
    0xea, 0xbf, 0x8a, 0xd2, 0x40, 0xc7, 0x38, 0xb5, 0xa3, 0xf7, 0xf2, 0xce, 0xf9, 0x61, 0x15, 0xa1,
    0xe0, 0xae, 0x5d, 0xa4, 0x9b, 0x34, 0x1a, 0x55, 0xad, 0x93, 0x32, 0x30, 0xf5, 0x8c, 0xb1, 0xe3,
    0x1d, 0xf6, 0xe2, 0x2e, 0x82, 0x66, 0xca, 0x60, 0xc0, 0x29, 0x23, 0xab, 0x0d, 0x53, 0x4e, 0x6f,
    0xd5, 0xdb, 0x37, 0x45, 0xde, 0xfd, 0x8e, 0x2f, 0x03, 0xff, 0x6a, 0x72, 0x6d, 0x6c, 0x5b, 0x51,
    0x8d, 0x1b, 0xaf, 0x92, 0xbb, 0xdd, 0xbc, 0x7f, 0x11, 0xd9, 0x5c, 0x41, 0x1f, 0x10, 0x5a, 0xd8,
    0x0a, 0xc1, 0x31, 0x88, 0xa5, 0xcd, 0x7b, 0xbd, 0x2d, 0x74, 0xd0, 0x12, 0xb8, 0xe5, 0xb4, 0xb0,
    0x89, 0x69, 0x97, 0x4a, 0x0c, 0x96, 0x77, 0x7e, 0x65, 0xb9, 0xf1, 0x09, 0xc5, 0x6e, 0xc6, 0x84,
    0x18, 0xf0, 0x7d, 0xec, 0x3a, 0xdc, 0x4d, 0x20, 0x79, 0xee, 0x5f, 0x3e, 0xd7, 0xcb, 0x39, 0x48,
];

const AES_RCON: [u8; 15] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36, 0x6c, 0xd8, 0xab, 0x4d,
];

const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const AES_INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const SHA1_INITIAL_HASH: [u32; 5] = [
    0x6745_2301,
    0xefcd_ab89,
    0x98ba_dcfe,
    0x1032_5476,
    0xc3d2_e1f0,
];

const SM3_INITIAL_HASH: [u32; 8] = [
    0x7380_166f,
    0x4914_b2b9,
    0x1724_42d7,
    0xda8a_0600,
    0xa96f_30bc,
    0x1631_38aa,
    0xe38d_ee4d,
    0xb0fb_0e4e,
];

const SHA256_INITIAL_HASH: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const SHA512_INITIAL_HASH: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

const SHA512_ROUND_CONSTANTS: [u64; 80] = [
    0x428a_2f98_d728_ae22,
    0x7137_4491_23ef_65cd,
    0xb5c0_fbcf_ec4d_3b2f,
    0xe9b5_dba5_8189_dbbc,
    0x3956_c25b_f348_b538,
    0x59f1_11f1_b605_d019,
    0x923f_82a4_af19_4f9b,
    0xab1c_5ed5_da6d_8118,
    0xd807_aa98_a303_0242,
    0x1283_5b01_4570_6fbe,
    0x2431_85be_4ee4_b28c,
    0x550c_7dc3_d5ff_b4e2,
    0x72be_5d74_f27b_896f,
    0x80de_b1fe_3b16_96b1,
    0x9bdc_06a7_25c7_1235,
    0xc19b_f174_cf69_2694,
    0xe49b_69c1_9ef1_4ad2,
    0xefbe_4786_384f_25e3,
    0x0fc1_9dc6_8b8c_d5b5,
    0x240c_a1cc_77ac_9c65,
    0x2de9_2c6f_592b_0275,
    0x4a74_84aa_6ea6_e483,
    0x5cb0_a9dc_bd41_fbd4,
    0x76f9_88da_8311_53b5,
    0x983e_5152_ee66_dfab,
    0xa831_c66d_2db4_3210,
    0xb003_27c8_98fb_213f,
    0xbf59_7fc7_beef_0ee4,
    0xc6e0_0bf3_3da8_8fc2,
    0xd5a7_9147_930a_a725,
    0x06ca_6351_e003_826f,
    0x1429_2967_0a0e_6e70,
    0x27b7_0a85_46d2_2ffc,
    0x2e1b_2138_5c26_c926,
    0x4d2c_6dfc_5ac4_2aed,
    0x5338_0d13_9d95_b3df,
    0x650a_7354_8baf_63de,
    0x766a_0abb_3c77_b2a8,
    0x81c2_c92e_47ed_aee6,
    0x9272_2c85_1482_353b,
    0xa2bf_e8a1_4cf1_0364,
    0xa81a_664b_bc42_3001,
    0xc24b_8b70_d0f8_9791,
    0xc76c_51a3_0654_be30,
    0xd192_e819_d6ef_5218,
    0xd699_0624_5565_a910,
    0xf40e_3585_5771_202a,
    0x106a_a070_32bb_d1b8,
    0x19a4_c116_b8d2_d0c8,
    0x1e37_6c08_5141_ab53,
    0x2748_774c_df8e_eb99,
    0x34b0_bcb5_e19b_48a8,
    0x391c_0cb3_c5c9_5a63,
    0x4ed8_aa4a_e341_8acb,
    0x5b9c_ca4f_7763_e373,
    0x682e_6ff3_d6b2_b8a3,
    0x748f_82ee_5def_b2fc,
    0x78a5_636f_4317_2f60,
    0x84c8_7814_a1f0_ab72,
    0x8cc7_0208_1a64_39ec,
    0x90be_fffa_2363_1e28,
    0xa450_6ceb_de82_bde9,
    0xbef9_a3f7_b2c6_7915,
    0xc671_78f2_e372_532b,
    0xca27_3ece_ea26_619c,
    0xd186_b8c7_21c0_c207,
    0xeada_7dd6_cde0_eb1e,
    0xf57d_4f7f_ee6e_d178,
    0x06f0_67aa_7217_6fba,
    0x0a63_7dc5_a2c8_98a6,
    0x113f_9804_bef9_0dae,
    0x1b71_0b35_131c_471b,
    0x28db_77f5_2304_7d84,
    0x32ca_ab7b_40c7_2493,
    0x3c9e_be0a_15c9_bebc,
    0x431d_67c4_9c10_0d4c,
    0x4cc5_d4be_cb3e_42b6,
    0x597f_299c_fc65_7e2a,
    0x5fcb_6fab_3ad6_faec,
    0x6c44_198c_4a47_5817,
];

fn hash_digest_hex(input: &[u8], algorithm: &str) -> Option<String> {
    hash_digest_bytes(input, algorithm).map(|bytes| hex_encode(&bytes))
}

fn hash_digest_base64(input: &[u8], algorithm: &str) -> Option<String> {
    hash_digest_bytes(input, algorithm).map(|bytes| base64_encode_with_flags(&bytes, 0))
}

fn hash_digest_bytes(input: &[u8], algorithm: &str) -> Option<Vec<u8>> {
    match normalize_digest_algorithm(algorithm).as_str() {
        "md5" => Some(md5_digest(input).to_vec()),
        "sm3" => Some(sm3_digest(input).to_vec()),
        "sha1" => Some(sha1_digest(input).to_vec()),
        "sha256" => Some(sha256_digest(input).to_vec()),
        "sha512" => Some(sha512_digest(input).to_vec()),
        _ => None,
    }
}

fn hmac_digest_hex(input: &[u8], algorithm: &str, key: &[u8]) -> Option<String> {
    hmac_digest_bytes(input, algorithm, key).map(|bytes| hex_encode(&bytes))
}

fn hmac_digest_base64(input: &[u8], algorithm: &str, key: &[u8]) -> Option<String> {
    hmac_digest_bytes(input, algorithm, key).map(|bytes| base64_encode_with_flags(&bytes, 0))
}

fn hmac_digest_bytes(input: &[u8], algorithm: &str, key: &[u8]) -> Option<Vec<u8>> {
    match normalize_hmac_algorithm(algorithm).as_str() {
        "hmacmd5" => Some(hmac_md5_digest(input, key).to_vec()),
        "hmacsha1" => Some(hmac_sha1_digest(input, key).to_vec()),
        "hmacsha256" => Some(hmac_sha256_digest(input, key).to_vec()),
        "hmacsha512" => Some(hmac_sha512_digest(input, key).to_vec()),
        _ => None,
    }
}

fn normalize_digest_algorithm(algorithm: &str) -> String {
    algorithm
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_hmac_algorithm(algorithm: &str) -> String {
    let normalized = normalize_digest_algorithm(algorithm);
    if normalized.starts_with("hmac") {
        normalized
    } else {
        format!("hmac{normalized}")
    }
}

fn create_sign_payload(params: &JsonValue) -> String {
    let object = match params {
        JsonValue::Object(object) => object,
        JsonValue::String(text) => match serde_json::from_str::<JsonValue>(text).ok() {
            Some(JsonValue::Object(object)) => {
                return create_sign_payload(&JsonValue::Object(object))
            }
            _ => return String::new(),
        },
        _ => return String::new(),
    };

    let mut keys = object.keys().collect::<Vec<_>>();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| {
            let value = object.get(key)?;
            json_sign_value_is_truthy(value)
                .then(|| format!("{}={}", key, json_sign_value_to_string(value)))
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn json_sign_value_is_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(value) => !value.is_empty(),
        JsonValue::Bool(value) => *value,
        JsonValue::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        JsonValue::Null => false,
        JsonValue::Array(_) | JsonValue::Object(_) => true,
    }
}

fn json_sign_value_to_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => value.clone(),
        JsonValue::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Null => String::new(),
        JsonValue::Array(_) | JsonValue::Object(_) => sorted_json_string(value),
    }
}

fn sorted_json_string(value: &JsonValue) -> String {
    serde_json::to_string(&sort_json_value(value)).unwrap_or_else(|_| "{}".to_string())
}

fn sort_json_value(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(items) => JsonValue::Array(items.iter().map(sort_json_value).collect()),
        JsonValue::Object(object) => {
            let mut sorted = JsonMap::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(value) = object.get(key) {
                    sorted.insert(key.clone(), sort_json_value(value));
                }
            }
            JsonValue::Object(sorted)
        }
        _ => value.clone(),
    }
}

fn hmac_md5_digest(input: &[u8], key: &[u8]) -> [u8; 16] {
    const BLOCK_SIZE: usize = 64;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..16].copy_from_slice(&md5_digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + input.len());
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 16);
    for byte in key_block {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(input);

    let inner_digest = md5_digest(&inner);
    outer.extend_from_slice(&inner_digest);
    md5_digest(&outer)
}

fn hmac_sha1_digest(input: &[u8], key: &[u8]) -> [u8; 20] {
    const BLOCK_SIZE: usize = 64;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..20].copy_from_slice(&sha1_digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + input.len());
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 20);
    for byte in key_block {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(input);

    let inner_digest = sha1_digest(&inner);
    outer.extend_from_slice(&inner_digest);
    sha1_digest(&outer)
}

fn hmac_sha256_digest(input: &[u8], key: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..32].copy_from_slice(&sha256_digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + input.len());
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 32);
    for byte in key_block {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(input);

    let inner_digest = sha256_digest(&inner);
    outer.extend_from_slice(&inner_digest);
    sha256_digest(&outer)
}

fn hmac_sha512_digest(input: &[u8], key: &[u8]) -> [u8; 64] {
    const BLOCK_SIZE: usize = 128;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..64].copy_from_slice(&sha512_digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + input.len());
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 64);
    for byte in key_block {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(input);

    let inner_digest = sha512_digest(&inner);
    outer.extend_from_slice(&inner_digest);
    sha512_digest(&outer)
}

fn sm3_digest(input: &[u8]) -> [u8; 32] {
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut message = Vec::with_capacity(((input.len() + 9).div_ceil(64)) * 64);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut state = SM3_INITIAL_HASH;
    for chunk in message.chunks_exact(64) {
        sm3_compress(chunk, &mut state);
    }

    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn sm3_compress(block: &[u8], state: &mut [u32; 8]) {
    let mut words = [0u32; 68];
    let mut words_prime = [0u32; 64];

    for (index, word) in words.iter_mut().take(16).enumerate() {
        let offset = index * 4;
        *word = u32::from_be_bytes([
            block[offset],
            block[offset + 1],
            block[offset + 2],
            block[offset + 3],
        ]);
    }

    for index in 16..68 {
        words[index] =
            sm3_p1(words[index - 16] ^ words[index - 9] ^ words[index - 3].rotate_left(15))
                ^ words[index - 13].rotate_left(7)
                ^ words[index - 6];
    }
    for index in 0..64 {
        words_prime[index] = words[index] ^ words[index + 4];
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for index in 0..64 {
        let t: u32 = if index < 16 { 0x79cc_4519 } else { 0x7a87_9d8a };
        let ss1 = a
            .rotate_left(12)
            .wrapping_add(e)
            .wrapping_add(t.rotate_left((index as u32) & 31))
            .rotate_left(7);
        let ss2 = ss1 ^ a.rotate_left(12);
        let tt1 = sm3_ff(a, b, c, index)
            .wrapping_add(d)
            .wrapping_add(ss2)
            .wrapping_add(words_prime[index]);
        let tt2 = sm3_gg(e, f, g, index)
            .wrapping_add(h)
            .wrapping_add(ss1)
            .wrapping_add(words[index]);

        d = c;
        c = b.rotate_left(9);
        b = a;
        a = tt1;
        h = g;
        g = f.rotate_left(19);
        f = e;
        e = sm3_p0(tt2);
    }

    state[0] ^= a;
    state[1] ^= b;
    state[2] ^= c;
    state[3] ^= d;
    state[4] ^= e;
    state[5] ^= f;
    state[6] ^= g;
    state[7] ^= h;
}

fn sm3_ff(x: u32, y: u32, z: u32, round: usize) -> u32 {
    if round < 16 {
        x ^ y ^ z
    } else {
        (x & y) | (x & z) | (y & z)
    }
}

fn sm3_gg(x: u32, y: u32, z: u32, round: usize) -> u32 {
    if round < 16 {
        x ^ y ^ z
    } else {
        (x & y) | ((!x) & z)
    }
}

fn sm3_p0(value: u32) -> u32 {
    value ^ value.rotate_left(9) ^ value.rotate_left(17)
}

fn sm3_p1(value: u32) -> u32 {
    value ^ value.rotate_left(15) ^ value.rotate_left(23)
}

fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut message = Vec::with_capacity(((input.len() + 9).div_ceil(64)) * 64);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = SHA1_INITIAL_HASH;
    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }

        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];

        for (index, word) in words.iter().copied().enumerate() {
            let (f, k) = match index {
                0..=19 => (((b & c) | ((!b) & d)), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => (((b & c) | (b & d) | (c & d)), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
    }

    let mut digest = [0u8; 20];
    for (index, word) in hash.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn sha256_digest(input: &[u8]) -> [u8; 32] {
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut message = Vec::with_capacity(((input.len() + 9).div_ceil(64)) * 64);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = SHA256_INITIAL_HASH;
    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];

        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_ROUND_CONSTANTS[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut digest = [0u8; 32];
    for (index, word) in hash.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn sha512_digest(input: &[u8]) -> [u8; 64] {
    let bit_len = (input.len() as u128).wrapping_mul(8);
    let mut message = Vec::with_capacity(((input.len() + 17).div_ceil(128)) * 128);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 128 != 112 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = SHA512_INITIAL_HASH;
    for chunk in message.chunks_exact(128) {
        let mut words = [0u64; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 8;
            *word = u64::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
                chunk[offset + 4],
                chunk[offset + 5],
                chunk[offset + 6],
                chunk[offset + 7],
            ]);
        }
        for index in 16..80 {
            let s0 = words[index - 15].rotate_right(1)
                ^ words[index - 15].rotate_right(8)
                ^ (words[index - 15] >> 7);
            let s1 = words[index - 2].rotate_right(19)
                ^ words[index - 2].rotate_right(61)
                ^ (words[index - 2] >> 6);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];

        for (index, word) in words.iter().copied().enumerate() {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(SHA512_ROUND_CONSTANTS[index])
                .wrapping_add(word);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut digest = [0u8; 64];
    for (index, word) in hash.iter().enumerate() {
        digest[index * 8..index * 8 + 8].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

const MD5_SHIFT_AMOUNTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_TABLE: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

fn md5_hex(input: &[u8]) -> String {
    hex_encode(&md5_digest(input))
}

fn md5_digest(input: &[u8]) -> [u8; 16] {
    let mut a0 = 0x6745_2301u32;
    let mut b0 = 0xefcd_ab89u32;
    let mut c0 = 0x98ba_dcfeu32;
    let mut d0 = 0x1032_5476u32;

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut message = Vec::with_capacity(((input.len() + 9).div_ceil(64)) * 64);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (index, word) in words.iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_le_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }

        let mut a = a0;
        let mut b = b0;
        let mut c = c0;
        let mut d = d0;

        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(MD5_TABLE[i])
                    .wrapping_add(words[g])
                    .rotate_left(MD5_SHIFT_AMOUNTS[i]),
            );
            a = d;
            d = c;
            c = b;
            b = next;
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut digest = [0u8; 16];
    digest[0..4].copy_from_slice(&a0.to_le_bytes());
    digest[4..8].copy_from_slice(&b0.to_le_bytes());
    digest[8..12].copy_from_slice(&c0.to_le_bytes());
    digest[12..16].copy_from_slice(&d0.to_le_bytes());
    digest
}

fn bytes_to_utf8_input(value: JsonValue) -> Vec<u8> {
    match value {
        JsonValue::String(value) => hex_decode(&value),
        JsonValue::Array(items) => items
            .into_iter()
            .filter_map(|item| item.as_i64().map(|value| (value & 0xff) as u8))
            .collect(),
        JsonValue::Number(value) => value
            .as_i64()
            .map(|value| vec![(value & 0xff) as u8])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn normalize_charset_name(charset: &str) -> String {
    charset
        .chars()
        .filter(|ch| *ch != '-' && *ch != '_' && !ch.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
}

fn encode_string_with_charset(input: &str, charset: &str) -> Vec<u8> {
    match normalize_charset_name(charset).as_str() {
        "utf8" => input.as_bytes().to_vec(),
        "iso88591" | "latin1" => input
            .chars()
            .map(|ch| {
                let codepoint = ch as u32;
                if codepoint <= 0xff {
                    codepoint as u8
                } else {
                    b'?'
                }
            })
            .collect(),
        "gbk" | "gb2312" | "gb18030" => encode_gbk_compat(input),
        _ => input.as_bytes().to_vec(),
    }
}

fn decode_bytes_with_charset(bytes: Vec<u8>, charset: &str) -> Option<String> {
    match normalize_charset_name(charset).as_str() {
        "utf8" => String::from_utf8(bytes).ok(),
        "iso88591" | "latin1" => Some(bytes.into_iter().map(char::from).collect()),
        "gbk" | "gb2312" | "gb18030" => Some(decode_gbk_compat(&bytes)),
        _ => String::from_utf8(bytes).ok(),
    }
}

fn encode_gbk_compat(input: &str) -> Vec<u8> {
    let mut output = Vec::new();
    for ch in input.chars() {
        if ch.is_ascii() {
            output.push(ch as u8);
        } else if let Some((lead, trail)) = char_to_gbk_pair(ch) {
            output.push(lead);
            output.push(trail);
        } else {
            output.push(b'?');
        }
    }
    output
}

fn decode_gbk_compat(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte < 0x80 {
            output.push(byte as char);
            index += 1;
            continue;
        }

        if let Some(next) = bytes.get(index + 1).copied() {
            if let Some(ch) = gbk_pair_to_char(byte, next) {
                output.push(ch);
            } else {
                output.push('\u{FFFD}');
            }
            index += 2;
        } else {
            output.push('\u{FFFD}');
            index += 1;
        }
    }

    output
}

fn char_to_gbk_pair(ch: char) -> Option<(u8, u8)> {
    match ch {
        '小' => Some((0xd0, 0xa1)),
        '说' => Some((0xcb, 0xb5)),
        '鬼' => Some((0xb9, 0xed)),
        '吹' => Some((0xb4, 0xb5)),
        '灯' => Some((0xb5, 0xc6)),
        '搜' => Some((0xcb, 0xd1)),
        '索' => Some((0xcb, 0xf7)),
        '提' => Some((0xcc, 0xe1)),
        '交' => Some((0xbd, 0xbb)),
        _ => None,
    }
}

fn gbk_pair_to_char(lead: u8, trail: u8) -> Option<char> {
    match (lead, trail) {
        (0xd0, 0xa1) => Some('小'),
        (0xcb, 0xb5) => Some('说'),
        (0xb9, 0xed) => Some('鬼'),
        (0xb4, 0xb5) => Some('吹'),
        (0xb5, 0xc6) => Some('灯'),
        (0xcb, 0xd1) => Some('搜'),
        (0xcb, 0xf7) => Some('索'),
        (0xcc, 0xe1) => Some('提'),
        (0xbd, 0xbb) => Some('交'),
        _ => None,
    }
}

const UPPER_HEX_TABLE: &[u8; 16] = b"0123456789ABCDEF";

fn percent_encode_uri(input: &str, charset: Option<&str>) -> String {
    percent_encode_with_charset(input, charset, is_encode_uri_allowed)
}

fn percent_encode_uri_component(input: &str, charset: Option<&str>) -> String {
    percent_encode_with_charset(input, charset, is_encode_uri_component_allowed)
}

fn percent_encode_with_charset(
    input: &str,
    charset: Option<&str>,
    is_allowed: fn(u8) -> bool,
) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii() && is_allowed(ch as u8) {
            output.push(ch);
        } else {
            let text = ch.to_string();
            let bytes = charset
                .map(|charset| encode_string_with_charset(&text, charset))
                .unwrap_or_else(|| text.into_bytes());
            for byte in bytes {
                output.push('%');
                output.push(UPPER_HEX_TABLE[(byte >> 4) as usize] as char);
                output.push(UPPER_HEX_TABLE[(byte & 0x0f) as usize] as char);
            }
        }
    }
    output
}

fn time_format_local(ms: f64, pattern: &str) -> String {
    time_format_utc(ms, pattern, DEFAULT_LOCAL_TIME_OFFSET_MS)
}

fn time_format_utc(ms: f64, pattern: &str, offset_ms: i64) -> String {
    if !ms.is_finite() {
        return String::new();
    }

    let total_ms = (ms.trunc() as i64).saturating_add(offset_ms);
    let seconds = total_ms.div_euclid(1000);
    let mut second_of_day = seconds.rem_euclid(86_400);
    let days = seconds.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    second_of_day %= 3_600;
    let minute = second_of_day / 60;
    let second = second_of_day % 60;

    format_time_pattern(pattern, year, month, day, hour, minute, second)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    (year as i32, m as u32, d as u32)
}

fn format_time_pattern(
    pattern: &str,
    year: i32,
    month: u32,
    day: u32,
    hour: i64,
    minute: i64,
    second: i64,
) -> String {
    let mut output = String::with_capacity(pattern.len());
    let mut index = 0;

    while index < pattern.len() {
        let remaining = &pattern[index..];
        if remaining.starts_with("yyyy") {
            output.push_str(&format!("{year:04}"));
            index += 4;
        } else if remaining.starts_with("MM") {
            output.push_str(&format!("{month:02}"));
            index += 2;
        } else if remaining.starts_with("dd") {
            output.push_str(&format!("{day:02}"));
            index += 2;
        } else if remaining.starts_with("HH") {
            output.push_str(&format!("{hour:02}"));
            index += 2;
        } else if remaining.starts_with("mm") {
            output.push_str(&format!("{minute:02}"));
            index += 2;
        } else if remaining.starts_with("ss") {
            output.push_str(&format!("{second:02}"));
            index += 2;
        } else if let Some(ch) = remaining.chars().next() {
            output.push(ch);
            index += ch.len_utf8();
        } else {
            break;
        }
    }

    output
}

fn to_num_chapter(input: &str) -> String {
    let Some(prefix_start) = input.find('第') else {
        return input.to_string();
    };
    let number_start = prefix_start + '第'.len_utf8();
    let Some(relative_suffix_start) = input[number_start..].find('章') else {
        return input.to_string();
    };
    let suffix_start = number_start + relative_suffix_start;
    if suffix_start == number_start {
        return input.to_string();
    }

    let number = &input[number_start..suffix_start];
    match chapter_number_to_int(number) {
        Some(number) => format!("第{number}章"),
        None => "第-1章".to_string(),
    }
}

fn chapter_number_to_int(input: &str) -> Option<i64> {
    let normalized = full_to_half(input)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if normalized.is_empty() {
        return None;
    }

    normalized
        .parse::<i64>()
        .ok()
        .or_else(|| chinese_num_to_int(&normalized))
}

fn full_to_half(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            '\u{3000}' => ' ',
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(ch as u32 - 0xfee0).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

fn chinese_num_to_int(input: &str) -> Option<i64> {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }

    let mut result = 0i64;
    let mut tmp = 0i64;
    let mut billion = 0i64;

    for (index, ch) in chars.iter().copied().enumerate() {
        let tmp_num = chinese_number_value(ch)?;
        match tmp_num {
            100_000_000 => {
                result += tmp;
                result *= tmp_num;
                billion = billion * 100_000_000 + result;
                result = 0;
                tmp = 0;
            }
            10_000 => {
                result += tmp;
                result *= tmp_num;
                tmp = 0;
            }
            10..=9999 => {
                if tmp == 0 {
                    tmp = 1;
                }
                result += tmp_num * tmp;
                tmp = 0;
            }
            _ => {
                tmp = if index >= 2 && index + 1 == chars.len() {
                    let previous = chinese_number_value(chars[index - 1])?;
                    if previous > 10 {
                        tmp_num * previous / 10
                    } else {
                        tmp * 10 + tmp_num
                    }
                } else {
                    tmp * 10 + tmp_num
                };
            }
        }
    }

    Some(result + tmp + billion)
}

fn chinese_number_value(ch: char) -> Option<i64> {
    match ch {
        '零' | '〇' => Some(0),
        '一' | '壹' => Some(1),
        '二' | '两' | '贰' => Some(2),
        '三' | '叁' => Some(3),
        '四' | '肆' => Some(4),
        '五' | '伍' => Some(5),
        '六' | '陆' => Some(6),
        '七' | '柒' => Some(7),
        '八' | '捌' => Some(8),
        '九' | '玖' => Some(9),
        '十' | '拾' => Some(10),
        '百' | '佰' => Some(100),
        '千' | '仟' => Some(1000),
        '万' => Some(10_000),
        '亿' => Some(100_000_000),
        _ => None,
    }
}

fn t2s(input: &str) -> String {
    input.chars().map(t2s_char).collect()
}

fn s2t(input: &str) -> String {
    input.chars().map(s2t_char).collect()
}

fn t2s_char(ch: char) -> char {
    match ch {
        '門' => '门',
        '會' => '会',
        '說' => '说',
        '書' => '书',
        '圖' => '图',
        '館' => '馆',
        '發' => '发',
        '體' => '体',
        '國' => '国',
        '語' => '语',
        '話' => '话',
        '學' => '学',
        '時' => '时',
        '開' => '开',
        '關' => '关',
        '臺' | '颱' | '檯' => '台',
        '個' => '个',
        '們' => '们',
        '對' => '对',
        '過' => '过',
        '點' => '点',
        _ => ch,
    }
}

fn s2t_char(ch: char) -> char {
    match ch {
        '门' => '門',
        '会' => '會',
        '说' => '說',
        '书' => '書',
        '图' => '圖',
        '馆' => '館',
        '发' => '發',
        '体' => '體',
        '国' => '國',
        '语' => '語',
        '话' => '話',
        '学' => '學',
        '时' => '時',
        '开' => '開',
        '关' => '關',
        '台' => '臺',
        '个' => '個',
        '们' => '們',
        '对' => '對',
        '过' => '過',
        '点' => '點',
        _ => ch,
    }
}

fn html_format(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < input.len() {
        let remaining = &input[index..];
        if remaining.starts_with('<') {
            if let Some(end) = remaining.find('>') {
                if html_tag_is_line_break(&remaining[..=end]) {
                    output.push('\n');
                }
                index += end + 1;
                continue;
            }
        }

        if remaining.starts_with('&') {
            if let Some((entity, len)) = decode_html_entity(remaining) {
                output.push(entity);
                index += len;
                continue;
            }
        }

        if let Some(ch) = remaining.chars().next() {
            output.push(ch);
            index += ch.len_utf8();
        } else {
            break;
        }
    }

    normalize_html_text(&output)
}

fn html_tag_is_line_break(tag: &str) -> bool {
    let name = tag
        .trim_matches(|ch| matches!(ch, '<' | '>' | '/' | '!' | '?'))
        .trim_start()
        .split(|ch: char| ch.is_ascii_whitespace() || ch == '/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    matches!(
        name.as_str(),
        "br" | "p"
            | "div"
            | "li"
            | "tr"
            | "table"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

fn decode_html_entity(input: &str) -> Option<(char, usize)> {
    let end = input.find(';')?;
    if end > 16 {
        return None;
    }
    let entity = &input[1..end];
    let ch = match entity {
        "nbsp" => ' ',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            let value = u32::from_str_radix(&entity[2..], 16).ok()?;
            char::from_u32(value)?
        }
        _ if entity.starts_with('#') => {
            let value = entity[1..].parse::<u32>().ok()?;
            char::from_u32(value)?
        }
        _ => return None,
    };

    Some((ch, end + 1))
}

fn normalize_html_text(input: &str) -> String {
    input
        .replace('\r', "\n")
        .split('\n')
        .filter_map(|line| {
            let collapsed = collapse_horizontal_whitespace(line);
            let trimmed = collapsed.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_horizontal_whitespace(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;

    for ch in input.chars() {
        if ch.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            output.push(ch);
            pending_space = false;
        }
    }

    output
}

#[derive(Clone, Debug)]
struct JsUrlParts {
    href: String,
    host: String,
    origin: String,
    pathname: String,
    query: Option<String>,
}

fn js_url_to_object<'js>(
    ctx: &Ctx<'js>,
    parts: JsUrlParts,
) -> Result<rquickjs::Object<'js>, QuickJsError> {
    let object = rquickjs::Object::new(ctx.clone())?;
    object.set("href", parts.href.clone())?;
    object.set("host", parts.host)?;
    object.set("origin", parts.origin)?;
    object.set("pathname", parts.pathname)?;

    if let Some(query) = parts.query {
        let params = rquickjs::Object::new(ctx.clone())?;
        for (key, value) in parse_query_params(&query) {
            params.set(key, value)?;
        }
        object.set("searchParams", params)?;
    } else {
        object.set("searchParams", rquickjs::Value::new_null(ctx.clone()))?;
    }

    let href = parts.href;
    object.set(
        "toString",
        rquickjs::Function::new(ctx.clone(), move || href.clone())?,
    )?;
    Ok(object)
}

fn resolve_js_url(url: &str, base_url: Option<&str>) -> Result<JsUrlParts, String> {
    if is_absolute_url(url) {
        return parse_absolute_url(url);
    }

    let Some(base_url) = base_url.filter(|value| !value.is_empty()) else {
        return Err("relative URL requires a base URL".to_string());
    };
    let base = parse_absolute_url(base_url)?;

    if url.starts_with("//") {
        let scheme = base
            .origin
            .split_once("://")
            .map(|(scheme, _)| scheme)
            .ok_or_else(|| "base URL is missing a scheme".to_string())?;
        return parse_absolute_url(&format!("{scheme}:{url}"));
    }

    let (path, suffix) = split_url_path_suffix(url);
    let resolved_path = if path.is_empty() && suffix.starts_with('?') {
        base.pathname.clone()
    } else if path.is_empty() && suffix.starts_with('#') {
        match base.query.as_deref() {
            Some(query) if !query.is_empty() => format!("{}?{}", base.pathname, query),
            _ => base.pathname.clone(),
        }
    } else if path.starts_with('/') {
        normalize_url_path(path)
    } else {
        let base_dir = base
            .pathname
            .rsplit_once('/')
            .map(|(prefix, _)| {
                if prefix.is_empty() {
                    "/".to_string()
                } else {
                    format!("{prefix}/")
                }
            })
            .unwrap_or_else(|| "/".to_string());
        normalize_url_path(&format!("{base_dir}{path}"))
    };

    parse_absolute_url(&format!("{}{}{}", base.origin, resolved_path, suffix))
}

fn parse_absolute_url(input: &str) -> Result<JsUrlParts, String> {
    let Some((scheme, rest)) = input.split_once("://") else {
        return Err("URL is missing a scheme".to_string());
    };
    if scheme.is_empty() {
        return Err("URL scheme is empty".to_string());
    }

    let authority_end = rest
        .find(|ch| matches!(ch, '/' | '?' | '#'))
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return Err("URL host is empty".to_string());
    }

    let remainder = &rest[authority_end..];
    let path_and_suffix = if remainder.is_empty() {
        "/".to_string()
    } else if remainder.starts_with('/') {
        remainder.to_string()
    } else {
        format!("/{remainder}")
    };
    let (path, suffix) = split_url_path_suffix(&path_and_suffix);
    let pathname = if path.is_empty() { "/" } else { path };
    let origin = format!("{scheme}://{authority}");
    let href = format!("{origin}{pathname}{suffix}");
    let query = suffix
        .strip_prefix('?')
        .map(|value| value.split('#').next().unwrap_or_default().to_string());

    Ok(JsUrlParts {
        href,
        host: authority
            .split_once(':')
            .map(|(host, _)| host)
            .unwrap_or(authority)
            .to_string(),
        origin,
        pathname: pathname.to_string(),
        query,
    })
}

fn is_absolute_url(input: &str) -> bool {
    input
        .split_once("://")
        .map(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        })
        .unwrap_or(false)
}

fn split_url_path_suffix(input: &str) -> (&str, &str) {
    let suffix_start = input
        .find(|ch| matches!(ch, '?' | '#'))
        .unwrap_or(input.len());
    (&input[..suffix_start], &input[suffix_start..])
}

fn normalize_url_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let trailing_slash = path.ends_with('/');
    let mut stack = Vec::<&str>::new();

    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            value => stack.push(value),
        }
    }

    let mut normalized = String::new();
    if absolute {
        normalized.push('/');
    }
    normalized.push_str(&stack.join("/"));
    if trailing_slash && !normalized.ends_with('/') {
        normalized.push('/');
    }
    if normalized.is_empty() {
        normalized.push('/');
    }
    normalized
}

fn parse_query_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((key.to_string(), percent_decode_utf8(value)))
        })
        .collect()
}

fn percent_decode_utf8(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut iter = input.as_bytes().iter().copied().peekable();

    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let high = iter.next();
            let low = iter.next();
            match (high, low) {
                (Some(high), Some(low)) => {
                    if let Some(value) = percent_hex_pair_value(high, low) {
                        bytes.push(value);
                    } else {
                        bytes.extend_from_slice(&[byte, high, low]);
                    }
                }
                (Some(high), None) => {
                    bytes.extend_from_slice(&[byte, high]);
                }
                (None, Some(low)) => {
                    bytes.extend_from_slice(&[byte, low]);
                }
                (None, None) => bytes.push(byte),
            }
        } else if byte == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(byte);
        }
    }

    String::from_utf8(bytes).unwrap_or_default()
}

fn percent_hex_pair_value(high: u8, low: u8) -> Option<u8> {
    Some((hex_value(high)? << 4) | hex_value(low)?)
}

fn js_value_type_name(value: &QuickJsValue<'_>) -> String {
    if value.is_null() {
        "null".to_string()
    } else if value.is_undefined() {
        "undefined".to_string()
    } else if value.as_array().is_some() {
        "array".to_string()
    } else if value.is_function() {
        "function".to_string()
    } else if value.as_bool().is_some() {
        "boolean".to_string()
    } else if value.as_int().is_some() || value.as_float().is_some() {
        "number".to_string()
    } else if value.is_string() {
        "string".to_string()
    } else if value.as_object().is_some() {
        "object".to_string()
    } else {
        value.type_name().to_string()
    }
}

fn is_encode_uri_allowed(byte: u8) -> bool {
    matches!(
        byte,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b':'
            | b'/'
            | b'?'
            | b'#'
            | b'['
            | b']'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b'%'
    )
}

fn is_encode_uri_component_allowed(byte: u8) -> bool {
    matches!(
        byte,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')'
    )
}

fn console_records(console: &ConsoleBuffer) -> Vec<ConsoleRecord> {
    console
        .lock()
        .map(|records| records.clone())
        .unwrap_or_default()
}

fn resolve_maybe_promise<'js>(
    value: QuickJsValue<'js>,
    ctx: &Ctx<'js>,
    interrupt: &InterruptState,
) -> JsResult<QuickJsValue<'js>> {
    let Some(promise) = value.as_promise().cloned() else {
        return Ok(value);
    };

    for _ in 0..MAX_PROMISE_JOBS {
        if interrupt.should_interrupt() {
            return Err(interrupted_error(interrupt));
        }

        if let Some(result) = promise.result::<QuickJsValue<'js>>() {
            return result
                .catch(promise.ctx())
                .map_err(|error| map_caught_error(error, interrupt));
        }

        if !ctx.execute_pending_job() {
            if promise.state() == rquickjs::promise::PromiseState::Pending {
                return Err(JsError::new(
                    JsErrorKind::Unsupported,
                    "promise is still pending and no QuickJS jobs are available",
                ));
            }
        }
    }

    Err(JsError::new(
        JsErrorKind::Unsupported,
        format!("promise job budget exceeded after {MAX_PROMISE_JOBS} jobs"),
    ))
}

fn drain_promise_jobs(ctx: &Ctx<'_>, interrupt: &InterruptState) -> JsResult<()> {
    for _ in 0..MAX_PROMISE_JOBS {
        if interrupt.should_interrupt() {
            return Err(interrupted_error(interrupt));
        }

        if !ctx.execute_pending_job() {
            return Ok(());
        }
    }

    Err(JsError::new(
        JsErrorKind::Unsupported,
        format!("promise job drain budget exceeded after {MAX_PROMISE_JOBS} jobs"),
    ))
}

fn interrupted_error(interrupt: &InterruptState) -> JsError {
    match interrupt.current_reason() {
        InterruptReason::Timeout => JsError::new(JsErrorKind::Timeout, "execution timeout elapsed"),
        InterruptReason::Cancelled => JsError::new(JsErrorKind::Cancelled, "execution cancelled"),
        InterruptReason::None => JsError::new(JsErrorKind::Cancelled, "execution interrupted"),
    }
}

fn quickjs_value_to_json(value: &QuickJsValue<'_>, depth: usize) -> JsResult<JsonValue> {
    if depth > MAX_JSON_DEPTH {
        return Err(JsError::new(
            JsErrorKind::NonJsonValue,
            "maximum JSON conversion depth exceeded",
        ));
    }

    if value.is_null() || value.is_undefined() {
        return Ok(JsonValue::Null);
    }

    if let Some(value) = value.as_bool() {
        return Ok(JsonValue::Bool(value));
    }

    if let Some(value) = value.as_int() {
        return Ok(JsonValue::Number(JsonNumber::from(value)));
    }

    if let Some(value) = value.as_float() {
        let number = JsonNumber::from_f64(value).ok_or_else(|| {
            JsError::new(
                JsErrorKind::NonJsonValue,
                "non-finite JavaScript number is not JSON-compatible",
            )
        })?;
        return Ok(JsonValue::Number(number));
    }

    if value.is_string() {
        let string = value
            .get::<String>()
            .map_err(|error| map_non_json_error(value.type_name(), error))?;
        return Ok(JsonValue::String(string));
    }

    if let Some(array) = value.as_array() {
        let mut items = Vec::with_capacity(array.len());
        for item in array.iter::<QuickJsValue<'_>>() {
            let item = item.map_err(|error| map_non_json_error("array item", error))?;
            items.push(quickjs_value_to_json(&item, depth + 1)?);
        }
        return Ok(JsonValue::Array(items));
    }

    if let Some(object) = value.as_object() {
        if value.is_function() || value.is_promise() || value.is_error() {
            return Err(JsError::new(
                JsErrorKind::NonJsonValue,
                format!("{} is not JSON-compatible", value.type_name()),
            ));
        }

        let mut map = JsonMap::new();
        for key in object.keys::<String>() {
            let key = key.map_err(|error| map_non_json_error("object key", error))?;
            let property = object
                .get::<_, QuickJsValue<'_>>(key.as_str())
                .map_err(|error| map_non_json_error("object property", error))?;
            map.insert(key, quickjs_value_to_json(&property, depth + 1)?);
        }
        return Ok(JsonValue::Object(map));
    }

    Err(JsError::new(
        JsErrorKind::NonJsonValue,
        format!("{} is not JSON-compatible", value.type_name()),
    ))
}

fn quickjs_value_to_string(value: &QuickJsValue<'_>) -> JsResult<String> {
    match quickjs_value_to_json(value, 0)? {
        JsonValue::String(value) => Ok(value),
        JsonValue::Null => Ok(String::new()),
        value => Ok(value.to_string()),
    }
}

fn optional_quickjs_string(value: Option<&QuickJsValue<'_>>, default: &str) -> String {
    value
        .and_then(|value| quickjs_value_to_string(value).ok())
        .unwrap_or_else(|| default.to_string())
}

fn json_to_quickjs<'js>(
    ctx: &Ctx<'js>,
    value: &JsonValue,
) -> Result<QuickJsValue<'js>, QuickJsError> {
    let encoded =
        serde_json::to_string(value).expect("serde_json::Value serialization cannot fail");
    ctx.json_parse(encoded)
}

fn map_caught_error(error: rquickjs::CaughtError<'_>, interrupt: &InterruptState) -> JsError {
    match interrupt.current_reason() {
        InterruptReason::Timeout => {
            return JsError::new(JsErrorKind::Timeout, "execution timeout elapsed");
        }
        InterruptReason::Cancelled => {
            return JsError::new(JsErrorKind::Cancelled, "execution cancelled");
        }
        InterruptReason::None => {}
    }

    match error {
        rquickjs::CaughtError::Exception(exception) => {
            let name = exception
                .as_object()
                .get::<_, Option<String>>("name")
                .ok()
                .flatten();
            let message = exception
                .message()
                .or_else(|| name.clone())
                .unwrap_or_else(|| "JavaScript exception".to_string());
            let kind = if message.starts_with("host callback ")
                || message.starts_with("unknown host method")
            {
                JsErrorKind::HostCallback
            } else if name.as_deref() == Some("SyntaxError") {
                JsErrorKind::Syntax
            } else {
                JsErrorKind::Exception
            };
            JsError::new(kind, message).with_stack(exception.stack())
        }
        rquickjs::CaughtError::Value(value) => {
            let json = quickjs_value_to_json(&value, 0).ok();
            JsError::new(
                JsErrorKind::Exception,
                format!("JavaScript threw {}", value.type_name()),
            )
            .with_value(json)
        }
        rquickjs::CaughtError::Error(error) => map_quickjs_engine_error(error),
    }
}

fn map_quickjs_engine_error(error: QuickJsError) -> JsError {
    let kind = match error {
        QuickJsError::Allocation => JsErrorKind::MemoryLimit,
        _ => JsErrorKind::Internal,
    };
    JsError::new(kind, error.to_string())
}

fn map_non_json_error(source: &str, error: QuickJsError) -> JsError {
    JsError::new(
        JsErrorKind::NonJsonValue,
        format!("failed to convert {source} into JSON: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[test]
    fn sm4_ecb_no_padding_matches_official_vector() {
        let key = hex_decode("0123456789abcdeffedcba9876543210");
        let plain = hex_decode("0123456789abcdeffedcba9876543210");
        let transformation = SymmetricTransformation {
            algorithm: SymmetricAlgorithm::Sm4,
            mode: SymmetricMode::Ecb,
            padding: SymmetricPadding::NoPadding,
        };

        let encrypted = sm4_encrypt_bytes(&plain, &key, &[], transformation).unwrap();
        assert_eq!(hex_encode(&encrypted), "681edf34d206965e86b3e94f536e4246");

        let decrypted = sm4_decrypt_bytes(&encrypted, &key, &[], transformation).unwrap();
        assert_eq!(hex_encode(&decrypted), "0123456789abcdeffedcba9876543210");
    }

    #[test]
    fn evaluates_js_expression_and_function_result() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                const twice = (value) => value * 2;
                ({ answer: twice(21), ok: true })
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!({"answer": 42, "ok": true}));
    }

    #[test]
    fn converts_nested_json_values() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    title: "Reader",
                    values: [1, 2.5, null, false],
                    nested: { source: "fixture" }
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "title": "Reader",
                "values": [1, 2.5, null, false],
                "nested": { "source": "fixture" }
            })
        );
    }

    #[test]
    fn maps_thrown_error_to_structured_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate(r#"throw new Error("boom")"#).unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
        assert_eq!(error.message, "boom");
    }

    #[test]
    fn maps_syntax_error_to_structured_error() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("function (").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Syntax);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn interrupts_long_running_script_on_timeout() {
        let sandbox = QuickJsSandbox::new(JsRuntimeConfig {
            timeout: Some(Duration::from_millis(10)),
            ..JsRuntimeConfig::default()
        });

        let error = sandbox.evaluate("while (true) {}").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Timeout);
    }

    #[test]
    fn rejects_already_cancelled_execution() {
        let sandbox = QuickJsSandbox::default();
        let token = CancellationToken::new();
        token.cancel();

        let error = sandbox
            .evaluate_with_options(
                "1 + 1",
                JsExecutionOptions {
                    cancellation_token: Some(token),
                    ..JsExecutionOptions::default()
                },
            )
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Cancelled);
    }

    #[test]
    fn reports_sandbox_capabilities() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", |_| Ok(json!(null)));
        registry.register("java.post", |_| Ok(json!(null)));
        let sandbox = QuickJsSandbox::new(JsRuntimeConfig {
            timeout: Some(Duration::from_secs(1)),
            memory_limit_bytes: Some(1024 * 1024),
            max_stack_size_bytes: Some(256 * 1024),
            ..JsRuntimeConfig::default()
        });
        let sandbox = QuickJsSandbox::with_host_callbacks(sandbox.config().clone(), registry);
        let capabilities = sandbox.capabilities();

        assert_eq!(capabilities.engine, "quickjs/rquickjs");
        assert_eq!(capabilities.timeout, CapabilityStatus::Enforced);
        assert_eq!(capabilities.memory_limit, CapabilityStatus::Enforced);
        assert_eq!(capabilities.stack_limit, CapabilityStatus::Enforced);
        assert_eq!(capabilities.console_capture, CapabilityStatus::Enforced);
        assert_eq!(capabilities.promise_jobs, CapabilityStatus::Enforced);
        assert_eq!(
            capabilities.host_callbacks,
            vec!["java.get".to_string(), "java.post".to_string()]
        );
    }

    #[test]
    fn captures_console_records_in_order() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                console.log("first", { n: 1 });
                Promise.resolve().then(() => console.warn("second", [2]));
                console.error("third", false);
                "done";
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!("done"));
        assert_eq!(
            result.console,
            vec![
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![json!("first"), json!({ "n": 1 })],
                },
                ConsoleRecord {
                    level: ConsoleLevel::Error,
                    args: vec![json!("third"), json!(false)],
                },
                ConsoleRecord {
                    level: ConsoleLevel::Warn,
                    args: vec![json!("second"), json!([2])],
                },
            ]
        );
    }

    #[test]
    fn legado_log_helpers_return_message_and_record_console_log() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalResult: log("global debug"),
                    javaResult: java.log("java debug")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalResult": "global debug",
                "javaResult": "java debug"
            })
        );
        assert_eq!(
            result.console,
            vec![
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![json!("global debug")],
                },
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![json!("java debug")],
                },
            ]
        );
    }

    #[test]
    fn toast_helpers_record_core_intent_without_host_ui_side_effects() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    shortResult: toast("short message"),
                    longResult: java.longToast("long message")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "shortResult": "",
                "longResult": ""
            })
        );
        assert_eq!(
            result.console,
            vec![
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![
                        json!("java.toast() requested"),
                        json!("short message"),
                        json!("short")
                    ],
                },
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![
                        json!("java.longToast() requested"),
                        json!("long message"),
                        json!("long")
                    ],
                },
            ]
        );
    }

    #[test]
    fn refresh_toc_url_records_core_intent_without_host_refresh_side_effect() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalResult: refreshTocUrl(),
                    javaResult: java.refreshTocUrl()
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalResult": "",
                "javaResult": ""
            })
        );
        assert_eq!(
            result.console,
            vec![
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![json!("java.refreshTocUrl() requested")],
                },
                ConsoleRecord {
                    level: ConsoleLevel::Log,
                    args: vec![json!("java.refreshTocUrl() requested")],
                },
            ]
        );
    }

    #[test]
    fn resolves_promise_then_result() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate("Promise.resolve(40).then((value) => value + 2)")
            .unwrap();

        assert_eq!(result.value, json!(42));
    }

    #[test]
    fn resolves_async_function_result() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                (async () => {
                    const value = await Promise.resolve(21);
                    return value * 2;
                })()
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!(42));
    }

    #[test]
    fn returns_unsupported_for_pending_promise_without_jobs() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("new Promise(() => {})").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Unsupported);
        assert!(error.message.contains("still pending"));
    }

    #[test]
    fn routes_java_get_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!({ "status": "stubbed" }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.get("https://example.test", { headers: { Accept: "text/plain" } })
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!({ "status": "stubbed" }));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::HttpGet { url, headers } => {
                assert_eq!(url, "https://example.test");
                assert_eq!(
                    *headers,
                    Some(json!({ "headers": { "Accept": "text/plain" } }))
                );
            }
            other => panic!("expected HttpGet, got {other:?}"),
        }
    }

    #[test]
    fn routes_java_connect_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.connect", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!({ "status": "connected" }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.connect("https://example.test", { headers: { Accept: "text/plain" } })
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!({ "status": "connected" }));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::HttpConnect { url, header } => {
                assert_eq!(url, "https://example.test");
                // The second JS arg `{ headers: {...} }` is a non-string object;
                // legado's connect takes `header: String?` (JSON-encoded map).
                // Non-string args map to None (legado would coerce to string).
                assert_eq!(*header, None);
            }
            other => panic!("expected HttpConnect, got {other:?}"),
        }
    }

    #[test]
    fn routes_java_ajax_all_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajaxAll", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            // legado returns one response per url; we stub a per-url status.
            Ok(json!([
                { "url": "https://one.example.test", "status": "ok" },
                { "url": "https://two.example.test", "status": "ok" }
            ]))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.ajaxAll(
                    [
                        { url: "https://one.example.test" },
                        { url: "https://two.example.test" }
                    ],
                    { headers: { Accept: "application/json" } }
                )
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!([
                { "url": "https://one.example.test", "status": "ok" },
                { "url": "https://two.example.test", "status": "ok" }
            ])
        );

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::AjaxAll { urls } => {
                // legado's ajaxAll(Array<String>) takes url strings. The test passes
                // objects {url:...}; build_host_descriptor extracts the string from
                // each array element via as_str — objects are non-string, so they
                // are filtered out. This is a pre-existing semantic gap (legado
                // expects string urls), recorded as a follow-up.
                assert!(urls.is_empty() || urls.iter().all(|u| u.is_empty()));
            }
            other => panic!("expected AjaxAll, got {other:?}"),
        }
    }

    #[test]
    fn routes_java_ajax_through_host_callback_registry_like_legado() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajax", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!({ "body": "<html>stub</html>" }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.ajax("https://example.test/chapter", { method: "GET" })
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!({ "body": "<html>stub</html>" }));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::Ajax { url } => {
                assert_eq!(url, "https://example.test/chapter");
            }
            other => panic!("expected Ajax, got {other:?}"),
        }
    }

    #[test]
    fn routes_global_ajax_binding_through_host_callback_registry_like_old_core() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajax", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!("mock ajax body"))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(r#"ajax("https://example.test/api")"#)
            .unwrap();

        assert_eq!(result.value, json!("mock ajax body"));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::Ajax { url } => {
                assert_eq!(url, "https://example.test/api");
            }
            other => panic!("expected Ajax, got {other:?}"),
        }
    }

    #[test]
    fn routes_global_ajax_all_binding_through_host_callback_registry_like_old_core() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajaxAll", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!(["one body", "two body"]))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                ajaxAll([
                    "https://one.example.test",
                    "https://two.example.test"
                ])
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!(["one body", "two body"]));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::AjaxAll { urls } => {
                assert_eq!(
                    urls,
                    &["https://one.example.test", "https://two.example.test"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                );
            }
            other => panic!("expected AjaxAll, got {other:?}"),
        }
    }

    #[test]
    fn java_base64_helpers_round_trip_utf8_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    encoded: java.base64Encode("reader"),
                    decoded: java.base64Decode("cmVhZGVy"),
                    unicode: java.base64Decode(java.base64Encode("小说"))
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "encoded": "cmVhZGVy",
                "decoded": "reader",
                "unicode": "小说"
            })
        );
    }

    #[test]
    fn java_base64_encode_honors_url_safe_no_padding_flags_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    flagged: java.base64Encode("hello??", 9),
                    defaultEncoded: java.base64Encode("hello??")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "flagged": "aGVsbG8_Pw",
                "defaultEncoded": "aGVsbG8/Pw=="
            })
        );
    }

    #[test]
    fn global_base64_helpers_honor_url_safe_flags_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    encoded: base64Encode("hello??", 9),
                    decoded: base64Decode("aGVsbG8_Pw", 8),
                    javaDecoded: java.base64Decode("aGVsbG8_Pw", 8)
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "encoded": "aGVsbG8_Pw",
                "decoded": "hello??",
                "javaDecoded": "hello??"
            })
        );
    }

    #[test]
    fn base64_decode_charset_overloads_match_legado_and_old_core_fixtures() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    latin1: base64Decode("Y2Fm6Q==", "ISO-8859-1"),
                    javaLatin1: java.base64Decode("Y2Fm6Q==", "ISO-8859-1"),
                    gbk: base64Decode("0KHLtQ==", "GBK"),
                    javaGbk: java.base64Decode("0KHLtQ==", "GBK")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "latin1": "café",
                "javaLatin1": "café",
                "gbk": "小说",
                "javaGbk": "小说"
            })
        );
    }

    #[test]
    fn java_base64_decode_to_byte_array_returns_js_bytes_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                (function() {
                    var bytes = java.base64DecodeToByteArray("YWJj");
                    return {
                        isArray: Array.isArray(bytes),
                        bytes: bytes,
                        hex: bytes.map(function(byte) {
                            var value = (byte & 255).toString(16);
                            return value.length < 2 ? "0" + value : value;
                        }).join("")
                    };
                })()
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "isArray": true,
                "bytes": [97, 98, 99],
                "hex": "616263"
            })
        );
    }

    #[test]
    fn java_base64_decode_to_byte_array_honors_url_safe_flags_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                java.base64DecodeToByteArray("-_8", 8).map(function(byte) {
                    var value = (byte & 255).toString(16);
                    return value.length < 2 ? "0" + value : value;
                }).join("")
                "#,
            )
            .unwrap();

        assert_eq!(result.value, json!("fbff"));
    }

    #[test]
    fn java_hex_helpers_round_trip_utf8_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    oldCoreEncoded: java.hexEncode("reader"),
                    oldCoreDecoded: java.hexDecode("726561646572"),
                    legadoEncoded: java.hexEncodeToString("小说"),
                    legadoDecoded: java.hexDecodeToString(java.hexEncodeToString("小说"))
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "oldCoreEncoded": "726561646572",
                "oldCoreDecoded": "reader",
                "legadoEncoded": "e5b08fe8afb4",
                "legadoDecoded": "小说"
            })
        );
    }

    #[test]
    fn java_hex_decode_to_byte_array_returns_js_bytes_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                (function() {
                    var bytes = java.hexDecodeToByteArray("616263");
                    return {
                        isArray: Array.isArray(bytes),
                        bytes: bytes,
                        text: String.fromCharCode.apply(null, bytes),
                        hex: bytes.map(function(byte) {
                            var value = (byte & 255).toString(16);
                            return value.length < 2 ? "0" + value : value;
                        }).join("")
                    };
                })()
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "isArray": true,
                "bytes": [97, 98, 99],
                "text": "abc",
                "hex": "616263"
            })
        );
    }

    #[test]
    fn global_hex_aliases_round_trip_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                (function() {
                    var bytes = hexDecodeToByteArray("616263");
                    return {
                        aliasText: hexDecodeToString(hexEncodeToString("abc")),
                        encoded: hexEncode("reader"),
                        decoded: hexDecode("726561646572"),
                        byteHex: bytes.map(function(byte) {
                            var value = (byte & 255).toString(16);
                            return value.length < 2 ? "0" + value : value;
                        }).join("")
                    };
                })()
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "aliasText": "abc",
                "encoded": "726561646572",
                "decoded": "reader",
                "byteHex": "616263"
            })
        );
    }

    #[test]
    fn md5_helpers_match_legado_and_old_core_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalFull: md5Encode("abc"),
                    globalShort: md5Encode16("abc"),
                    javaFull: java.md5Encode("abc"),
                    javaShort: java.md5Encode16("abc")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalFull": "900150983cd24fb0d6963f7d28e17f72",
                "globalShort": "3cd24fb0d6963f7d",
                "javaFull": "900150983cd24fb0d6963f7d28e17f72",
                "javaShort": "3cd24fb0d6963f7d"
            })
        );
    }

    #[test]
    fn hash_digest_matches_old_core_sha256_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalSha256: hashDigest("abc", "SHA-256"),
                    javaSha256: java.hashDigest("abc", "SHA-256")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalSha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "javaSha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            })
        );
    }

    #[test]
    fn java_digest_hex_matches_legado_and_old_core_hash_digest_fixture_paths() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    sha256: java.digestHex("abc", "SHA-256"),
                    md5: java.digestHex("abc", "MD5")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "md5": "900150983cd24fb0d6963f7d28e17f72"
            })
        );
    }

    #[test]
    fn java_digest_base64_str_matches_legado_digest_fixture_paths() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    sha256: java.digestBase64Str("abc", "SHA-256"),
                    md5: java.digestBase64Str("abc", "MD5")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "sha256": "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=",
                "md5": "kAFQmDzST7DWlj99KOF/cg=="
            })
        );
    }

    #[test]
    fn hmac_digest_matches_old_core_sha256_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalHmac: hmacDigest("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    javaHmac: java.hmacDigest("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalHmac": "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",
                "javaHmac": "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
            })
        );
    }

    #[test]
    fn hmac_hex_aliases_match_legado_and_old_core_hmac_digest_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    javaUpper: java.HMacHex("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    javaLower: java.hmacHex("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    globalUpper: HMacHex("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    globalLower: hmacHex("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key")
                })
                "#,
            )
            .unwrap();

        let expected = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";
        assert_eq!(
            result.value,
            json!({
                "javaUpper": expected,
                "javaLower": expected,
                "globalUpper": expected,
                "globalLower": expected
            })
        );
    }

    #[test]
    fn hmac_base64_aliases_match_legado_and_old_core_hmac_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    javaUpper: java.HMacBase64("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    javaLower: java.hmacBase64("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    globalUpper: HMacBase64("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key"),
                    globalLower: hmacBase64("The quick brown fox jumps over the lazy dog", "HMAC-SHA256", "key")
                })
                "#,
            )
            .unwrap();

        let expected = "97yD9DBThCSxMpjmqm+xQ+9NWaFJRhdZl0edvC0aPNg=";
        assert_eq!(
            result.value,
            json!({
                "javaUpper": expected,
                "javaLower": expected,
                "globalUpper": expected,
                "globalLower": expected
            })
        );
    }

    #[test]
    fn java_byte_string_helpers_round_trip_utf8_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    bytes: java.strToBytes("reader"),
                    text: java.bytesToStr("726561646572"),
                    unicode: java.bytesToStr(java.strToBytes("小说"))
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "bytes": "726561646572",
                "text": "reader",
                "unicode": "小说"
            })
        );
    }

    #[test]
    fn java_byte_string_helpers_honor_latin1_charset_overload_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    latin1Bytes: java.strToBytes("café", "ISO-8859-1"),
                    latin1Text: java.bytesToStr("636166e9", "ISO-8859-1"),
                    latin1ArrayText: java.bytesToStr([0x63, 0x61, 0x66, 0xe9], "ISO-8859-1"),
                    globalLatin1Bytes: strToBytes("café", "latin1"),
                    globalLatin1Text: bytesToStr("636166e9", "latin1")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "latin1Bytes": "636166e9",
                "latin1Text": "café",
                "latin1ArrayText": "café",
                "globalLatin1Bytes": "636166e9",
                "globalLatin1Text": "café"
            })
        );
    }

    #[test]
    fn byte_array_helpers_round_trip_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalText: bytesToStr(base64DecodeToByteArray("YWJj")),
                    javaText: java.bytesToStr(java.base64DecodeToByteArray("YWJj")),
                    globalBytesAreArray: Array.isArray(base64DecodeToByteArray("YWJj"))
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalText": "abc",
                "javaText": "abc",
                "globalBytesAreArray": true
            })
        );
    }

    #[test]
    fn java_encode_uri_percent_encodes_utf8_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    query: java.encodeURI("https://example.test/search?q=小说 1&safe=%2F"),
                    text: java.encodeURI("小说")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "query": "https://example.test/search?q=%E5%B0%8F%E8%AF%B4%201&safe=%2F",
                "text": "%E5%B0%8F%E8%AF%B4"
            })
        );
    }

    #[test]
    fn java_encode_uri_component_escapes_query_separators_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                java.encodeURIComponent("小说 1&safe=%2F/path")
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!("%E5%B0%8F%E8%AF%B4%201%26safe%3D%252F%2Fpath")
        );
    }

    #[test]
    fn encode_uri_global_aliases_honor_latin1_charset_like_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalUri: encodeURI("https://example.test/café path", "ISO-8859-1"),
                    javaUri: java.encodeURI("https://example.test/café path", "ISO-8859-1"),
                    globalComponent: encodeURIComponent("café/path", "latin1"),
                    javaComponent: java.encodeURIComponent("café/path", "latin1")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalUri": "https://example.test/caf%E9%20path",
                "javaUri": "https://example.test/caf%E9%20path",
                "globalComponent": "caf%E9%2Fpath",
                "javaComponent": "caf%E9%2Fpath"
            })
        );
    }

    #[test]
    fn time_format_utc_formats_with_offset_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalUtc: timeFormatUTC(1705276800000, "yyyy-MM-dd HH:mm", 0),
                    javaUtc: java.timeFormatUTC(1705276800000, "yyyy-MM-dd HH:mm", 0),
                    offset: timeFormatUTC(1705276800000, "yyyy-MM-dd HH:mm:ss", 8 * 60 * 60 * 1000)
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalUtc": "2024-01-15 00:00",
                "javaUtc": "2024-01-15 00:00",
                "offset": "2024-01-15 08:00:00"
            })
        );
    }

    #[test]
    fn time_format_matches_legado_default_and_old_core_format_argument() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalFormatted: timeFormat(1705276800000, "yyyy-MM-dd HH:mm"),
                    javaFormatted: java.timeFormat(1705276800000, "yyyy-MM-dd HH:mm"),
                    legadoDefault: java.timeFormat(1705276800000)
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalFormatted": "2024-01-15 08:00",
                "javaFormatted": "2024-01-15 08:00",
                "legadoDefault": "2024/01/15 08:00"
            })
        );
    }

    #[test]
    fn to_num_chapter_matches_legado_number_normalization() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalChinese: toNumChapter("正文 第 一百二十三 章 名称"),
                    javaFullWidth: java.toNumChapter("第１２章"),
                    passThrough: java.toNumChapter("Chapter 7")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalChinese": "第123章",
                "javaFullWidth": "第12章",
                "passThrough": "Chapter 7"
            })
        );
    }

    #[test]
    fn chinese_conversion_helpers_match_legado_and_old_core_subset() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalT2s: t2s("門會說書"),
                    javaT2s: java.t2s("門會說書"),
                    globalS2t: s2t("门会说书"),
                    javaS2t: java.s2t("门会说书")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalT2s": "门会说书",
                "javaT2s": "门会说书",
                "globalS2t": "門會說書",
                "javaS2t": "門會說書"
            })
        );
    }

    #[test]
    fn random_uuid_matches_legado_shape_and_changes_between_calls() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                (function() {
                    var globalUuid = randomUUID();
                    var javaUuid = java.randomUUID();
                    var uuidV4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
                    return {
                        globalShape: uuidV4.test(globalUuid),
                        javaShape: uuidV4.test(javaUuid),
                        distinct: globalUuid !== javaUuid
                    };
                })()
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalShape": true,
                "javaShape": true,
                "distinct": true
            })
        );
    }

    #[test]
    fn html_format_strips_blocks_and_decodes_entities_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalText: htmlFormat("<p>One&nbsp;&amp;&nbsp;Two</p><br><div>Three</div>"),
                    javaText: java.htmlFormat("<p>One&nbsp;&amp;&nbsp;Two</p><br><div>Three</div>")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "globalText": "One & Two\nThree",
                "javaText": "One & Two\nThree"
            })
        );
    }

    #[test]
    fn to_url_resolves_relative_paths_like_legado_and_old_core() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                (function() {
                    var resolved = toURL("/book/1", "https://owned.example/root/index.html");
                    var javaResolved = java.toURL("/book/1", "https://owned.example/root/index.html");
                    return {
                        text: String(resolved),
                        javaText: String(javaResolved),
                        host: resolved.host,
                        origin: resolved.origin,
                        pathname: resolved.pathname
                    };
                })()
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "text": "https://owned.example/book/1",
                "javaText": "https://owned.example/book/1",
                "host": "owned.example",
                "origin": "https://owned.example",
                "pathname": "/book/1"
            })
        );
    }

    #[test]
    fn log_type_returns_stable_js_type_names_like_old_core_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    objectType: logType({"a": 1}),
                    arrayType: logType([1, 2, 3]),
                    javaArrayType: java.logType([1, 2, 3]),
                    nullType: logType(null),
                    stringType: logType("reader")
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "objectType": "object",
                "arrayType": "array",
                "javaArrayType": "array",
                "nullType": "null",
                "stringType": "string"
            })
        );
    }

    #[test]
    fn get_web_view_ua_returns_controlled_default_like_old_core_fixture() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({
                    globalUa: getWebViewUA(),
                    javaUa: java.getWebViewUA()
                })
                "#,
            )
            .unwrap();

        let expected = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1";
        assert_eq!(
            result.value,
            json!({
                "globalUa": expected,
                "javaUa": expected
            })
        );
    }

    #[test]
    fn routes_get_source_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.getSource", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!({
                "sourceId": "fixture-source",
                "sourceName": "Fixture"
            }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                ({
                    javaSource: java.getSource().sourceId,
                    globalSource: getSource().sourceName
                })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "javaSource": "fixture-source",
                "globalSource": "Fixture"
            })
        );

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], HostDescriptor::GetSource);
        assert_eq!(calls[1], HostDescriptor::GetSource);
    }

    #[test]
    fn routes_java_get_string_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.getString", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!("Dune"))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox.evaluate(r#"java.getString("article h1")"#).unwrap();

        assert_eq!(result.value, json!("Dune"));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::GetString { rule } => {
                assert_eq!(rule, "article h1");
            }
            other => panic!("expected GetString, got {other:?}"),
        }
    }

    #[test]
    fn routes_java_get_string_list_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.getStringList", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!(["Dune", "Foundation"]))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(r#"java.getStringList("article a.title")"#)
            .unwrap();

        assert_eq!(result.value, json!(["Dune", "Foundation"]));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::GetStringList { rule } => {
                assert_eq!(rule, "article a.title");
            }
            other => panic!("expected GetStringList, got {other:?}"),
        }
    }

    #[test]
    fn routes_java_post_and_returns_json() {
        let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.post", move |descriptor| {
            captured.lock().unwrap().push(descriptor.clone());
            Ok(json!({
                "method": descriptor.callback_name(),
                "status": "posted"
            }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(r#"java.post("https://example.test/post", "body-text")"#)
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "method": "java.post",
                "status": "posted"
            })
        );

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            HostDescriptor::HttpPost { url, body, headers } => {
                assert_eq!(url, "https://example.test/post");
                assert_eq!(body, "body-text");
                assert_eq!(*headers, None);
            }
            other => panic!("expected HttpPost, got {other:?}"),
        }
    }

    #[test]
    fn rejects_java_get_without_string_url() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", |_| Ok(json!(null)));
        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

        let error = sandbox.evaluate("java.get(42)").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
        assert!(error.message.contains("URL argument must be a string"));
    }

    #[test]
    fn propagates_host_callback_errors() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", |_| Err(HostError::new("network unavailable")));
        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

        let error = sandbox
            .evaluate(r#"java.get("https://example.test")"#)
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::HostCallback);
        assert!(error.message.contains("network unavailable"));
    }

    #[test]
    fn rejects_unknown_host_method() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox
            .evaluate(r#"java.call("java.delete", "https://example.test")"#)
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::HostCallback);
        assert!(error.message.contains("unknown host method"));
    }

    // -----------------------------------------------------------------------
    // Empty results
    // -----------------------------------------------------------------------

    #[test]
    fn returns_empty_array_and_empty_object() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("[]").unwrap();
        assert_eq!(result.value, json!([]));

        let result = sandbox.evaluate("({})").unwrap();
        assert_eq!(result.value, json!({}));
    }

    #[test]
    fn undefined_and_null_both_become_json_null() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("undefined").unwrap();
        assert_eq!(result.value, json!(null));

        let result = sandbox.evaluate("null").unwrap();
        assert_eq!(result.value, json!(null));
    }

    #[test]
    fn empty_string_preserved_as_json_string() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("\"\"").unwrap();
        assert_eq!(result.value, json!(""));
    }

    // -----------------------------------------------------------------------
    // Error expressions
    // -----------------------------------------------------------------------

    #[test]
    fn maps_type_error_to_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("null.foo").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn maps_range_error_to_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("new Array(-1)").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
    }

    #[test]
    fn thrown_string_maps_to_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate(r#"throw "string error""#).unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
    }

    #[test]
    fn thrown_number_maps_to_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("throw 42").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
    }

    #[test]
    fn thrown_object_maps_to_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox
            .evaluate(r#"throw { code: 500, reason: "internal" }"#)
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
    }

    #[test]
    fn syntax_error_includes_message() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("var x = ;").unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Syntax);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn error_stack_is_captured_when_available() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox
            .evaluate(
                r#"
                function boom() { throw new Error("stack-test"); }
                boom();
                "#,
            )
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
        assert!(error.stack.is_some());
    }

    // -----------------------------------------------------------------------
    // Duplicate results
    // -----------------------------------------------------------------------

    #[test]
    fn arrays_preserve_duplicate_values() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("[1, 1, 2, 2, 3]").unwrap();
        assert_eq!(result.value, json!([1, 1, 2, 2, 3]));
    }

    #[test]
    fn arrays_preserve_duplicate_strings() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate(r#"["dup", "dup", "unique"]"#).unwrap();
        assert_eq!(result.value, json!(["dup", "dup", "unique"]));
    }

    #[test]
    fn object_keys_with_duplicate_values_preserved() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(r#"({ a: "same", b: "same", c: "different" })"#)
            .unwrap();
        assert_eq!(
            result.value,
            json!({ "a": "same", "b": "same", "c": "different" })
        );
    }

    // -----------------------------------------------------------------------
    // Encoding / escaping
    // -----------------------------------------------------------------------

    #[test]
    fn unicode_strings_round_trip_through_sandbox() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate(r#""日本語テスト 🚀 ✨""#).unwrap();
        assert_eq!(result.value, json!("日本語テスト 🚀 ✨"));
    }

    #[test]
    fn escaped_string_characters_preserved() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(r#""line1\nline2\ttabbed\\backslash""#)
            .unwrap();
        assert_eq!(result.value, json!("line1\nline2\ttabbed\\backslash"));
    }

    #[test]
    fn strings_with_quotes_and_special_chars() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate(r#""quote: \" end & <tag>""#).unwrap();
        assert_eq!(result.value, json!("quote: \" end & <tag>"));
    }

    #[test]
    fn unicode_escape_sequences_decoded() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate(r#""\u00e9""#).unwrap();
        assert_eq!(result.value, json!("é"));
    }

    #[test]
    fn nested_unicode_in_arrays_and_objects() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(r#"({ "标题": ["沙丘", "基地"], "作者": "弗兰克" })"#)
            .unwrap();
        assert_eq!(
            result.value,
            json!({ "标题": ["沙丘", "基地"], "作者": "弗兰克" })
        );
    }

    // -----------------------------------------------------------------------
    // JS expression edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn negative_and_float_numbers_preserved() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("[-1, -2.5, 0, 3.14159]").unwrap();
        assert_eq!(result.value, json!([-1, -2.5, 0, 3.14159]));
    }

    #[test]
    fn deeply_nested_object_converts() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(
                r#"
                ({ a: { b: { c: { d: { e: "deep" } } } } })
                "#,
            )
            .unwrap();
        assert_eq!(
            result.value,
            json!({ "a": { "b": { "c": { "d": { "e": "deep" } } } } })
        );
    }

    #[test]
    fn function_value_rejected_as_non_json() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("(function() { return 1; })").unwrap_err();
        assert_eq!(error.kind, JsErrorKind::NonJsonValue);
    }

    #[test]
    fn symbol_value_rejected_as_non_json() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("Symbol('test')").unwrap_err();
        assert_eq!(error.kind, JsErrorKind::NonJsonValue);
    }

    #[test]
    fn boolean_and_number_constants() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("true").unwrap();
        assert_eq!(result.value, json!(true));

        let result = sandbox.evaluate("false").unwrap();
        assert_eq!(result.value, json!(false));

        let result = sandbox.evaluate("42").unwrap();
        assert_eq!(result.value, json!(42));
    }

    #[test]
    fn nan_and_infinity_rejected_as_non_json() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox.evaluate("NaN").unwrap_err();
        assert_eq!(error.kind, JsErrorKind::NonJsonValue);

        let error = sandbox.evaluate("Infinity").unwrap_err();
        assert_eq!(error.kind, JsErrorKind::NonJsonValue);
    }

    // -----------------------------------------------------------------------
    // Fallback / host callback edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn host_callback_returning_null_propagates_null() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", |_| Ok(json!(null)));
        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

        let result = sandbox
            .evaluate(r#"java.get("https://example.test")"#)
            .unwrap();
        assert_eq!(result.value, json!(null));
    }

    #[test]
    fn host_callback_returning_empty_array_propagates() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", |_| Ok(json!([])));
        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

        let result = sandbox
            .evaluate(r#"java.get("https://example.test")"#)
            .unwrap();
        assert_eq!(result.value, json!([]));
    }

    #[test]
    fn unregistered_host_callback_errors() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox
            .evaluate(r#"java.get("https://example.test")"#)
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::HostCallback);
        assert!(error.message.contains("unregistered"));
    }

    #[test]
    fn host_callback_can_provide_fallback_data() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", move |_| {
            Ok(json!({
                "title": "fallback-title",
                "items": ["a", "a", "b"]
            }))
        });
        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

        let result = sandbox
            .evaluate(
                r#"
                const response = java.get("https://example.test");
                ({ title: response.title, count: response.items.length })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({ "title": "fallback-title", "count": 3 })
        );
    }

    // -----------------------------------------------------------------------
    // Console capture edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn console_log_with_no_arguments_captured() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("console.log(); 'done'").unwrap();
        assert_eq!(result.value, json!("done"));
        assert_eq!(result.console.len(), 1);
        assert!(result.console[0].args.is_empty());
    }

    #[test]
    fn console_log_with_null_and_undefined() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate("console.log(null, undefined); 'done'")
            .unwrap();

        assert_eq!(result.console.len(), 1);
        assert_eq!(result.console[0].args, vec![json!(null), json!(null)]);
    }

    #[test]
    fn console_log_preserves_duplicate_arguments() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate(r#"console.log("dup", "dup", "dup"); 'done'"#)
            .unwrap();

        assert_eq!(
            result.console[0].args,
            vec![json!("dup"), json!("dup"), json!("dup")]
        );
    }

    // -----------------------------------------------------------------------
    // Promise edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn rejected_promise_maps_to_exception() {
        let sandbox = QuickJsSandbox::default();

        let error = sandbox
            .evaluate("Promise.reject(new Error('rejected'))")
            .unwrap_err();

        assert_eq!(error.kind, JsErrorKind::Exception);
        assert!(error.message.contains("rejected"));
    }

    #[test]
    fn promise_resolving_with_null() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("Promise.resolve(null)").unwrap();
        assert_eq!(result.value, json!(null));
    }

    #[test]
    fn promise_resolving_with_empty_object() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox.evaluate("Promise.resolve({})").unwrap();
        assert_eq!(result.value, json!({}));
    }

    #[test]
    fn promise_chain_preserves_duplicate_values() {
        let sandbox = QuickJsSandbox::default();

        let result = sandbox
            .evaluate("Promise.resolve([1, 1, 2]).then(arr => arr)")
            .unwrap();
        assert_eq!(result.value, json!([1, 1, 2]));
    }
}

// ============================================================================
// P3 residual host routing: java.downloadFile / cacheFile / importScript /
// setContent / put / reGetBook.
//
// These host-routed methods are deliberately NOT added to the `HostMethod`
// enum, to keep this change merge-safe with concurrent edits to that enum on
// `codex/reader-js-compat-runtime` (Head/WebView/StartBrowser/getCookie are
// being added there by another work stream). They reuse the existing
// `HostCall` descriptor + `HostCallbackRegistry` boundary, so the host
// receives the same `{name, args}` descriptor shape as `java.ajax` etc. and
// executes the real network/file/state operation; reader-js only routes.
// ============================================================================

#[derive(Clone, Copy, Debug)]
enum ResidualArgShape {
    /// No required args (extras ignored, matching legado leniency). e.g. reGetBook.
    NoArgs,
    /// First arg must be a URL string. e.g. downloadFile, importScript.
    UrlString,
    /// First arg URL string; optional second arg passes through. e.g. cacheFile(url[, saveTime]).
    UrlStringOptExtra,
    /// First arg content string (or null); optional second base-url string. e.g. setContent.
    ContentOptBase,
    /// Exactly two string args (key, value). e.g. put.
    KeyValueStrings,
    // ===== S3 closure: 15 new arg shapes for the 28 new methods =====
    /// URL string + optional headers object (passes through as JSON). e.g. head.
    UrlAndHeaders,
    /// Three nullable strings (html, url, js). e.g. webView.
    WebViewArgs,
    /// Three nullable strings + required regex string. e.g. webViewGetSource/OverrideUrl.
    WebViewRegexArgs,
    /// URL + title strings. e.g. startBrowser.
    UrlAndTitle,
    /// URL + title strings + optional bool. e.g. startBrowserAwait(url, title[, refetch]).
    UrlTitleOptBool,
    /// Single image-URL string. e.g. getVerificationCode.
    ImageUrl,
    /// Tag string + optional key string. e.g. getCookie(tag[, key]).
    TagOptKey,
    /// Single path string. e.g. getFile, readFile, deleteFile, unzip*, getTxtInFolder.
    PathString,
    /// Path string + optional charset string. e.g. readTxtFile(path[, charset]).
    PathOptCharset,
    /// URL + inner-path strings + optional charset. e.g. getZip/Rar/7zStringContent.
    UrlPathOptCharset,
    /// URL + inner-path strings. e.g. getZip/Rar/7zByteArrayContent.
    UrlPath,
    /// Any data + optional useCache bool. e.g. queryTTF(data[, useCache]).
    TtfDataOptBool,
    /// text + errorTTF + correctTTF + optional filter bool. e.g. replaceFont.
    ReplaceFontArgs,
    /// URL string + optional mimeType string. e.g. openUrl(url[, mimeType]).
    UrlOptMime,
}

fn require_url_string_arg<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    args: &[JsonValue],
) -> Result<(), QuickJsError> {
    let Some(url) = args.first() else {
        return Err(Exception::throw_type(
            ctx,
            format!("{name} requires a URL string argument").as_str(),
        ));
    };
    if !url.is_string() {
        return Err(Exception::throw_type(
            ctx,
            format!("{name} URL argument must be a string").as_str(),
        ));
    }
    Ok(())
}

fn validate_residual_args<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    shape: ResidualArgShape,
    args: &[JsonValue],
) -> Result<(), QuickJsError> {
    match shape {
        ResidualArgShape::NoArgs => Ok(()),
        ResidualArgShape::UrlString | ResidualArgShape::UrlStringOptExtra => {
            require_url_string_arg(ctx, name, args)
        }
        ResidualArgShape::ContentOptBase => {
            let Some(content) = args.first() else {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} requires a content argument").as_str(),
                ));
            };
            if !content.is_string() && !content.is_null() {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} content argument must be a string or null").as_str(),
                ));
            }
            Ok(())
        }
        ResidualArgShape::KeyValueStrings => {
            if args.len() < 2 {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} requires key and value string arguments").as_str(),
                ));
            }
            if !args[0].is_string() || !args[1].is_string() {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} key and value arguments must be strings").as_str(),
                ));
            }
            Ok(())
        }
        // S3 closure shapes
        ResidualArgShape::UrlAndHeaders => require_url_string_arg(ctx, name, args),
        ResidualArgShape::WebViewArgs => {
            // 3 nullable strings (html, url, js). Legado allows null/undefined
            // for each; we accept string or null.
            for (i, arg) in args.iter().take(3).enumerate() {
                if !arg.is_string() && !arg.is_null() {
                    return Err(Exception::throw_type(
                        ctx,
                        format!("{name} argument {} must be a string or null", i + 1).as_str(),
                    ));
                }
            }
            Ok(())
        }
        ResidualArgShape::WebViewRegexArgs => {
            // 3 nullable strings + 4th required regex string
            for (i, arg) in args.iter().take(3).enumerate() {
                if !arg.is_string() && !arg.is_null() {
                    return Err(Exception::throw_type(
                        ctx,
                        format!("{name} argument {} must be a string or null", i + 1).as_str(),
                    ));
                }
            }
            let Some(regex) = args.get(3) else {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} requires a regex string as 4th argument").as_str(),
                ));
            };
            if !regex.is_string() {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} regex argument must be a string").as_str(),
                ));
            }
            Ok(())
        }
        ResidualArgShape::UrlAndTitle => require_two_string_args(ctx, name, args),
        ResidualArgShape::UrlTitleOptBool => {
            require_two_string_args(ctx, name, args)?;
            // optional 3rd bool — pass through if present
            Ok(())
        }
        ResidualArgShape::ImageUrl => require_url_string_arg(ctx, name, args),
        ResidualArgShape::TagOptKey => {
            require_url_string_arg(ctx, name, args)?;
            // optional 2nd key string — pass through
            Ok(())
        }
        ResidualArgShape::PathString => require_url_string_arg(ctx, name, args),
        ResidualArgShape::PathOptCharset => {
            require_url_string_arg(ctx, name, args)?;
            // optional 2nd charset string — pass through
            Ok(())
        }
        ResidualArgShape::UrlPathOptCharset => require_two_string_args(ctx, name, args),
        ResidualArgShape::UrlPath => require_two_string_args(ctx, name, args),
        ResidualArgShape::TtfDataOptBool => {
            // 1st arg required (any type: url/file/base64/bytes); optional 2nd bool
            if args.is_empty() {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} requires a data argument").as_str(),
                ));
            }
            Ok(())
        }
        ResidualArgShape::ReplaceFontArgs => {
            // text + errorTTF + correctTTF required; optional filter bool
            if args.len() < 3 {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} requires text, errorTTF, correctTTF arguments").as_str(),
                ));
            }
            if !args[0].is_string() {
                return Err(Exception::throw_type(
                    ctx,
                    format!("{name} text argument must be a string").as_str(),
                ));
            }
            Ok(())
        }
        ResidualArgShape::UrlOptMime => require_url_string_arg(ctx, name, args),
    }
}

/// Require exactly two leading string arguments. Used by startBrowser(url, title),
/// getZipStringContent(url, path), etc.
fn require_two_string_args<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    args: &[JsonValue],
) -> Result<(), QuickJsError> {
    if args.len() < 2 {
        return Err(Exception::throw_type(
            ctx,
            format!("{name} requires two string arguments").as_str(),
        ));
    }
    if !args[0].is_string() || !args[1].is_string() {
        return Err(Exception::throw_type(
            ctx,
            format!("{name} first two arguments must be strings").as_str(),
        ));
    }
    Ok(())
}

/// Coerce a JSON value to an `Option<String>` — `null`/missing → `None`,
/// string → `Some(s)`, anything else → `None` (legado leniency for nullable
/// string args like webView's html/url/js).
fn nullable_string(value: Option<&JsonValue>) -> Option<String> {
    value.and_then(|v| {
        if v.is_null() {
            None
        } else {
            v.as_str().map(String::from)
        }
    })
}

fn make_residual_host_callback<'js>(
    ctx: Ctx<'js>,
    name: &'static str,
    shape: ResidualArgShape,
    registry: HostCallbackRegistry,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |ctx: Ctx<'js>,
              args: Rest<QuickJsValue<'js>>|
              -> Result<QuickJsValue<'js>, QuickJsError> {
            let json_args = host_args_to_json(&ctx, &args.0)?;
            validate_residual_args(&ctx, name, shape, &json_args)?;
            let descriptor = build_residual_descriptor(&ctx, name, shape, &json_args)?;

            let result = registry.call(name, descriptor).map_err(|error| {
                Exception::throw_internal(
                    &ctx,
                    format!("host callback {name} failed: {error}").as_str(),
                )
            })?;

            json_to_quickjs(&ctx, &result).map_err(|_| {
                Exception::throw_internal(
                    &ctx,
                    format!("host callback {name} returned invalid JSON").as_str(),
                )
            })
        },
    )
}

/// Wire the residual `java.*` host-routed methods (downloadFile, cacheFile,
/// importScript, setContent, put, reGetBook) plus top-level global aliases for
/// the network/file helpers (mirroring how `ajax`/`getSource` are exposed both
/// as `java.ajax` and global `ajax`). The state helpers (setContent/put/
/// reGetBook) stay java-namespaced.
///
/// Called from `install_host_api` right before the `java` object is published
/// to the global scope. Uses a dedicated routing path so the `HostMethod` enum
/// stays untouched (concurrent-agent safety).
fn install_residual_host_routing<'js>(
    ctx: Ctx<'js>,
    java: &rquickjs::Object<'js>,
    registry: HostCallbackRegistry,
) -> Result<(), QuickJsError> {
    use ResidualArgShape::*;

    java.set(
        "downloadFile",
        make_residual_host_callback(
            ctx.clone(),
            "java.downloadFile",
            UrlString,
            registry.clone(),
        )?,
    )?;
    java.set(
        "cacheFile",
        make_residual_host_callback(
            ctx.clone(),
            "java.cacheFile",
            UrlStringOptExtra,
            registry.clone(),
        )?,
    )?;
    java.set(
        "importScript",
        make_residual_host_callback(
            ctx.clone(),
            "java.importScript",
            UrlString,
            registry.clone(),
        )?,
    )?;
    java.set(
        "setContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.setContent",
            ContentOptBase,
            registry.clone(),
        )?,
    )?;
    java.set(
        "put",
        make_residual_host_callback(ctx.clone(), "java.put", KeyValueStrings, registry.clone())?,
    )?;
    java.set(
        "reGetBook",
        make_residual_host_callback(ctx.clone(), "java.reGetBook", NoArgs, registry.clone())?,
    )?;

    // ===== S3 closure: 28 new java.* methods routed through host callbacks =====
    // Network/HTTP
    java.set(
        "head",
        make_residual_host_callback(ctx.clone(), "java.head", UrlAndHeaders, registry.clone())?,
    )?;
    // WebView/Browser
    java.set(
        "webView",
        make_residual_host_callback(ctx.clone(), "java.webView", WebViewArgs, registry.clone())?,
    )?;
    java.set(
        "webViewGetSource",
        make_residual_host_callback(
            ctx.clone(),
            "java.webViewGetSource",
            WebViewRegexArgs,
            registry.clone(),
        )?,
    )?;
    java.set(
        "webViewGetOverrideUrl",
        make_residual_host_callback(
            ctx.clone(),
            "java.webViewGetOverrideUrl",
            WebViewRegexArgs,
            registry.clone(),
        )?,
    )?;
    java.set(
        "startBrowser",
        make_residual_host_callback(
            ctx.clone(),
            "java.startBrowser",
            UrlAndTitle,
            registry.clone(),
        )?,
    )?;
    java.set(
        "startBrowserAwait",
        make_residual_host_callback(
            ctx.clone(),
            "java.startBrowserAwait",
            UrlTitleOptBool,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getVerificationCode",
        make_residual_host_callback(
            ctx.clone(),
            "java.getVerificationCode",
            ImageUrl,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getCookie",
        make_residual_host_callback(ctx.clone(), "java.getCookie", TagOptKey, registry.clone())?,
    )?;
    // File/Archive
    java.set(
        "getFile",
        make_residual_host_callback(ctx.clone(), "java.getFile", PathString, registry.clone())?,
    )?;
    java.set(
        "readFile",
        make_residual_host_callback(ctx.clone(), "java.readFile", PathString, registry.clone())?,
    )?;
    java.set(
        "readTxtFile",
        make_residual_host_callback(
            ctx.clone(),
            "java.readTxtFile",
            PathOptCharset,
            registry.clone(),
        )?,
    )?;
    java.set(
        "deleteFile",
        make_residual_host_callback(ctx.clone(), "java.deleteFile", PathString, registry.clone())?,
    )?;
    java.set(
        "unzipFile",
        make_residual_host_callback(ctx.clone(), "java.unzipFile", PathString, registry.clone())?,
    )?;
    java.set(
        "un7zFile",
        make_residual_host_callback(ctx.clone(), "java.un7zFile", PathString, registry.clone())?,
    )?;
    java.set(
        "unrarFile",
        make_residual_host_callback(ctx.clone(), "java.unrarFile", PathString, registry.clone())?,
    )?;
    java.set(
        "unArchiveFile",
        make_residual_host_callback(
            ctx.clone(),
            "java.unArchiveFile",
            PathString,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getTxtInFolder",
        make_residual_host_callback(
            ctx.clone(),
            "java.getTxtInFolder",
            PathString,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getZipStringContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.getZipStringContent",
            UrlPathOptCharset,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getRarStringContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.getRarStringContent",
            UrlPathOptCharset,
            registry.clone(),
        )?,
    )?;
    java.set(
        "get7zStringContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.get7zStringContent",
            UrlPathOptCharset,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getZipByteArrayContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.getZipByteArrayContent",
            UrlPath,
            registry.clone(),
        )?,
    )?;
    java.set(
        "getRarByteArrayContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.getRarByteArrayContent",
            UrlPath,
            registry.clone(),
        )?,
    )?;
    java.set(
        "get7zByteArrayContent",
        make_residual_host_callback(
            ctx.clone(),
            "java.get7zByteArrayContent",
            UrlPath,
            registry.clone(),
        )?,
    )?;
    // Font/TTF
    java.set(
        "queryBase64TTF",
        make_residual_host_callback(
            ctx.clone(),
            "java.queryBase64TTF",
            TtfDataOptBool,
            registry.clone(),
        )?,
    )?;
    java.set(
        "queryTTF",
        make_residual_host_callback(
            ctx.clone(),
            "java.queryTTF",
            TtfDataOptBool,
            registry.clone(),
        )?,
    )?;
    java.set(
        "replaceFont",
        make_residual_host_callback(
            ctx.clone(),
            "java.replaceFont",
            ReplaceFontArgs,
            registry.clone(),
        )?,
    )?;
    // Device/UI
    java.set(
        "androidId",
        make_residual_host_callback(ctx.clone(), "java.androidId", NoArgs, registry.clone())?,
    )?;
    java.set(
        "openUrl",
        make_residual_host_callback(ctx.clone(), "java.openUrl", UrlOptMime, registry.clone())?,
    )?;

    let globals = ctx.globals();
    globals.set(
        "downloadFile",
        make_residual_host_callback(
            ctx.clone(),
            "java.downloadFile",
            UrlString,
            registry.clone(),
        )?,
    )?;
    globals.set(
        "cacheFile",
        make_residual_host_callback(
            ctx.clone(),
            "java.cacheFile",
            UrlStringOptExtra,
            registry.clone(),
        )?,
    )?;
    globals.set(
        "importScript",
        make_residual_host_callback(
            ctx.clone(),
            "java.importScript",
            UrlString,
            registry.clone(),
        )?,
    )?;
    Ok(())
}

// ============================================================================
// P3 follow-up: HostDescriptor construction.
//
// `build_host_descriptor` and `build_residual_descriptor` take already-validated
// JSON args and construct the strong-typed `HostDescriptor` variant that the
// host callback receives. Validation is still done by `HostMethod::validate_args`
// and `validate_residual_args` (unchanged, merge-safe with concurrent agent on
// `codex/reader-js-compat-runtime`); these builders only pattern-match + extract
// typed fields.
// ============================================================================

/// Build a `HostDescriptor` for one of the 8 `HostMethod` enum variants.
/// `args` MUST have been validated by `HostMethod::validate_args` first.
fn build_host_descriptor(
    method: HostMethod,
    args: &[JsonValue],
) -> Result<HostDescriptor, QuickJsError> {
    Ok(match method {
        HostMethod::Get => HostDescriptor::HttpGet {
            url: args[0].as_str().expect("validated URL string").to_string(),
            headers: args.get(1).cloned(),
        },
        HostMethod::Post => HostDescriptor::HttpPost {
            url: args[0].as_str().expect("validated URL string").to_string(),
            body: args
                .get(1)
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string(),
            headers: args.get(2).cloned(),
        },
        HostMethod::Connect => HostDescriptor::HttpConnect {
            url: args[0].as_str().expect("validated URL string").to_string(),
            header: args.get(1).and_then(JsonValue::as_str).map(String::from),
        },
        HostMethod::Ajax => HostDescriptor::Ajax {
            // legado: if `url` is a list, take the first element. host_args_to_json
            // converts JS arrays to JsonValue::Array; we extract the first string.
            url: args
                .first()
                .map(|v| {
                    if let Some(arr) = v.as_array() {
                        arr.first()
                            .and_then(JsonValue::as_str)
                            .unwrap_or("")
                            .to_string()
                    } else {
                        v.as_str().unwrap_or("").to_string()
                    }
                })
                .unwrap_or_default(),
        },
        HostMethod::AjaxAll => HostDescriptor::AjaxAll {
            urls: args
                .first()
                .and_then(JsonValue::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        },
        HostMethod::GetSource => HostDescriptor::GetSource,
        HostMethod::GetString => HostDescriptor::GetString {
            rule: args[0].as_str().expect("validated rule string").to_string(),
        },
        HostMethod::GetStringList => HostDescriptor::GetStringList {
            rule: args[0].as_str().expect("validated rule string").to_string(),
        },
    })
}

/// Build a `HostDescriptor` for one of the 6 residual methods (downloadFile,
/// cacheFile, importScript, setContent, put, reGetBook). `args` MUST have been
/// validated by `validate_residual_args` first.
fn build_residual_descriptor<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    shape: ResidualArgShape,
    args: &[JsonValue],
) -> Result<HostDescriptor, QuickJsError> {
    Ok(match (name, shape) {
        ("java.downloadFile", ResidualArgShape::UrlString) => HostDescriptor::DownloadFile {
            url: args[0].as_str().expect("validated URL string").to_string(),
        },
        ("java.cacheFile", ResidualArgShape::UrlStringOptExtra) => HostDescriptor::CacheFile {
            url: args[0].as_str().expect("validated URL string").to_string(),
            save_time: args.get(1).and_then(JsonValue::as_i64),
        },
        ("java.importScript", ResidualArgShape::UrlString) => HostDescriptor::ImportScript {
            path: args[0].as_str().expect("validated URL string").to_string(),
        },
        ("java.setContent", ResidualArgShape::ContentOptBase) => HostDescriptor::SetContent {
            content: args.first().and_then(|v| {
                if v.is_null() {
                    None
                } else {
                    v.as_str().map(String::from)
                }
            }),
            base_url: args.get(1).and_then(JsonValue::as_str).map(String::from),
        },
        ("java.put", ResidualArgShape::KeyValueStrings) => HostDescriptor::Put {
            key: args[0].as_str().expect("validated key string").to_string(),
            value: args[1]
                .as_str()
                .expect("validated value string")
                .to_string(),
        },
        ("java.reGetBook", ResidualArgShape::NoArgs) => HostDescriptor::ReGetBook,
        // ===== S3 closure: 28 new descriptor builders =====
        ("java.head", ResidualArgShape::UrlAndHeaders) => HostDescriptor::HttpHead {
            url: args[0].as_str().expect("validated URL string").to_string(),
            headers: args.get(1).cloned(),
        },
        ("java.webView", ResidualArgShape::WebViewArgs) => HostDescriptor::WebView {
            html: nullable_string(args.first()),
            url: nullable_string(args.get(1)),
            js: nullable_string(args.get(2)),
        },
        ("java.webViewGetSource", ResidualArgShape::WebViewRegexArgs) => {
            HostDescriptor::WebViewGetSource {
                html: nullable_string(args.first()),
                url: nullable_string(args.get(1)),
                js: nullable_string(args.get(2)),
                source_regex: args[3]
                    .as_str()
                    .expect("validated regex string")
                    .to_string(),
            }
        }
        ("java.webViewGetOverrideUrl", ResidualArgShape::WebViewRegexArgs) => {
            HostDescriptor::WebViewGetOverrideUrl {
                html: nullable_string(args.first()),
                url: nullable_string(args.get(1)),
                js: nullable_string(args.get(2)),
                override_url_regex: args[3]
                    .as_str()
                    .expect("validated regex string")
                    .to_string(),
            }
        }
        ("java.startBrowser", ResidualArgShape::UrlAndTitle) => HostDescriptor::StartBrowser {
            url: args[0].as_str().expect("validated URL string").to_string(),
            title: args[1]
                .as_str()
                .expect("validated title string")
                .to_string(),
        },
        ("java.startBrowserAwait", ResidualArgShape::UrlTitleOptBool) => {
            HostDescriptor::StartBrowserAwait {
                url: args[0].as_str().expect("validated URL string").to_string(),
                title: args[1]
                    .as_str()
                    .expect("validated title string")
                    .to_string(),
                refetch_after_success: args.get(2).and_then(JsonValue::as_bool),
            }
        }
        ("java.getVerificationCode", ResidualArgShape::ImageUrl) => {
            HostDescriptor::GetVerificationCode {
                image_url: args[0].as_str().expect("validated URL string").to_string(),
            }
        }
        ("java.getCookie", ResidualArgShape::TagOptKey) => HostDescriptor::GetCookie {
            tag: args[0].as_str().expect("validated tag string").to_string(),
            key: args.get(1).and_then(JsonValue::as_str).map(String::from),
        },
        ("java.getFile", ResidualArgShape::PathString) => HostDescriptor::GetFile {
            path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.readFile", ResidualArgShape::PathString) => HostDescriptor::ReadFile {
            path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.readTxtFile", ResidualArgShape::PathOptCharset) => HostDescriptor::ReadTxtFile {
            path: args[0].as_str().expect("validated path string").to_string(),
            charset: args.get(1).and_then(JsonValue::as_str).map(String::from),
        },
        ("java.deleteFile", ResidualArgShape::PathString) => HostDescriptor::DeleteFile {
            path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.unzipFile", ResidualArgShape::PathString) => HostDescriptor::UnzipFile {
            zip_path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.un7zFile", ResidualArgShape::PathString) => HostDescriptor::Un7zFile {
            zip_path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.unrarFile", ResidualArgShape::PathString) => HostDescriptor::UnrarFile {
            zip_path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.unArchiveFile", ResidualArgShape::PathString) => HostDescriptor::UnArchiveFile {
            zip_path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.getTxtInFolder", ResidualArgShape::PathString) => HostDescriptor::GetTxtInFolder {
            path: args[0].as_str().expect("validated path string").to_string(),
        },
        ("java.getZipStringContent", ResidualArgShape::UrlPathOptCharset) => {
            HostDescriptor::GetZipStringContent {
                url: args[0].as_str().expect("validated URL string").to_string(),
                path: args[1].as_str().expect("validated path string").to_string(),
                charset: args.get(2).and_then(JsonValue::as_str).map(String::from),
            }
        }
        ("java.getRarStringContent", ResidualArgShape::UrlPathOptCharset) => {
            HostDescriptor::GetRarStringContent {
                url: args[0].as_str().expect("validated URL string").to_string(),
                path: args[1].as_str().expect("validated path string").to_string(),
                charset: args.get(2).and_then(JsonValue::as_str).map(String::from),
            }
        }
        ("java.get7zStringContent", ResidualArgShape::UrlPathOptCharset) => {
            HostDescriptor::Get7zStringContent {
                url: args[0].as_str().expect("validated URL string").to_string(),
                path: args[1].as_str().expect("validated path string").to_string(),
                charset: args.get(2).and_then(JsonValue::as_str).map(String::from),
            }
        }
        ("java.getZipByteArrayContent", ResidualArgShape::UrlPath) => {
            HostDescriptor::GetZipByteArrayContent {
                url: args[0].as_str().expect("validated URL string").to_string(),
                path: args[1].as_str().expect("validated path string").to_string(),
            }
        }
        ("java.getRarByteArrayContent", ResidualArgShape::UrlPath) => {
            HostDescriptor::GetRarByteArrayContent {
                url: args[0].as_str().expect("validated URL string").to_string(),
                path: args[1].as_str().expect("validated path string").to_string(),
            }
        }
        ("java.get7zByteArrayContent", ResidualArgShape::UrlPath) => {
            HostDescriptor::Get7zByteArrayContent {
                url: args[0].as_str().expect("validated URL string").to_string(),
                path: args[1].as_str().expect("validated path string").to_string(),
            }
        }
        ("java.queryBase64TTF", ResidualArgShape::TtfDataOptBool) => {
            HostDescriptor::QueryBase64TTF {
                data: args[0]
                    .as_str()
                    .expect("validated base64 string")
                    .to_string(),
            }
        }
        ("java.queryTTF", ResidualArgShape::TtfDataOptBool) => HostDescriptor::QueryTTF {
            data: args[0].clone(),
            use_cache: args.get(1).and_then(JsonValue::as_bool),
        },
        ("java.replaceFont", ResidualArgShape::ReplaceFontArgs) => HostDescriptor::ReplaceFont {
            text: args[0].as_str().expect("validated text string").to_string(),
            error_query_ttf: args[1].clone(),
            correct_query_ttf: args[2].clone(),
            filter: args.get(3).and_then(JsonValue::as_bool),
        },
        ("java.androidId", ResidualArgShape::NoArgs) => HostDescriptor::AndroidId,
        ("java.openUrl", ResidualArgShape::UrlOptMime) => HostDescriptor::OpenUrl {
            url: args[0].as_str().expect("validated URL string").to_string(),
            mime_type: args.get(1).and_then(JsonValue::as_str).map(String::from),
        },
        (other_name, other_shape) => {
            return Err(Exception::throw_internal(
                ctx,
                // Should be unreachable: install_residual_host_routing only wires
                // the shapes above. Defensive — never expected to fire.
                format!(
                    "build_residual_descriptor: unconfigured mapping for {other_name} ({other_shape:?})"
                )
                .as_str(),
            ));
        }
    })
}

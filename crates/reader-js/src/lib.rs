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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCall {
    pub name: String,
    pub args: Vec<JsonValue>,
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

type HostCallback = Arc<dyn Fn(HostCall) -> Result<JsonValue, HostError> + Send + Sync + 'static>;
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
        F: Fn(HostCall) -> Result<JsonValue, HostError> + Send + Sync + 'static,
    {
        self.callbacks.insert(name.into(), Arc::new(callback));
    }

    pub fn contains(&self, name: &str) -> bool {
        self.callbacks.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.callbacks.keys().cloned().collect()
    }

    fn call(&self, call: HostCall) -> Result<JsonValue, HostError> {
        let callback = self
            .callbacks
            .get(call.name.as_str())
            .ok_or_else(|| HostError::new(format!("unregistered host callback: {}", call.name)))?;
        callback(call)
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
        ctx.globals().set("java", java)?;
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

            let result = registry
                .call(HostCall {
                    name: method.callback_name().to_string(),
                    args: json_args,
                })
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
        |ctx: Ctx<'js>, input: String, algorithm: String| -> Result<String, QuickJsError> {
            hash_digest_hex(input.as_bytes(), &algorithm).ok_or_else(|| {
                Exception::throw_type(
                    &ctx,
                    format!("hashDigest unsupported algorithm: {algorithm}").as_str(),
                )
            })
        },
    )
}

fn make_hash_digest_base64_callback<'js>(
    ctx: Ctx<'js>,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        |ctx: Ctx<'js>, input: String, algorithm: String| -> Result<String, QuickJsError> {
            hash_digest_base64(input.as_bytes(), &algorithm).ok_or_else(|| {
                Exception::throw_type(
                    &ctx,
                    format!("digestBase64Str unsupported algorithm: {algorithm}").as_str(),
                )
            })
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
        |ctx: Ctx<'js>,
         args: Rest<QuickJsValue<'js>>|
         -> Result<rquickjs::Object<'js>, QuickJsError> {
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
            let base_url = args
                .0
                .get(1)
                .and_then(|value| quickjs_value_to_json(value, 0).ok())
                .and_then(|value| value.as_str().map(ToOwned::to_owned));
            let parts = resolve_js_url(&url, base_url.as_deref()).map_err(|message| {
                Exception::throw_type(&ctx, format!("toURL failed: {message}").as_str())
            })?;
            js_url_to_object(&ctx, parts)
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
            let result = registry
                .call(HostCall {
                    name: method.callback_name().to_string(),
                    args: json_args,
                })
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

fn hash_digest_hex(input: &[u8], algorithm: &str) -> Option<String> {
    hash_digest_bytes(input, algorithm).map(|bytes| hex_encode(&bytes))
}

fn hash_digest_base64(input: &[u8], algorithm: &str) -> Option<String> {
    hash_digest_bytes(input, algorithm).map(|bytes| base64_encode_with_flags(&bytes, 0))
}

fn hash_digest_bytes(input: &[u8], algorithm: &str) -> Option<Vec<u8>> {
    match normalize_digest_algorithm(algorithm).as_str() {
        "md5" => Some(md5_digest(input).to_vec()),
        "sha256" => Some(sha256_digest(input).to_vec()),
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
        "hmacsha256" => Some(hmac_sha256_digest(input, key).to_vec()),
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
        _ => None,
    }
}

fn gbk_pair_to_char(lead: u8, trail: u8) -> Option<char> {
    match (lead, trail) {
        (0xd0, 0xa1) => Some('小'),
        (0xcb, 0xb5) => Some('说'),
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
    let resolved_path = if path.starts_with('/') {
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
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.get", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!({
                "status": "stubbed",
                "args": call.args
            }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.get("https://example.test", { headers: { Accept: "text/plain" } })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "status": "stubbed",
                "args": [
                    "https://example.test",
                    { "headers": { "Accept": "text/plain" } }
                ]
            })
        );

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "java.get");
        assert_eq!(
            calls[0].args,
            vec![
                json!("https://example.test"),
                json!({ "headers": { "Accept": "text/plain" } })
            ]
        );
    }

    #[test]
    fn routes_java_connect_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.connect", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!({
                "status": "connected",
                "args": call.args
            }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.connect("https://example.test", { headers: { Accept: "text/plain" } })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "status": "connected",
                "args": [
                    "https://example.test",
                    { "headers": { "Accept": "text/plain" } }
                ]
            })
        );

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "java.connect");
        assert_eq!(
            calls[0].args,
            vec![
                json!("https://example.test"),
                json!({ "headers": { "Accept": "text/plain" } })
            ]
        );
    }

    #[test]
    fn routes_java_ajax_all_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajaxAll", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!([
                { "url": call.args[0][0]["url"], "status": "ok" },
                { "url": call.args[0][1]["url"], "status": "ok" }
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
        assert_eq!(calls[0].name, "java.ajaxAll");
        assert_eq!(
            calls[0].args,
            vec![
                json!([
                    { "url": "https://one.example.test" },
                    { "url": "https://two.example.test" }
                ]),
                json!({ "headers": { "Accept": "application/json" } })
            ]
        );
    }

    #[test]
    fn routes_java_ajax_through_host_callback_registry_like_legado() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajax", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!({
                "body": "<html>stub</html>",
                "url": call.args[0],
                "options": call.args.get(1).cloned().unwrap_or(JsonValue::Null)
            }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(
                r#"
                java.ajax("https://example.test/chapter", { method: "GET" })
                "#,
            )
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "body": "<html>stub</html>",
                "url": "https://example.test/chapter",
                "options": { "method": "GET" }
            })
        );

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "java.ajax");
        assert_eq!(
            calls[0].args,
            vec![
                json!("https://example.test/chapter"),
                json!({ "method": "GET" })
            ]
        );
    }

    #[test]
    fn routes_global_ajax_binding_through_host_callback_registry_like_old_core() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajax", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!("mock ajax body"))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(r#"ajax("https://example.test/api")"#)
            .unwrap();

        assert_eq!(result.value, json!("mock ajax body"));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "java.ajax");
        assert_eq!(calls[0].args, vec![json!("https://example.test/api")]);
    }

    #[test]
    fn routes_global_ajax_all_binding_through_host_callback_registry_like_old_core() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.ajaxAll", move |call| {
            captured.lock().unwrap().push(call.clone());
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
        assert_eq!(calls[0].name, "java.ajaxAll");
        assert_eq!(
            calls[0].args,
            vec![json!([
                "https://one.example.test",
                "https://two.example.test"
            ])]
        );
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
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.getSource", move |call| {
            captured.lock().unwrap().push(call.clone());
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
        assert_eq!(calls[0].name, "java.getSource");
        assert_eq!(calls[0].args, Vec::<JsonValue>::new());
        assert_eq!(calls[1].name, "java.getSource");
        assert_eq!(calls[1].args, Vec::<JsonValue>::new());
    }

    #[test]
    fn routes_java_get_string_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.getString", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!("Dune"))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox.evaluate(r#"java.getString("article h1")"#).unwrap();

        assert_eq!(result.value, json!("Dune"));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "java.getString");
        assert_eq!(calls[0].args, vec![json!("article h1")]);
    }

    #[test]
    fn routes_java_get_string_list_through_host_callback_registry() {
        let calls = Arc::new(Mutex::new(Vec::<HostCall>::new()));
        let captured = calls.clone();
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.getStringList", move |call| {
            captured.lock().unwrap().push(call.clone());
            Ok(json!(["Dune", "Foundation"]))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(r#"java.getStringList("article a.title")"#)
            .unwrap();

        assert_eq!(result.value, json!(["Dune", "Foundation"]));

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "java.getStringList");
        assert_eq!(calls[0].args, vec![json!("article a.title")]);
    }

    #[test]
    fn routes_java_post_and_returns_json() {
        let mut registry = HostCallbackRegistry::new();
        registry.register("java.post", |call| {
            Ok(json!({
                "method": call.name,
                "url": call.args[0],
                "body": call.args[1],
            }))
        });

        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(r#"java.post("https://example.test/post", { q: "reader" })"#)
            .unwrap();

        assert_eq!(
            result.value,
            json!({
                "method": "java.post",
                "url": "https://example.test/post",
                "body": { "q": "reader" }
            })
        );
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

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
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const MAX_JSON_DEPTH: usize = 128;
const MAX_PROMISE_JOBS: usize = 1024;

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

    fn install_host_api<'js>(&self, ctx: &Ctx<'js>) -> Result<(), QuickJsError> {
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
            "call",
            make_host_dispatch_callback(ctx.clone(), self.host_callbacks.clone())?,
        )?;
        ctx.globals().set("java", java)?;
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
            self.install_host_api(&ctx)
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
}

impl HostMethod {
    fn callback_name(self) -> &'static str {
        match self {
            Self::Get => "java.get",
            Self::Post => "java.post",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "get" | "java.get" => Some(Self::Get),
            "post" | "java.post" => Some(Self::Post),
            _ => None,
        }
    }

    fn validate_args<'js>(self, ctx: &Ctx<'js>, args: &[JsonValue]) -> Result<(), QuickJsError> {
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
}

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
        Arc,
    },
    time::{Duration, Instant},
};

const MAX_JSON_DEPTH: usize = 128;

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsRuntimeCapabilities {
    pub engine: &'static str,
    pub timeout: CapabilityStatus,
    pub cancellation: CapabilityStatus,
    pub memory_limit: CapabilityStatus,
    pub max_stack_size: CapabilityStatus,
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
            make_host_callback(ctx.clone(), "java.get", self.host_callbacks.clone())?,
        )?;
        java.set(
            "post",
            make_host_callback(ctx.clone(), "java.post", self.host_callbacks.clone())?,
        )?;
        ctx.globals().set("java", java)?;
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

        let context = Context::full(&runtime).map_err(map_quickjs_engine_error)?;
        context.with(|ctx| {
            self.install_host_api(&ctx)
                .map_err(map_quickjs_engine_error)?;
            let result = ctx.eval::<QuickJsValue<'_>, _>(script).catch(&ctx);
            let value = result.map_err(|error| map_caught_error(error, &interrupt))?;
            quickjs_value_to_json(&value, 0).map(|value| JsEvaluation { value })
        })
    }

    fn capabilities(&self) -> JsRuntimeCapabilities {
        JsRuntimeCapabilities {
            engine: "quickjs/rquickjs",
            timeout: configured_status(self.config.timeout.is_some()),
            cancellation: CapabilityStatus::SupportedNotConfigured,
            memory_limit: configured_status(self.config.memory_limit_bytes.is_some()),
            max_stack_size: configured_status(self.config.max_stack_size_bytes.is_some()),
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

fn make_host_callback<'js>(
    ctx: Ctx<'js>,
    name: &'static str,
    registry: HostCallbackRegistry,
) -> Result<rquickjs::Function<'js>, QuickJsError> {
    rquickjs::Function::new(
        ctx,
        move |ctx: Ctx<'js>,
              args: Rest<QuickJsValue<'js>>|
              -> Result<QuickJsValue<'js>, QuickJsError> {
            let mut json_args = Vec::with_capacity(args.0.len());
            for arg in args.0.iter() {
                match quickjs_value_to_json(arg, 0) {
                    Ok(value) => json_args.push(value),
                    Err(error) => {
                        return Err(Exception::throw_type(
                            &ctx,
                            format!("host callback argument is not JSON-compatible: {error}")
                                .as_str(),
                        ));
                    }
                }
            }

            let result = registry
                .call(HostCall {
                    name: name.to_string(),
                    args: json_args,
                })
                .map_err(|error| {
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
            let kind = if name.as_deref() == Some("SyntaxError") {
                JsErrorKind::Syntax
            } else {
                JsErrorKind::Exception
            };
            let message = exception
                .message()
                .or(name)
                .unwrap_or_else(|| "JavaScript exception".to_string());
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
    fn reports_memory_limit_capability_when_configured() {
        let sandbox = QuickJsSandbox::new(JsRuntimeConfig {
            memory_limit_bytes: Some(1024 * 1024),
            ..JsRuntimeConfig::default()
        });

        assert_eq!(
            sandbox.capabilities().memory_limit,
            CapabilityStatus::Enforced
        );
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
}

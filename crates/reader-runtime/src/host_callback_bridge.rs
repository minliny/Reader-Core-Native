//! Bridge wiring `java.get`/`java.post`/`java.ajax`/`java.connect`/`java.ajaxAll`
//! JS host callbacks to the Core → Host `http.execute` request/response flow.
//!
//! ## Why this exists
//!
//! Legado's `AnalyzeUrl.analyzeJs()` lets `@js:`/`<js>` URL rules call
//! `java.get(url)` / `java.ajax(url)` to fetch additional HTTP resources
//! while building the final search/detail/toc URL. Reader's `reader-js`
//! crate already parses these calls into [`HostDescriptor`] variants and
//! invokes a registered [`HostCallbackRegistry`] callback, but the default
//! `QuickJsSandbox` registers no callbacks — so `java.get` returned
//! "unregistered host callback" and 28% of P0 corpus sources failed L2-search.
//!
//! ## The synchronous-JS / async-HTTP challenge
//!
//! QuickJS executes synchronously: `java.get(url)` must return the body
//! before JS can continue. But the HTTP response arrives asynchronously via
//! `host.complete` on the runtime's mpsc worker queue. If the worker thread
//! (executing JS) blocks waiting for `host.complete`, it cannot process the
//! next `WorkItem` → deadlock.
//!
//! ## Solution (B: synchronous blocking + send-time interception)
//!
//! 1. The callback (running on the worker thread, inside JS) emits a
//!    `host.request` event via [`EventSink`] and blocks on a [`PendingCallback`]
//!    Condvar.
//! 2. `Runtime::send` intercepts every `host.complete` / `host.error`
//!    command: if its `operationId` is registered in [`HostCallbackBridge::pending`],
//!    the result is delivered directly to the blocked callback via
//!    [`PendingCallback::complete`] and the command is NOT enqueued on the
//!    worker mpsc queue. This breaks the deadlock because the completion
//!    never needs the worker thread to be free.
//! 3. The callback wakes, extracts the HTTP body, and returns it to JS.
//!
//! Non-JS host operations (the normal `book_search` continuation flow)
//! have operation IDs NOT in `pending`, so they fall through to the
//! normal enqueue path — zero behavioral change for existing flows.
//!
//! ## Legado reference
//!
//! - `legado/.../AnalyzeUrl.kt:153` `analyzeJs()` — JS may call java.get/post/ajax
//! - `legado/.../JsExtensions.kt` `ajax`/`get`/`post` — suspend functions that
//!   perform HTTP and return body/response. Legado uses coroutine suspension;
//!   Reader uses synchronous blocking + send-time interception (this module).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use reader_contract::{methods, Event, HostCapability};
use reader_js::{
    HostCallbackRegistry, HostDescriptor, HostError, JsRuntimeConfig, JsSandbox, QuickJsSandbox,
};
use serde_json::Value as JsonValue;

use crate::sink::EventSink;

/// Default time a JS host callback waits for `host.complete` before failing.
/// Matches Legado's default HTTP timeout (30s).
const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(30);

/// Operation IDs for JS host callbacks start here to avoid accidental overlap
/// with the runtime's `host_operations` counter (which starts at 1). The
/// interception in `Runtime::send` keys off the `pending` map, not the ID
/// range, so collisions are harmless — but distinct ranges aid debugging.
const JS_CALLBACK_OP_ID_BASE: u64 = 1_000_000;

/// A pending JS host callback: the worker thread blocks on `wait()` while
/// `Runtime::send` (on the FFI host thread) calls `complete()`.
struct PendingCallback {
    result: Mutex<Option<Result<JsonValue, String>>>,
    notify: Condvar,
}

impl PendingCallback {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(None),
            notify: Condvar::new(),
        })
    }

    /// Deliver the host result (or error message) to the blocked callback.
    fn complete(&self, result: Result<JsonValue, String>) {
        if let Ok(mut guard) = self.result.lock() {
            if guard.is_none() {
                *guard = Some(result);
            }
            // If already set (e.g. timeout raced), drop the late result.
        }
        self.notify.notify_one();
    }

    /// Block the calling thread until `complete` is called or `timeout` elapses.
    fn wait(&self, timeout: Duration) -> Result<JsonValue, String> {
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .result
            .lock()
            .map_err(|_| "callback mutex poisoned".to_string())?;
        while guard.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "host callback timed out after {timeout:?} (host did not respond)"
                ));
            }
            let remaining = deadline.saturating_duration_since(now);
            let (g, wait_result) = self
                .notify
                .wait_timeout(guard, remaining)
                .map_err(|_| "callback mutex poisoned".to_string())?;
            guard = g;
            if wait_result.timed_out() && guard.is_none() {
                return Err(format!(
                    "host callback timed out after {timeout:?} (host did not respond)"
                ));
            }
        }
        guard
            .take()
            .unwrap_or_else(|| Err("callback result missing".to_string()))
    }
}

/// Selects how the host HTTP response is shaped before returning to JS.
///
/// Legado contract (mirrored by `reader-js` bindings):
/// - `java.get`/`java.post`/`java.connect` → response object (`{body,status,...}`)
/// - `java.ajax` → body string only
/// - `java.ajaxAll` → array of body strings
#[derive(Clone, Copy, Debug)]
enum HttpResponseShape {
    /// Return the full host result object (status/headers/body/...).
    Object,
    /// Return only the `body` string (Legado `java.ajax` contract).
    BodyString,
}

/// Bridge between JS host callbacks and the Core → Host `http.execute` flow.
///
/// Held by [`crate::remote::RemoteState`] (shared via `Arc`) and cloned into
/// [`crate::runtime::Runtime`] for send-time interception. All fields are
/// `Arc`-wrapped so cloning is cheap and shares state.
#[derive(Clone)]
pub struct HostCallbackBridge {
    /// operationId → pending callback. `Runtime::send` looks up here to decide
    /// whether a `host.complete`/`host.error` belongs to a JS callback.
    pending: Arc<Mutex<HashMap<u64, Arc<PendingCallback>>>>,
    /// Monotonic counter for JS-callback operation IDs.
    next_op_id: Arc<AtomicU64>,
    /// Event sink used to emit `host.request` events from inside the callback.
    sink: Arc<dyn EventSink>,
    /// The requestId of the command currently being dispatched on the worker
    /// thread. Set by `dispatch_remote` / `dispatch_host_complete` before any
    /// JS evaluation. Read by the callback to stamp the originating request
    /// on the emitted `host.request`.
    current_request_id: Arc<AtomicU64>,
    /// How long a callback waits for `host.complete` before failing.
    timeout: Duration,
}

impl HostCallbackBridge {
    /// Construct a new bridge that emits host requests via `sink`.
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_op_id: Arc::new(AtomicU64::new(JS_CALLBACK_OP_ID_BASE)),
            sink,
            current_request_id: Arc::new(AtomicU64::new(0)),
            timeout: DEFAULT_CALLBACK_TIMEOUT,
        }
    }

    /// Set the requestId that JS-callback `host.request` events should carry.
    /// Called by the runtime before dispatching a command that may evaluate
    /// `@js:`/`<js>` URL rules.
    pub fn set_current_request_id(&self, request_id: u64) {
        self.current_request_id.store(request_id, Ordering::Release);
    }

    /// Try to route a `host.complete` / `host.error` command to a pending JS
    /// callback. Returns `true` if `operation_id` was found and the callback
    /// was signaled (in which case `Runtime::send` must NOT enqueue the command).
    ///
    /// Called from `Runtime::send` on the FFI host thread.
    pub fn try_complete(&self, method: &str, params: &JsonValue) -> bool {
        let Some(op_id) = params.get("operationId").and_then(JsonValue::as_u64) else {
            return false;
        };
        let pending = {
            let mut map = match self.pending.lock() {
                Ok(map) => map,
                Err(_) => return false,
            };
            map.remove(&op_id)
        };
        let Some(callback) = pending else {
            return false;
        };
        let result = if method == methods::HOST_COMPLETE {
            let host_result = params.get("result").cloned().unwrap_or(JsonValue::Null);
            Ok(host_result)
        } else {
            // host.error — surface a human-readable message if present.
            let error_msg = params
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(JsonValue::as_str)
                .or_else(|| params.get("error").and_then(JsonValue::as_str))
                .unwrap_or("host.error for JS callback")
                .to_string();
            Err(error_msg)
        };
        callback.complete(result);
        true
    }

    /// Build a `HostCallbackRegistry` with `java.get`/`java.post`/`java.connect`/
    /// `java.ajax`/`java.ajaxAll` registered to route through this bridge, plus
    /// the non-HTTP `java.*` helpers (`java.put`/`java.getCookie`/
    /// `java.getSource`/`java.webView`) registered with Core-side semantics so
    /// URL-building JS no longer fails with "unregistered host callback".
    ///
    /// The returned registry is used to construct a
    /// `QuickJsSandbox::with_host_callbacks` that `RemoteContentPipeline`
    /// consumes via `with_js_sandbox`.
    pub fn build_registry(&self) -> HostCallbackRegistry {
        let mut registry = HostCallbackRegistry::new();

        let bridge = self.clone();
        registry.register("java.get", move |descriptor| {
            bridge.execute_single_http(descriptor, HttpResponseShape::Object)
        });

        let bridge = self.clone();
        registry.register("java.post", move |descriptor| {
            bridge.execute_single_http(descriptor, HttpResponseShape::Object)
        });

        let bridge = self.clone();
        registry.register("java.connect", move |descriptor| {
            bridge.execute_single_http(descriptor, HttpResponseShape::Object)
        });

        let bridge = self.clone();
        registry.register("java.ajax", move |descriptor| {
            bridge.execute_single_http(descriptor, HttpResponseShape::BodyString)
        });

        let bridge = self.clone();
        registry.register("java.ajaxAll", move |descriptor| {
            bridge.execute_ajax_all(descriptor)
        });

        // `java.put(key, value)` — Legado stores key-value in the source/book/
        // chapter variable map and returns `value`. Core has no cross-evaluation
        // variable store (each URL-building JS runs in a single evaluation), so
        // we accept the call, drop the key, and return `value` — this lets JS
        // that uses `var x = java.put("k", v)` or `java.put("k", v); ...v...`
        // proceed instead of failing with "unregistered host callback: java.put".
        // Sources that rely on cross-evaluation retrieval (`java.get("k")` in a
        // later evaluation) are out of scope for URL-building JS.
        registry.register("java.put", move |descriptor| {
            let HostDescriptor::Put { key, value } = descriptor else {
                return Err(HostError::new(
                    "java.put expected HostDescriptor::Put".to_string(),
                ));
            };
            let _ = key; // key accepted; Core has no variable store to persist into
            Ok(JsonValue::String(value))
        });

        // `java.getCookie(tag, key?)` — Legado reads from the host cookie jar.
        // Core does not manage cookies (host-managed per red line 4). Return an
        // empty string so cookie-dependent JS degrades gracefully rather than
        // failing with "unregistered host callback".
        registry.register("java.getCookie", move |descriptor| {
            let HostDescriptor::GetCookie { tag, key } = descriptor else {
                return Err(HostError::new(
                    "java.getCookie expected HostDescriptor::GetCookie".to_string(),
                ));
            };
            let _ = (tag, key);
            Ok(JsonValue::String(String::new()))
        });

        // `java.getSource()` — Legado returns the currently-bound book-source
        // object. Core exposes the source as a JS global (`source`) via
        // `evaluate_url_js`'s prelude; `java.getSource()` is a separate API
        // surface that Legado JS may call. Return an empty object so
        // `java.getSource().bookSourceUrl` resolves to undefined (no crash)
        // rather than throwing "unregistered host callback: java.getSource".
        registry.register("java.getSource", move |descriptor| {
            let HostDescriptor::GetSource = descriptor else {
                return Err(HostError::new(
                    "java.getSource expected HostDescriptor::GetSource".to_string(),
                ));
            };
            Ok(JsonValue::Object(serde_json::Map::new()))
        });

        // `java.webView(html?, url?, js?)` — Legado loads a WebView, runs JS,
        // and returns the body. Core never touches WebView (red line 4: Core
        // does not open sockets, touch WebView, or store plaintext credentials).
        // Register the callback so the error is a clean "unsupported" signal
        // rather than "unregistered host callback" — the source still fails,
        // but with a message that explains why (red line 4) instead of looking
        // like a wiring gap.
        registry.register("java.webView", move |descriptor| {
            let HostDescriptor::WebView { .. } = descriptor else {
                return Err(HostError::new(
                    "java.webView expected HostDescriptor::WebView".to_string(),
                ));
            };
            Err(HostError::new(
                "java.webView is unsupported in Core (red line 4: Core does not touch WebView) \
                 — host platforms handle WebView; sources requiring WebView cannot run in Core"
                    .to_string(),
            ))
        });

        registry
    }

    /// Build a `QuickJsSandbox` wired with this bridge's host callbacks.
    pub fn build_sandbox(&self) -> QuickJsSandbox {
        let registry = self.build_registry();
        QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry)
    }

    /// Execute a single HTTP request through the host and return the result.
    /// Blocks the calling (worker) thread until `host.complete` arrives.
    fn execute_single_http(
        &self,
        descriptor: HostDescriptor,
        shape: HttpResponseShape,
    ) -> Result<JsonValue, HostError> {
        let (url, method, headers, body) = match descriptor {
            HostDescriptor::HttpGet { url, headers } => {
                (url, "GET", headers, JsonValue::String(String::new()))
            }
            HostDescriptor::HttpPost { url, body, headers } => {
                (url, "POST", headers, JsonValue::String(body))
            }
            HostDescriptor::HttpConnect { url, header } => {
                let headers = header.and_then(|h| serde_json::from_str(&h).ok());
                (url, "GET", headers, JsonValue::String(String::new()))
            }
            HostDescriptor::Ajax { url } => (url, "GET", None, JsonValue::String(String::new())),
            other => {
                return Err(HostError::new(format!(
                    "host descriptor not supported by HTTP bridge: {other:?}"
                )));
            }
        };

        let params = serde_json::json!({
            "url": url,
            "method": method,
            "headers": headers.unwrap_or_else(|| serde_json::json!({})),
            "body": body,
        });

        let host_result = self.dispatch_and_wait(params)?;
        Ok(self.shape_response(host_result, shape))
    }

    /// Execute `java.ajaxAll`: fetch each URL sequentially and return an
    /// array of body strings. Sequential (not concurrent) to match the
    /// single-worker-thread model — Legado's concurrency is a coroutine
    /// optimization, not a behavioral contract.
    fn execute_ajax_all(&self, descriptor: HostDescriptor) -> Result<JsonValue, HostError> {
        let urls = match descriptor {
            HostDescriptor::AjaxAll { urls } => urls,
            other => {
                return Err(HostError::new(format!(
                    "ajaxAll expected AjaxAll descriptor, got {other:?}"
                )));
            }
        };

        let mut bodies = Vec::with_capacity(urls.len());
        for url in urls {
            let params = serde_json::json!({
                "url": url,
                "method": "GET",
                "headers": {},
                "body": "",
            });
            let host_result = self.dispatch_and_wait(params)?;
            let body = host_result
                .get("body")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string();
            bodies.push(JsonValue::String(body));
        }
        Ok(JsonValue::Array(bodies))
    }

    /// Emit a `host.request` for `params` and block until `host.complete` /
    /// `host.error` is routed back via [`Self::try_complete`].
    fn dispatch_and_wait(&self, params: JsonValue) -> Result<JsonValue, HostError> {
        let request_id = self.current_request_id.load(Ordering::Acquire);
        if request_id == 0 {
            return Err(HostError::new(
                "JS host callback invoked with no active request context \
                 (current_request_id not set)",
            ));
        }

        let op_id = self.next_op_id.fetch_add(1, Ordering::AcqRel);
        let callback = PendingCallback::new();
        {
            let mut map = self
                .pending
                .lock()
                .map_err(|_| HostError::new("callback registry mutex poisoned"))?;
            map.insert(op_id, callback.clone());
        }

        self.sink.emit(&Event::host_request(
            request_id,
            op_id,
            HostCapability::HttpExecute,
            params,
        ));

        let result = callback.wait(self.timeout);

        if result.is_err() {
            // Clean up the pending entry if we timed out (try_complete may
            // have already removed it, but a timed-out wait leaves it behind).
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&op_id);
            }
        }

        result.map_err(HostError::new)
    }

    /// Shape the host HTTP result for the JS return value.
    fn shape_response(&self, host_result: JsonValue, shape: HttpResponseShape) -> JsonValue {
        match shape {
            HttpResponseShape::Object => host_result,
            HttpResponseShape::BodyString => {
                let body = host_result
                    .get("body")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .to_string();
                JsonValue::String(body)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// A sink that captures events and lets the test thread inspect them.
    struct CapturingSink {
        events: StdMutex<Vec<Event>>,
    }

    impl CapturingSink {
        fn new() -> Self {
            Self {
                events: StdMutex::new(Vec::new()),
            }
        }

        fn drain(&self) -> Vec<Event> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl EventSink for CapturingSink {
        fn emit(&self, event: &Event) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn try_complete_returns_false_for_unknown_op_id() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        let params = serde_json::json!({ "operationId": 9999, "result": {} });
        assert!(!bridge.try_complete(methods::HOST_COMPLETE, &params));
    }

    #[test]
    fn try_complete_returns_false_when_operation_id_missing() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        let params = serde_json::json!({ "result": {} });
        assert!(!bridge.try_complete(methods::HOST_COMPLETE, &params));
    }

    #[test]
    fn execute_single_http_emits_host_request_and_receives_body() {
        // Simulate a host that, on seeing host.request, immediately calls
        // try_complete with a canned response. The worker thread (this test)
        // blocks in the callback until the simulated host thread completes it.
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink.clone());
        bridge.set_current_request_id(42);

        // Spawn a thread that waits for the host.request event, then completes.
        let bridge_for_host = bridge.clone();
        let host_thread = std::thread::spawn(move || {
            // Spin until the host.request event appears.
            let mut op_id = None;
            for _ in 0..200 {
                let events = sink.drain();
                if let Some(Event::HostRequest { operation_id, .. }) = events.into_iter().next() {
                    op_id = Some(operation_id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let op_id = op_id.expect("host.request not emitted");
            let params = serde_json::json!({
                "operationId": op_id,
                "result": {
                    "status": 200,
                    "body": "{\"hello\":\"world\"}",
                    "headers": { "content-type": "application/json" }
                }
            });
            assert!(
                bridge_for_host.try_complete(methods::HOST_COMPLETE, &params),
                "try_complete should route to pending callback"
            );
        });

        let registry = bridge.build_registry();
        let result = registry.call(
            "java.get",
            HostDescriptor::HttpGet {
                url: "https://example.test/data".to_string(),
                headers: None,
            },
        );
        host_thread.join().unwrap();
        let value = result.expect("java.get should succeed");
        assert_eq!(value["body"], "{\"hello\":\"world\"}");
        assert_eq!(value["status"], 200);
    }

    #[test]
    fn java_ajax_returns_body_string_only() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink.clone());
        bridge.set_current_request_id(7);

        let bridge_for_host = bridge.clone();
        let host_thread = std::thread::spawn(move || {
            let mut op_id = None;
            for _ in 0..200 {
                let events = sink.drain();
                if let Some(Event::HostRequest { operation_id, .. }) = events.into_iter().next() {
                    op_id = Some(operation_id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let op_id = op_id.expect("host.request not emitted");
            let params = serde_json::json!({
                "operationId": op_id,
                "result": { "status": 200, "body": "plain text body" }
            });
            assert!(bridge_for_host.try_complete(methods::HOST_COMPLETE, &params));
        });

        let registry = bridge.build_registry();
        let result = registry.call(
            "java.ajax",
            HostDescriptor::Ajax {
                url: "https://example.test/ajax".to_string(),
            },
        );
        host_thread.join().unwrap();
        let value = result.expect("java.ajax should succeed");
        assert_eq!(value, serde_json::json!("plain text body"));
    }

    #[test]
    fn host_error_routes_error_message_to_callback() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink.clone());
        bridge.set_current_request_id(99);

        let bridge_for_host = bridge.clone();
        let host_thread = std::thread::spawn(move || {
            let mut op_id = None;
            for _ in 0..200 {
                let events = sink.drain();
                if let Some(Event::HostRequest { operation_id, .. }) = events.into_iter().next() {
                    op_id = Some(operation_id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let op_id = op_id.expect("host.request not emitted");
            let params = serde_json::json!({
                "operationId": op_id,
                "error": { "message": "DNS resolution failed" }
            });
            assert!(bridge_for_host.try_complete(methods::HOST_ERROR, &params));
        });

        let registry = bridge.build_registry();
        let result = registry.call(
            "java.get",
            HostDescriptor::HttpGet {
                url: "https://unresolved.invalid/".to_string(),
                headers: None,
            },
        );
        host_thread.join().unwrap();
        let err = result.expect_err("java.get should propagate host.error");
        assert!(
            err.message.contains("DNS resolution failed"),
            "error message should include host detail: {}",
            err.message
        );
    }

    #[test]
    fn callback_times_out_when_host_never_responds() {
        let sink = Arc::new(CapturingSink::new());
        let mut bridge = HostCallbackBridge::new(sink);
        bridge.timeout = Duration::from_millis(50);
        bridge.set_current_request_id(1);

        let registry = bridge.build_registry();
        let result = registry.call(
            "java.get",
            HostDescriptor::HttpGet {
                url: "https://example.test/never".to_string(),
                headers: None,
            },
        );
        let err = result.expect_err("should time out");
        assert!(err.message.contains("timed out"), "error: {}", err.message);
    }

    #[test]
    fn missing_request_context_fails_fast() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        // current_request_id never set → should fail immediately.
        let registry = bridge.build_registry();
        let result = registry.call(
            "java.get",
            HostDescriptor::HttpGet {
                url: "https://example.test/".to_string(),
                headers: None,
            },
        );
        let err = result.expect_err("should fail without request context");
        assert!(
            err.message.contains("request context"),
            "error: {}",
            err.message
        );
    }

    #[test]
    fn build_sandbox_executes_js_with_java_get_callback() {
        // End-to-end: build a sandbox with the bridge's callbacks, run JS that
        // calls java.get, and verify the body is returned to JS.
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink.clone());
        bridge.set_current_request_id(100);

        let bridge_for_host = bridge.clone();
        let host_thread = std::thread::spawn(move || {
            let mut op_id = None;
            for _ in 0..200 {
                let events = sink.drain();
                if let Some(Event::HostRequest { operation_id, .. }) = events.into_iter().next() {
                    op_id = Some(operation_id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let op_id = op_id.expect("host.request not emitted");
            let params = serde_json::json!({
                "operationId": op_id,
                "result": { "status": 200, "body": "{\"ok\":true}" }
            });
            assert!(bridge_for_host.try_complete(methods::HOST_COMPLETE, &params));
        });

        let sandbox = bridge.build_sandbox();
        let script = r#"
            var resp = java.get("https://example.test/api");
            JSON.stringify({ body: resp.body, status: resp.status });
        "#;
        let result = sandbox.evaluate(script);
        host_thread.join().unwrap();
        let value = result.expect("JS evaluation should succeed");
        let body_str = value.value.as_str().expect("JS result should be a string");
        let parsed: serde_json::Value = serde_json::from_str(body_str).unwrap();
        assert_eq!(parsed["body"], "{\"ok\":true}");
        assert_eq!(parsed["status"], 200);
    }

    /// `java.put(key, value)` returns the value (Legado contract) instead of
    /// failing with "unregistered host callback". Batch v4 had 10 sources
    /// failing on this; the registration lets URL-building JS proceed.
    #[test]
    fn java_put_returns_value_without_network() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink.clone());
        bridge.set_current_request_id(1);
        let registry = bridge.build_registry();
        let result = registry.call(
            "java.put",
            HostDescriptor::Put {
                key: "sign".to_string(),
                value: "abc123".to_string(),
            },
        );
        let value = result.expect("java.put should succeed");
        assert_eq!(value, serde_json::json!("abc123"));
        // No host.request emitted — java.put is pure local state.
        assert!(sink.drain().is_empty(), "java.put must not emit host.request");
    }

    /// `java.getCookie(tag, key)` returns an empty string (Core does not manage
    /// cookies — host-managed per red line 4). Registered so cookie-dependent
    /// JS degrades gracefully instead of failing "unregistered".
    #[test]
    fn java_get_cookie_returns_empty_string() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        bridge.set_current_request_id(1);
        let registry = bridge.build_registry();
        let result = registry.call(
            "java.getCookie",
            HostDescriptor::GetCookie {
                tag: "https://example.test".to_string(),
                key: Some("session".to_string()),
            },
        );
        let value = result.expect("java.getCookie should succeed");
        assert_eq!(value, serde_json::json!(""));
    }

    /// `java.getSource()` returns an empty object (the source is exposed as a
    /// JS global `source` via evaluate_url_js; java.getSource is a separate
    /// Legado API). Registered so JS that calls it doesn't fail "unregistered".
    #[test]
    fn java_get_source_returns_empty_object() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        bridge.set_current_request_id(1);
        let registry = bridge.build_registry();
        let result = registry.call("java.getSource", HostDescriptor::GetSource);
        let value = result.expect("java.getSource should succeed");
        assert_eq!(value, serde_json::json!({}));
    }

    /// `java.webView(...)` returns an error (red line 4: Core does not touch
    /// WebView). The callback IS registered, so the error is a clean
    /// "unsupported" signal explaining red line 4 — not "unregistered host
    /// callback". Batch v4 had 1 source failing on the unregistered form.
    #[test]
    fn java_web_view_returns_unsupported_error() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        bridge.set_current_request_id(1);
        let registry = bridge.build_registry();
        let result = registry.call(
            "java.webView",
            HostDescriptor::WebView {
                html: None,
                url: Some("https://example.test".to_string()),
                js: None,
            },
        );
        let err = result.expect_err("java.webView must error (red line 4)");
        assert!(
            err.message.contains("unsupported"),
            "error should signal unsupported: {}",
            err.message
        );
        assert!(
            err.message.contains("WebView"),
            "error should mention WebView: {}",
            err.message
        );
    }

    /// End-to-end: a JS sandbox built with the bridge's registry can call
    /// `java.put` / `java.getSource` / `java.getCookie` without throwing
    /// "unregistered host callback". `java.webView` throws an exception
    /// carrying the red-line-4 message.
    #[test]
    fn build_sandbox_executes_js_with_non_http_host_callbacks() {
        let sink = Arc::new(CapturingSink::new());
        let bridge = HostCallbackBridge::new(sink);
        bridge.set_current_request_id(1);
        let sandbox = bridge.build_sandbox();

        // java.put / java.getSource / java.getCookie succeed.
        let result = sandbox.evaluate(
            r#"
            var putResult = java.put("k", "v");
            var src = java.getSource();
            var cookie = java.getCookie("https://example.test", "session");
            JSON.stringify({ put: putResult, src: src, cookie: cookie });
        "#,
        );
        let value = result.expect("non-HTTP callbacks should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(value.value.as_str().expect("result is string"))
                .expect("result is valid JSON");
        assert_eq!(parsed["put"], "v", "java.put returns value");
        assert_eq!(parsed["src"], serde_json::json!({}), "java.getSource returns empty object");
        assert_eq!(parsed["cookie"], "", "java.getCookie returns empty");

        // java.webView throws an exception with the red-line-4 message.
        let result = sandbox.evaluate(r#"java.webView(null, "https://example.test", null)"#);
        let err = result.expect_err("java.webView must throw");
        assert!(
            err.message.contains("unsupported"),
            "webView error should signal unsupported: {}",
            err.message
        );
    }
}
